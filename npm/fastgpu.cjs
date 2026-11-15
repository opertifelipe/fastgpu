#!/usr/bin/env node
'use strict';

const { accessSync, constants } = require('node:fs');
const { join } = require('node:path');

try {
  if (process.platform !== 'linux' || process.arch !== 'x64') {
    throw new Error('this npm package supports Linux x64 only');
  }
  if (typeof process.execve !== 'function') {
    throw new Error('use Node.js 22.15 or newer in the 22.x series, or Node.js 24+');
  }
  const binary = join(__dirname, 'bin', 'fastgpu');
  accessSync(binary, constants.X_OK);
  // Replace the launcher so terminals, signals, and exit codes belong to Rust.
  process.execve(binary, [binary, ...process.argv.slice(2)], process.env);
} catch (error) {
  console.error(`fastgpu: could not start the packaged binary: ${error.message}`);
  process.exitCode = 1;
}
