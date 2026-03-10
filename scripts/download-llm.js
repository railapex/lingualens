// Download TranslateGemma 4B GGUF model for the LLM sidecar
// Run: node scripts/download-llm.js
//
// Set LINGUALENS_MODEL_DIR to override the default download location.
// Default: %APPDATA%/com.lingualens.app/models/

import { createWriteStream, existsSync } from 'fs';
import { mkdir } from 'fs/promises';
import { pipeline } from 'stream/promises';
import { Readable } from 'stream';
import { join } from 'path';
import { homedir } from 'os';

const MODEL_URL =
  'https://huggingface.co/SandLogicTechnologies/translategemma-4b-it-GGUF/resolve/main/translategemma-4b-it-Q4_K_M.gguf';
const MODEL_FILENAME = 'translategemma-4b-it-Q4_K_M.gguf';

// Match Tauri's app_data_dir on Windows
const MODELS_DIR =
  process.env.LINGUALENS_MODEL_DIR ||
  join(homedir(), 'AppData/Roaming/com.lingualens.app/models');

async function download() {
  const dest = join(MODELS_DIR, MODEL_FILENAME);

  if (existsSync(dest)) {
    console.log(`Model already exists: ${dest}`);
    return;
  }

  console.log('Downloading TranslateGemma 4B Q4_K_M...');
  console.log(`  URL: ${MODEL_URL}`);
  console.log(`  Dest: ${dest}`);

  await mkdir(MODELS_DIR, { recursive: true });

  const res = await fetch(MODEL_URL);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);

  const total = parseInt(res.headers.get('content-length') || '0');
  if (total) {
    console.log(`  Size: ${(total / 1024 / 1024 / 1024).toFixed(2)} GB`);
  }

  const fileStream = createWriteStream(dest);
  const webStream = Readable.fromWeb(res.body);

  let downloaded = 0;
  webStream.on('data', (chunk) => {
    downloaded += chunk.length;
    const pct = total ? ((downloaded / total) * 100).toFixed(1) : '?';
    const mb = (downloaded / 1024 / 1024).toFixed(0);
    process.stdout.write(`\r  Progress: ${pct}% (${mb} MB)`);
  });

  await pipeline(webStream, fileStream);
  console.log('\n  Done.');
}

download().catch((e) => {
  console.error('Download failed:', e);
  process.exit(1);
});
