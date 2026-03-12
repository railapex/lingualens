#!/usr/bin/env node

import { spawn } from 'node:child_process';

const [subcommand = 'build', ...userArgs] = process.argv.slice(2);

const platformFeature =
  process.platform === 'darwin'
    ? 'gpu-macos'
    : process.platform === 'win32'
      ? 'gpu-windows'
      : null;

const tauriArgs = [subcommand, ...userArgs];
const hasFeaturesArg = userArgs.some(
  (arg) => arg === '--features' || arg.startsWith('--features=')
);

if (platformFeature && !hasFeaturesArg) {
  const passthroughIndex = tauriArgs.indexOf('--');
  if (passthroughIndex === -1) {
    tauriArgs.push('--', '--features', platformFeature);
  } else {
    tauriArgs.splice(passthroughIndex + 1, 0, '--features', platformFeature);
  }
  console.log(`[lingualens] Enabling ${platformFeature} for ${process.platform}`);
}

if (!platformFeature) {
  console.log(
    `[lingualens] No default GPU feature for ${process.platform}; running base tauri command`
  );
}

const npxBin = process.platform === 'win32' ? 'npx.cmd' : 'npx';
const child = spawn(npxBin, ['tauri', ...tauriArgs], {
  stdio: 'inherit',
  env: process.env,
});

child.on('error', (err) => {
  console.error(`[lingualens] Failed to start tauri command: ${err.message}`);
  process.exit(1);
});

child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 1);
});
