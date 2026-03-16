#!/usr/bin/env node
//
// GPU feature wrapper for tauri dev/build.
//
// Target-conditional deps in Cargo.toml handle CUDA/DirectML (Windows) and
// CoreML/Metal (macOS) automatically. The gpu-windows / gpu-macos feature flags
// are only needed for CI builds that use the feature-gated Cargo.toml layout.
// If the features exist, inject them; otherwise pass through cleanly.

import { spawn } from 'node:child_process';

const [subcommand = 'build', ...userArgs] = process.argv.slice(2);

const platformFeature =
  process.platform === 'darwin'
    ? 'gpu-macos'
    : process.platform === 'win32'
      ? 'gpu-windows'
      : null;

const tauriArgs = [subcommand, ...userArgs];

// Only inject --features if the caller didn't already specify them.
// The feature may not exist in Cargo.toml (target-conditional deps handle GPU
// automatically), so we probe first and skip gracefully.
const hasFeaturesArg = userArgs.some(
  (arg) => arg === '--features' || arg.startsWith('--features=')
);

let injectFeature = false;
if (platformFeature && !hasFeaturesArg) {
  // Quick check: does Cargo.toml define a [features] section with this feature?
  try {
    const { readFileSync } = await import('node:fs');
    const cargo = readFileSync('src-tauri/Cargo.toml', 'utf8');
    if (cargo.includes(`${platformFeature}`) && cargo.includes('[features]')) {
      injectFeature = true;
    }
  } catch { /* ignore — just skip injection */ }
}

if (injectFeature) {
  const passthroughIndex = tauriArgs.indexOf('--');
  if (passthroughIndex === -1) {
    tauriArgs.push('--', '--features', platformFeature);
  } else {
    tauriArgs.splice(passthroughIndex + 1, 0, '--features', platformFeature);
  }
  console.log(`[lingualens] Enabling ${platformFeature} for ${process.platform}`);
} else {
  console.log(`[lingualens] GPU deps are target-conditional; no feature flag needed`);
}

const child = spawn('npx', ['tauri', ...tauriArgs], {
  stdio: 'inherit',
  env: process.env,
  shell: true,
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
