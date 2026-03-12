// Download all models for LinguaLens
// Run: node scripts/download-models.mjs
//
// Downloads to: %APPDATA%/com.lingualens.app/models/

import { createWriteStream, existsSync } from 'fs';
import { mkdir } from 'fs/promises';
import { pipeline } from 'stream/promises';
import { Readable } from 'stream';
import { join } from 'path';
import { homedir } from 'os';

const MODELS_DIR =
  process.env.LINGUALENS_MODEL_DIR ||
  (process.platform === 'win32'
    ? join(homedir(), 'AppData/Roaming/com.lingualens.app/models')
    : join(homedir(), 'Library/Application Support/com.lingualens.app/models'));

const DOWNLOADS = [
  {
    name: 'TranslateGemma 4B Q4_K_M',
    url: 'https://huggingface.co/mradermacher/translategemma-4b-it-GGUF/resolve/main/translategemma-4b-it.Q4_K_M.gguf',
    dest: 'translategemma-4b-it.Q4_K_M.gguf',
  },
];

async function download(entry) {
  const dest = join(MODELS_DIR, entry.dest);

  if (existsSync(dest)) {
    console.log(`  [skip] ${entry.name} — already exists`);
    return;
  }

  // Ensure parent directory exists
  const dir = dest.substring(0, dest.lastIndexOf('/') || dest.lastIndexOf('\\'));
  if (dir && dir !== dest) {
    await mkdir(dir, { recursive: true });
  }

  console.log(`  [download] ${entry.name}`);
  console.log(`    URL:  ${entry.url}`);
  console.log(`    Dest: ${dest}`);

  const res = await fetch(entry.url);
  if (!res.ok) throw new Error(`HTTP ${res.status} for ${entry.url}`);

  const total = parseInt(res.headers.get('content-length') || '0');
  if (total) {
    console.log(`    Size: ${(total / 1024 / 1024 / 1024).toFixed(2)} GB`);
  }

  const fileStream = createWriteStream(dest);
  const webStream = Readable.fromWeb(res.body);

  let downloaded = 0;
  webStream.on('data', (chunk) => {
    downloaded += chunk.length;
    const pct = total ? ((downloaded / total) * 100).toFixed(1) : '?';
    const mb = (downloaded / 1024 / 1024).toFixed(0);
    process.stdout.write(`\r    Progress: ${pct}% (${mb} MB)`);
  });

  await pipeline(webStream, fileStream);
  console.log('  Done.');
}

async function main() {
  console.log(`Models directory: ${MODELS_DIR}\n`);
  await mkdir(MODELS_DIR, { recursive: true });

  for (const entry of DOWNLOADS) {
    await download(entry);
  }

  console.log('\nAll models ready.');
}

main().catch((e) => {
  console.error('\nDownload failed:', e);
  process.exit(1);
});
