'use strict';

const { execFileSync } = require('node:child_process');
const { chmodSync, copyFileSync, mkdirSync, openSync, readSync, closeSync } = require('node:fs');
const { dirname, join, resolve } = require('node:path');
const metadata = require('../package.json');
const root = resolve(__dirname, '..');

try {
  if (process.platform !== 'linux' || process.arch !== 'x64') {
    throw new Error('build this npm package on Linux x64');
  }
  const cargo = JSON.parse(execFileSync('cargo', [
    'metadata', '--no-deps', '--format-version', '1', '--locked',
  ], { cwd: root, encoding: 'utf8' }));
  const version = cargo.packages.find(pkg => pkg.name === 'fastgpu')?.version;
  if (version !== metadata.version) {
    throw new Error(`package.json version ${metadata.version} differs from Cargo.toml version ${version}`);
  }
  if (!process.env.FASTGPU_NPM_BINARY) {
    execFileSync('make', ['build'], { cwd: root, stdio: 'inherit' });
  }
  const source = process.env.FASTGPU_NPM_BINARY
    ? resolve(process.env.FASTGPU_NPM_BINARY)
    : join(root, 'bin', 'fastgpu');
  const header = Buffer.alloc(20);
  const file = openSync(source, 'r');
  try {
    readSync(file, header, 0, header.length, 0);
  } finally {
    closeSync(file);
  }
  if (header.subarray(0, 4).toString('hex') !== '7f454c46'
      || header[4] !== 2 || header[5] !== 1 || header.readUInt16LE(18) !== 62) {
    throw new Error('the package requires a Linux x64 ELF binary');
  }
  const output = execFileSync(source, ['--version'], { encoding: 'utf8' }).trim();
  if (output !== `fastgpu version ${metadata.version}`) {
    throw new Error(`binary version mismatch: expected fastgpu version ${metadata.version}`);
  }
  const destination = join(root, 'npm', 'bin', 'fastgpu');
  mkdirSync(dirname(destination), { recursive: true });
  if (source !== destination) copyFileSync(source, destination);
  chmodSync(destination, 0o755);
  chmodSync(join(root, 'npm', 'fastgpu.cjs'), 0o755);
} catch (error) {
  console.error(`fastgpu: npm packaging failed: ${error.message}`);
  process.exitCode = 1;
}
