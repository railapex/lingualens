// Download MarianMT ONNX models from Hugging Face
// Run: node scripts/download-models.js

import { mkdir, writeFile } from 'fs/promises';
import { existsSync } from 'fs';
import { join } from 'path';

const HF_BASE = 'https://huggingface.co';
const MODELS_DIR = join(import.meta.dirname, '..', 'models');

const MODELS = [
  {
    name: 'Xenova/opus-mt-es-en',
    dir: 'opus-mt-es-en',
    files: [
      'config.json',
      'generation_config.json',
      'tokenizer.json',
      'tokenizer_config.json',
      'onnx/encoder_model_quantized.onnx',
      'onnx/decoder_model_merged_quantized.onnx',
    ],
  },
  {
    name: 'Xenova/opus-mt-en-es',
    dir: 'opus-mt-en-es',
    files: [
      'config.json',
      'generation_config.json',
      'tokenizer.json',
      'tokenizer_config.json',
      'onnx/encoder_model_quantized.onnx',
      'onnx/decoder_model_merged_quantized.onnx',
    ],
  },
];

async function downloadFile(url, dest) {
  console.log(`  Downloading ${url.split('/').slice(-2).join('/')}...`);
  const res = await fetch(url);
  if (!res.ok) throw new Error(`HTTP ${res.status} for ${url}`);
  const buf = Buffer.from(await res.arrayBuffer());
  await writeFile(dest, buf);
  console.log(`  -> ${(buf.length / 1024 / 1024).toFixed(1)} MB`);
}

for (const model of MODELS) {
  console.log(`\nModel: ${model.name}`);
  const modelDir = join(MODELS_DIR, model.dir);

  for (const file of model.files) {
    const dest = join(modelDir, file);
    const destDir = join(dest, '..');

    if (existsSync(dest)) {
      console.log(`  Skipping ${file} (exists)`);
      continue;
    }

    await mkdir(destDir, { recursive: true });
    const url = `${HF_BASE}/${model.name}/resolve/main/${file}`;
    await downloadFile(url, dest);
  }
}

console.log('\nDone.');
