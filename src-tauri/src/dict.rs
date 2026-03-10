// Spanish↔English dictionary — fast single-word lookup
//
// Loaded from models/dict/{es-en,en-es}.tsv at startup.
// Built by: node scripts/build-dictionary.mjs
// Source: kaikki.org Wiktionary extracts (re-runnable pipeline)
//
// Used for: (1) language detection, (2) instant single-word translation.
// Falls back to TranslateGemma for phrases and unknown words.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

struct Dict {
    es_en: HashMap<String, String>,
    en_es: HashMap<String, String>,
}

static DICT: OnceLock<Dict> = OnceLock::new();

fn load_tsv(path: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(content) = std::fs::read_to_string(path) {
        for line in content.lines() {
            if let Some((word, gloss)) = line.split_once('\t') {
                map.insert(word.to_string(), gloss.to_string());
            }
        }
    }
    map
}

fn load(data_dir: &Path) -> Dict {
    let dict_dir = data_dir.join("models").join("dict");

    let es_en = load_tsv(&dict_dir.join("es-en.tsv"));
    let en_es = load_tsv(&dict_dir.join("en-es.tsv"));

    if es_en.is_empty() && en_es.is_empty() {
        log::warn!("[dict] No dictionary files found. Run: node scripts/build-dictionary.mjs");
    } else {
        log::info!("[dict] Loaded {} es→en, {} en→es entries", es_en.len(), en_es.len());
    }

    Dict { es_en, en_es }
}

fn get_dict(data_dir: &Path) -> &'static Dict {
    DICT.get_or_init(|| load(data_dir))
}

/// Initialize dictionary eagerly (call from startup thread).
pub fn preload(data_dir: &Path) {
    let t0 = std::time::Instant::now();
    let dict = get_dict(data_dir);
    log::info!(
        "[dict] Preloaded in {:.0?} ({} es→en, {} en→es)",
        t0.elapsed(),
        dict.es_en.len(),
        dict.en_es.len()
    );
}

/// Try to translate short text using the dictionary.
/// Returns Some for 1-2 word lookups, None otherwise (fall back to model).
pub fn try_translate(text: &str, source_lang: &str, data_dir: &Path) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() || words.len() > 2 {
        return None;
    }

    let dict = get_dict(data_dir);
    let map = match source_lang {
        "es" => &dict.es_en,
        "en" => &dict.en_es,
        _ => return None,
    };

    if words.len() == 1 {
        let clean = words[0]
            .trim_matches(|c: char| !c.is_alphabetic())
            .to_lowercase();
        return map.get(&clean).cloned();
    }

    // Two words — try as phrase first, then individual
    let phrase = words
        .iter()
        .map(|w| w.trim_matches(|c: char| !c.is_alphabetic()).to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");

    if let Some(gloss) = map.get(&phrase) {
        return Some(gloss.clone());
    }

    // Both words individually
    let glosses: Vec<&str> = words
        .iter()
        .filter_map(|w| {
            let clean = w.trim_matches(|c: char| !c.is_alphabetic()).to_lowercase();
            map.get(&clean).map(|s| s.as_str())
        })
        .collect();

    if glosses.len() == words.len() {
        Some(glosses.join(" "))
    } else {
        None
    }
}

/// Detect if text is Spanish by checking dictionary membership.
/// Words in BOTH dictionaries are ambiguous (loanwords) and don't count.
/// Returns confidence: fraction of exclusively-Spanish words / total words.
pub fn spanish_confidence(text: &str, data_dir: &Path) -> f32 {
    let dict = get_dict(data_dir);

    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphabetic()).to_lowercase())
        .filter(|w| w.len() >= 2)
        .collect();

    if words.is_empty() {
        return 0.0;
    }

    let mut spanish_only = 0;
    let mut english_only = 0;
    for w in &words {
        let in_es = dict.es_en.contains_key(w.as_str());
        let in_en = dict.en_es.contains_key(w.as_str());
        if in_es && !in_en {
            spanish_only += 1;
        } else if in_en && !in_es {
            english_only += 1;
        }
        // Both or neither: ambiguous, don't count
    }

    if spanish_only == 0 && english_only == 0 {
        return 0.0; // all ambiguous or unknown — let whatlang decide
    }

    // Return fraction favoring Spanish
    spanish_only as f32 / (spanish_only + english_only) as f32
}
