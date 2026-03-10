// Download and process Wiktionary dictionaries for LinguaLens
// Source: kaikki.org (English Wiktionary extracts per language)
//
// Builds:
//   es-en.tsv — Spanish → English (from Spanish Wiktionary extract)
//   en-es.tsv — English → Spanish (reverse-built from Spanish entries' translations)
//
// Run: node scripts/build-dictionary.mjs
// Re-run anytime to update from latest Wiktionary dump.
//
// Output: %APPDATA%/com.lingualens.app/models/dict/

import { createWriteStream, createReadStream, existsSync } from 'fs';
import { mkdir, stat } from 'fs/promises';
import { createInterface } from 'readline';
import { createGunzip } from 'zlib';
import { pipeline } from 'stream/promises';
import { Readable } from 'stream';
import { join } from 'path';
import { homedir } from 'os';

const MODELS_DIR =
  process.env.LINGUALENS_MODEL_DIR ||
  join(homedir(), 'AppData/Roaming/com.lingualens.app/models');

const DICT_DIR = join(MODELS_DIR, 'dict');

// POS types to include
const INCLUDE_POS = new Set([
  'noun', 'adj', 'adv', 'verb', 'pron', 'prep', 'conj', 'det',
  'intj', 'num', 'particle', 'phrase',
]);

// Tags that mark entries we don't want
const SKIP_TAGS = new Set([
  'obsolete', 'archaic', 'rare', 'dated',
]);

// ---- Download helper ----

async function downloadIfNeeded(url, dest) {
  if (existsSync(dest)) {
    const s = await stat(dest);
    if (s.size > 1_000_000) {
      console.log(`  [skip] Already downloaded (${(s.size / 1024 / 1024).toFixed(0)} MB)`);
      return;
    }
  }

  console.log(`  [download] ${url}`);
  const res = await fetch(url);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);

  const total = parseInt(res.headers.get('content-length') || '0');
  if (total) console.log(`    Size: ${(total / 1024 / 1024).toFixed(0)} MB compressed`);

  const fileStream = createWriteStream(dest);
  const webStream = Readable.fromWeb(res.body);

  let downloaded = 0;
  webStream.on('data', (chunk) => {
    downloaded += chunk.length;
    if (total) {
      const pct = ((downloaded / total) * 100).toFixed(0);
      process.stdout.write(`\r    ${pct}% (${(downloaded / 1024 / 1024).toFixed(0)} MB)`);
    }
  });

  await pipeline(webStream, fileStream);
  console.log('\n  Downloaded.');
}

// ---- Gloss extraction ----

function extractGloss(senses) {
  if (!senses || !senses.length) return null;

  const glosses = [];
  for (const sense of senses) {
    if (sense.tags && sense.tags.some(t => SKIP_TAGS.has(t))) continue;

    const g = sense.glosses;
    if (!g || !g.length) continue;

    // In kaikki format, glosses[0] is sometimes "POS: word", glosses[1] is the actual def
    const text = g.length > 1 ? g[1] : g[0];
    if (!text) continue;

    // Skip useless glosses
    if (text.length > 80) continue;
    if (text.startsWith('Alternative') || text.startsWith('Obsolete')) continue;
    if (text.startsWith('plural of ') || text.startsWith('feminine of ')) continue;
    if (text.includes('form of ') && text.length < 40) continue;

    glosses.push(text);
    if (glosses.length >= 2) break;
  }

  return glosses.length ? glosses.join('; ') : null;
}

// ---- Extract Spanish translations from English entries ----

function extractSpanishTranslations(entry) {
  // English Wiktionary entries have a "translations" field with per-language translations
  const translations = [];

  if (entry.translations) {
    for (const t of entry.translations) {
      if (t.lang === 'Spanish' || t.code === 'es') {
        if (t.word && t.word.length >= 2 && t.word.length <= 40) {
          translations.push(t.word);
        }
      }
    }
  }

  // Also check senses for translation tags
  if (entry.senses) {
    for (const sense of entry.senses) {
      if (sense.translations) {
        for (const t of sense.translations) {
          if ((t.lang === 'Spanish' || t.code === 'es') && t.word) {
            if (t.word.length >= 2 && t.word.length <= 40) {
              translations.push(t.word);
            }
          }
        }
      }
    }
  }

  return [...new Set(translations)].slice(0, 3);
}

// ---- Process Spanish JSONL → es-en dict ----

async function processSpanish(gzPath) {
  console.log('\n  [process] Spanish → English...');

  const dict = new Map();
  const forms = new Map();
  let lineCount = 0;
  let kept = 0;

  const gunzip = createGunzip();
  const input = createReadStream(gzPath).pipe(gunzip);
  const rl = createInterface({ input, crlfDelay: Infinity });

  for await (const line of rl) {
    lineCount++;
    if (lineCount % 100000 === 0) {
      process.stdout.write(`\r    ${lineCount} entries, ${kept} kept...`);
    }

    let entry;
    try { entry = JSON.parse(line); } catch { continue; }

    const word = entry.word;
    if (!word || word.length < 2 || word.length > 40) continue;
    if (word.startsWith('-') || word.startsWith(' ')) continue; // skip affixes
    if (word.includes(' ') && word.length > 30) continue;

    const pos = entry.pos;
    if (!pos || !INCLUDE_POS.has(pos)) continue;

    // Skip pure form-of entries for verbs (conjugations bloat the dict)
    const isFormOf = entry.senses?.every(s =>
      s.form_of || (s.tags && s.tags.includes('form-of'))
    );
    if (isFormOf && pos === 'verb') continue;

    const gloss = extractGloss(entry.senses);
    if (!gloss) continue;

    const lower = word.toLowerCase();

    if (!dict.has(lower) || gloss.length < dict.get(lower).length) {
      dict.set(lower, gloss);
      kept++;
    }

    // Index inflected forms (plural, feminine, etc.) — but only nouns/adj
    if (entry.forms && (pos === 'noun' || pos === 'adj')) {
      for (const f of entry.forms) {
        if (!f.form || f.form === word) continue;
        const fl = f.form.toLowerCase();
        if (fl.length >= 2 && fl.length <= 40 && !fl.startsWith('-')) {
          if (!forms.has(fl)) forms.set(fl, lower);
        }
      }
    }
  }

  process.stdout.write(`\r    ${lineCount} entries, ${kept} headwords kept\n`);

  // Add inflected forms pointing to headword's gloss
  let formCount = 0;
  for (const [form, headword] of forms) {
    if (!dict.has(form) && dict.has(headword)) {
      dict.set(form, dict.get(headword));
      formCount++;
    }
  }
  console.log(`    + ${formCount} inflected forms`);
  console.log(`    = ${dict.size} total entries`);

  return dict;
}

// ---- Process English JSONL → en-es dict (from translation fields) ----

async function processEnglish(gzPath) {
  console.log('\n  [process] English → Spanish (from translation fields)...');

  const dict = new Map();
  let lineCount = 0;
  let kept = 0;

  const gunzip = createGunzip();
  const input = createReadStream(gzPath).pipe(gunzip);
  const rl = createInterface({ input, crlfDelay: Infinity });

  for await (const line of rl) {
    lineCount++;
    if (lineCount % 100000 === 0) {
      process.stdout.write(`\r    ${lineCount} entries, ${kept} kept...`);
    }

    let entry;
    try { entry = JSON.parse(line); } catch { continue; }

    const word = entry.word;
    if (!word || word.length < 2 || word.length > 40) continue;
    if (word.startsWith('-') || word.startsWith(' ')) continue;

    const pos = entry.pos;
    if (!pos || !INCLUDE_POS.has(pos)) continue;

    const translations = extractSpanishTranslations(entry);
    if (!translations.length) continue;

    const lower = word.toLowerCase();
    const gloss = translations.join('; ');

    if (!dict.has(lower) || gloss.length < dict.get(lower).length) {
      dict.set(lower, gloss);
      kept++;
    }
  }

  process.stdout.write(`\r    ${lineCount} entries, ${kept} with Spanish translations\n`);
  console.log(`    = ${dict.size} total entries`);

  return dict;
}

// ---- Write TSV ----

async function writeTsv(dict, outPath) {
  const sorted = [...dict.entries()].sort((a, b) => a[0].localeCompare(b[0]));

  console.log(`\n  [write] ${sorted.length} entries → ${outPath}`);

  const out = createWriteStream(outPath);
  for (const [word, gloss] of sorted) {
    out.write(`${word}\t${gloss}\n`);
  }
  out.end();
  await new Promise(r => out.on('finish', r));

  const s = await stat(outPath);
  console.log(`    Size: ${(s.size / 1024 / 1024).toFixed(1)} MB`);
}

// ---- Main ----

async function main() {
  console.log(`Dictionary output: ${DICT_DIR}\n`);
  await mkdir(DICT_DIR, { recursive: true });

  // Spanish dictionary (Spanish words with English definitions)
  const esGz = join(DICT_DIR, 'kaikki-spanish.jsonl.gz');
  console.log('--- Spanish → English ---');
  await downloadIfNeeded(
    'https://kaikki.org/dictionary/Spanish/kaikki.org-dictionary-Spanish.jsonl.gz',
    esGz,
  );
  const esEn = await processSpanish(esGz);
  await writeTsv(esEn, join(DICT_DIR, 'es-en.tsv'));

  // English dictionary (English words, extract Spanish translations)
  const enGz = join(DICT_DIR, 'kaikki-english.jsonl.gz');
  console.log('\n--- English → Spanish ---');
  await downloadIfNeeded(
    'https://kaikki.org/dictionary/English/kaikki.org-dictionary-English.jsonl.gz',
    enGz,
  );
  const enEs = await processEnglish(enGz);
  await writeTsv(enEs, join(DICT_DIR, 'en-es.tsv'));

  console.log('\nDictionary build complete.');
  console.log(`  es→en: ${esEn.size} entries`);
  console.log(`  en→es: ${enEs.size} entries`);
}

main().catch((e) => {
  console.error('\nBuild failed:', e);
  process.exit(1);
});
