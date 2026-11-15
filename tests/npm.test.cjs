'use strict';

const assert = require('node:assert/strict');
const { spawn, spawnSync } = require('node:child_process');
const { once } = require('node:events');
const { chmodSync, copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { test } = require('node:test');

function fixture(t, source) {
  const root = mkdtempSync(join(tmpdir(), 'fastgpu-npm-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, 'npm', 'bin'), { recursive: true });
  const launcher = join(root, 'npm', 'fastgpu.cjs');
  copyFileSync(resolve(__dirname, '../npm/fastgpu.cjs'), launcher);
  if (source !== undefined) {
    const binary = join(root, 'npm', 'bin', 'fastgpu');
    writeFileSync(binary, `#!${process.execPath}\n${source}\n`, { mode: 0o755 });
  }
  return { root, launcher };
}

test('launcher preserves arguments, cwd, environment, and process identity', t => {
  const { root, launcher } = fixture(t, `console.log(JSON.stringify({
    args: process.argv.slice(2), cwd: process.cwd(),
    marker: process.env.FASTGPU_TEST_MARKER, pid: process.pid
  }));`);
  const args = ['shell', '--cwd', 'home:project with spaces', "quote'back", '$(printf unexpected)', ''];
  const result = spawnSync(process.execPath, [launcher, ...args], {
    cwd: root, encoding: 'utf8', env: { ...process.env, FASTGPU_TEST_MARKER: 'fixture' },
  });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), { args, cwd: root, marker: 'fixture', pid: result.pid });
});

test('launcher preserves stdin, stdout, stderr, and nonzero exit status', t => {
  const { launcher } = fixture(t, `
    process.stdin.pipe(process.stdout);
    process.stdin.on('end', () => { process.stderr.write('fixture error\\n'); process.exitCode = 37; });
  `);
  const result = spawnSync(process.execPath, [launcher], { input: 'stream input\n', encoding: 'utf8' });
  assert.equal(result.stdout, 'stream input\n');
  assert.equal(result.stderr, 'fixture error\n');
  assert.equal(result.status, 37);
});

for (const [signal, expected] of [['SIGINT', 42], ['SIGTERM', 43]]) {
  test(`launcher preserves ${signal} delivery to the binary`, { timeout: 5000 }, async t => {
    const { launcher } = fixture(t, `
      process.on('${signal}', () => process.exit(${expected}));
      console.log('ready');
      setInterval(() => {}, 1000);
    `);
    const child = spawn(process.execPath, [launcher], { stdio: ['ignore', 'pipe', 'pipe'] });
    t.after(() => child.kill('SIGKILL'));
    const [ready] = await once(child.stdout, 'data');
    assert.equal(ready.toString(), 'ready\n');
    const exited = once(child, 'exit');
    child.kill(signal);
    const [code, exitSignal] = await exited;
    assert.equal(code, expected);
    assert.equal(exitSignal, null);
  });
}

test('launcher reports a missing binary without a JavaScript stack trace', t => {
  const { launcher } = fixture(t);
  const result = spawnSync(process.execPath, [launcher], { encoding: 'utf8' });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /^fastgpu: could not start the packaged binary:/);
  assert.doesNotMatch(result.stderr, /at Object\./);
});

test('launcher rejects unsupported platforms and Node runtimes', t => {
  const { launcher } = fixture(t, 'process.exit(0);');
  for (const [setup, message] of [
    ["Object.defineProperty(process, 'arch', { value: 'arm64' });", /Linux x64 only/],
    ['process.execve = undefined;', /Node\.js 22\.15/],
  ]) {
    const result = spawnSync(process.execPath, ['-e', `${setup}require(${JSON.stringify(launcher)});`], { encoding: 'utf8' });
    assert.equal(result.status, 1);
    assert.match(result.stderr, message);
  }
});

test('launcher rejects a binary without execute permission', t => {
  const { root, launcher } = fixture(t, 'process.exit(0);');
  chmodSync(join(root, 'npm', 'bin', 'fastgpu'), 0o644);
  const result = spawnSync(process.execPath, [launcher], { encoding: 'utf8' });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /EACCES/);
});
