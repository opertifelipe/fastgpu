'use strict';

const assert = require('node:assert/strict');
const { execFileSync, spawnSync } = require('node:child_process');
const { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { dirname, join, resolve } = require('node:path');
const { test } = require('node:test');
const metadata = require('../package.json');
const root = resolve(__dirname, '..');

test('tarball installs offline without Cargo or installation scripts', { timeout: 300000 }, t => {
  const temporary = mkdtempSync(join(tmpdir(), 'fastgpu-package-'));
  t.after(() => rmSync(temporary, { recursive: true, force: true }));
  execFileSync(process.execPath, ['scripts/build-npm.cjs'], { cwd: root });
  const [archive] = JSON.parse(execFileSync('npm', [
    'pack', '--ignore-scripts', '--json', '--pack-destination', temporary,
  ], { cwd: root, encoding: 'utf8' }));

  const expected = [
    'package.json', 'README.md', 'LICENSE', 'DESIGN.md',
    'npm/fastgpu.cjs', 'npm/bin/fastgpu',
    ...readdirSync(join(root, 'docs')).filter(name => name.endsWith('.md')).map(name => `docs/${name}`),
  ];
  assert.deepEqual(archive.files.map(file => file.path).sort(), expected.sort());
  for (const name of ['npm/fastgpu.cjs', 'npm/bin/fastgpu']) {
    assert.ok(archive.files.find(file => file.path === name).mode & 0o111);
  }

  const prefix = join(temporary, 'installed');
  const env = {
    ...process.env,
    // Node/npm and base system tools are available; Cargo is not required.
    PATH: `${dirname(process.execPath)}:/usr/bin:/bin`,
    GPU_CONFIG_DIR: join(temporary, 'profile'),
    npm_config_cache: join(temporary, 'cache'),
  };
  delete env.RUNPOD_API_KEY;
  execFileSync('npm', [
    'install', '--global', '--prefix', prefix, '--offline', '--no-audit', '--no-fund',
    join(temporary, archive.filename),
  ], { env });
  const binary = join(prefix, 'bin', 'fastgpu');
  assert.equal(execFileSync(binary, ['--version'], { env, encoding: 'utf8' }), `fastgpu version ${metadata.version}\n`);
  assert.match(execFileSync(binary, ['--help'], { env, encoding: 'utf8' }), /Your remote GPU environment/);
  const config = JSON.parse(execFileSync(binary, ['config'], { env, encoding: 'utf8' }));
  assert.equal(config.local_port, 8080);
  const installed = JSON.parse(readFileSync(join(prefix, 'lib/node_modules/fastgpu/package.json'), 'utf8'));
  assert.equal(installed.private, true);
  for (const hook of ['preinstall', 'install', 'postinstall', 'prepare']) {
    assert.equal(installed.scripts[hook], undefined);
  }
});

test('packaging rejects invalid or mismatched binaries', t => {
  const temporary = mkdtempSync(join(tmpdir(), 'fastgpu-invalid-package-'));
  t.after(() => rmSync(temporary, { recursive: true, force: true }));
  mkdirSync(join(temporary, 'bin'));
  const invalid = join(temporary, 'bin', 'fastgpu');
  writeFileSync(invalid, '#!/bin/sh\nexit 0\n', { mode: 0o755 });
  for (const [binary, message] of [[invalid, /Linux x64 ELF/], ['/bin/true', /binary version mismatch/]]) {
    const result = spawnSync(process.execPath, ['scripts/build-npm.cjs'], {
      cwd: root, encoding: 'utf8', env: { ...process.env, FASTGPU_NPM_BINARY: binary },
    });
    assert.equal(result.status, 1);
    assert.match(result.stderr, message);
  }
});
