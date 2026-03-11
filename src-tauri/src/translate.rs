// Translation via TranslateGemma 4B (llama-cpp-2)

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use std::num::NonZeroU32;
use std::path::Path;
use std::pin::pin;
use std::sync::{Mutex, OnceLock};

struct TranslateState {
    backend: LlamaBackend,
    model: LlamaModel,
}

static STATE: OnceLock<Mutex<TranslateState>> = OnceLock::new();

const MODEL_FILENAME: &str = "translategemma-4b-it.Q4_K_M.gguf";
const CTX_SIZE: u32 = 512;
const MAX_TOKENS: i32 = 128;

fn lang_name(code: &str) -> &str {
    match code {
        "es" => "Spanish",
        "en" => "English",
        "fr" => "French",
        "de" => "German",
        "it" => "Italian",
        "pt" => "Portuguese",
        _ => code,
    }
}

/// Build TranslateGemma's official prompt format (from the model's Jinja chat template).
/// Two blank lines before the text is intentional — it's part of the template spec.
fn build_prompt(text: &str, source_lang: &str, target_lang: &str) -> String {
    let src = lang_name(source_lang);
    let tgt = lang_name(target_lang);
    format!(
        "<start_of_turn>user\n\
You are a professional {src} ({source_lang}) to {tgt} ({target_lang}) translator. \
Your goal is to accurately convey the meaning and nuances of the original {src} text \
while adhering to {tgt} grammar, vocabulary, and cultural sensitivities.\n\
Produce only the {tgt} translation, without any additional explanations or commentary. \
Please translate the following {src} text into {tgt}:\n\
\n\
\n\
{text}<end_of_turn>\n\
<start_of_turn>model\n"
    )
}

fn ensure_loaded(data_dir: &Path) -> Result<(), String> {
    if STATE.get().is_some() {
        return Ok(());
    }

    let model_path = data_dir.join("models").join(MODEL_FILENAME);
    if !model_path.exists() {
        return Err(format!(
            "Model not found: {}. Restart the app to trigger download.",
            model_path.display()
        ));
    }

    log::info!("Loading TranslateGemma from {}", model_path.display());

    let backend = LlamaBackend::init().map_err(|e| format!("Backend init failed: {e}"))?;

    // GPU offload: all layers to primary GPU for ~10x speedup
    // force_cpu=true → 0 GPU layers (CPU-only inference, requires restart to change)
    let gpu_layers = if crate::config::get().force_cpu { 0 } else { 999 };
    let model_params = pin!(LlamaModelParams::default()
        .with_n_gpu_layers(gpu_layers)
        .with_split_mode(llama_cpp_2::model::params::LlamaSplitMode::None)
        .with_main_gpu(0));

    let model = LlamaModel::load_from_file(&backend, &model_path, &model_params)
        .map_err(|e| format!("Model load failed: {e}"))?;

    log::info!(
        "TranslateGemma loaded ({} GPU layers)",
        if gpu_layers == 0 { "0 — CPU only" } else { "all" }
    );

    let _ = STATE.set(Mutex::new(TranslateState { backend, model }));
    Ok(())
}

fn run_inference(state: &TranslateState, prompt: &str) -> Result<String, String> {
    let ctx_params = LlamaContextParams::default()
        .with_n_ctx(Some(NonZeroU32::new(CTX_SIZE).unwrap()))
        .with_n_threads(4)
        .with_n_threads_batch(4);

    let mut ctx = state
        .model
        .new_context(&state.backend, ctx_params)
        .map_err(|e| format!("Context creation failed: {e}"))?;

    let tokens = state
        .model
        .str_to_token(prompt, AddBos::Always)
        .map_err(|e| format!("Tokenization failed: {e}"))?;

    if tokens.len() as i32 + MAX_TOKENS > CTX_SIZE as i32 {
        return Err("Input too long for context window".into());
    }

    // Feed prompt tokens
    let mut batch = LlamaBatch::new(CTX_SIZE as usize, 1);
    let last_idx = (tokens.len() - 1) as i32;
    for (i, token) in (0_i32..).zip(tokens.iter()) {
        batch
            .add(*token, i, &[0], i == last_idx)
            .map_err(|e| format!("Batch add failed: {e}"))?;
    }

    ctx.decode(&mut batch)
        .map_err(|e| format!("Prompt decode failed: {e}"))?;

    // Generate with temp=0 for deterministic output
    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::temp(0.0),
        LlamaSampler::greedy(),
    ]);

    let mut output = String::new();
    let mut decoder = encoding_rs::UTF_8.new_decoder();
    let mut n_cur = batch.n_tokens();
    let n_len = tokens.len() as i32 + MAX_TOKENS;

    while n_cur <= n_len {
        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        sampler.accept(token);

        if state.model.is_eog_token(token) {
            break;
        }

        let piece = state
            .model
            .token_to_piece(token, &mut decoder, true, None)
            .map_err(|e| format!("Token decode failed: {e}"))?;

        // Stop on first newline — clean translation should be a single line.
        // Narrative/explanation comes after the first line break.
        if piece.contains('\n') && !output.is_empty() {
            break;
        }

        output.push_str(&piece);

        batch.clear();
        batch
            .add(token, n_cur, &[0], true)
            .map_err(|e| format!("Batch add failed: {e}"))?;

        ctx.decode(&mut batch)
            .map_err(|e| format!("Decode failed: {e}"))?;

        n_cur += 1;
    }

    let output = sanitize_translation(output.trim());
    Ok(output)
}

/// Clean up model output artifacts (preambles, markdown, quotes, language labels).
fn sanitize_translation(output: &str) -> String {
    let mut s = output.to_string();

    // Strip common preamble patterns
    for prefix in &[
        "here's the translation:",
        "here is the translation:",
        "here's the translation of the provided text:",
        "translated text:",
        "translation:",
        "translates to:",
    ] {
        if let Some(rest) = s.to_lowercase().strip_prefix(prefix) {
            s = s[s.len() - rest.len()..].trim().to_string();
        }
    }

    // Strip language label on first line: **Spanish**: ..., *English* - ...
    let first_line_end = s.find('\n').unwrap_or(s.len());
    let first_line = s[..first_line_end].to_string();
    let rest_of_lines = s[first_line_end..].to_string();
    for lang in &["Spanish", "English", "French", "German", "Italian", "Portuguese"] {
        for pattern in &[
            format!("**{}**:", lang), format!("**{}**-", lang),
            format!("*{}*:", lang), format!("*{}*-", lang),
            format!("{}:", lang), format!("{}-", lang),
        ] {
            if let Some(rest) = first_line.strip_prefix(pattern.as_str()) {
                s = rest.trim().to_string() + &rest_of_lines;
                break;
            }
        }
    }

    // If model gave markdown bold (**text**), extract the bold part
    if let Some(start) = s.find("**") {
        let after = &s[start + 2..];
        if let Some(end) = after.find("**") {
            s = after[..end].trim().to_string();
        }
    }

    // Strip leading markdown bullet/list markers
    s = s.trim_start_matches(|c: char| c == '*' || c == '-' || c == '•')
        .trim_start()
        .to_string();

    // Strip surrounding quotes
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s = s[1..s.len()-1].to_string();
    }

    s.trim().to_string()
}

/// Pre-load TranslateGemma model. Call from a background thread on startup.
pub fn preload(data_dir: &Path) {
    let t0 = std::time::Instant::now();
    match ensure_loaded(data_dir) {
        Ok(()) => log::info!("[translate] Preloaded in {:.0?}", t0.elapsed()),
        Err(e) => log::error!("[translate] Preload failed: {e}"),
    }
}

pub fn translate(
    text: &str,
    source_lang: &str,
    target_lang: &str,
    data_dir: &Path,
) -> Result<String, String> {
    ensure_loaded(data_dir)?;

    let state = STATE.get().unwrap().lock().map_err(|e| e.to_string())?;
    let prompt = build_prompt(text, source_lang, target_lang);
    run_inference(&state, &prompt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect_lang;
    use std::path::PathBuf;

    fn model_dir() -> PathBuf {
        let appdata = std::env::var("APPDATA").expect("APPDATA not set");
        PathBuf::from(appdata).join("com.lingualens.app")
    }

    fn require_model() -> bool {
        if !model_dir().join("models").join(MODEL_FILENAME).exists() {
            eprintln!("SKIP: model not found");
            return false;
        }
        true
    }

    /// Verify translation changed language. For short text, whatlang is unreliable
    /// so we check "not source language" rather than "is target language".
    fn assert_output_lang(result: &str, expected_lang: &str, input: &str) {
        assert!(!result.is_empty(), "Empty translation for: {input}");
        let source_lang = if expected_lang == "en" { "es" } else { "en" };
        let detected = detect_lang(result, "es", "en", &model_dir());
        assert_ne!(
            detected, source_lang,
            "'{input}' → '{result}' — still detected as {source_lang}, expected {expected_lang}"
        );
    }

    // --- Language detection ---

    #[test]
    fn test_detect_spanish_simple() {
        assert_eq!(detect_lang("buenos dias", "es", "en", &model_dir()), "es");
    }

    #[test]
    fn test_detect_spanish_complex() {
        assert_eq!(detect_lang("buenos noches, mi amor", "es", "en", &model_dir()), "es");
    }

    #[test]
    fn test_detect_spanish_sentence() {
        assert_eq!(detect_lang("me gustaría una cerveza por favor", "es", "en", &model_dir()), "es");
    }

    #[test]
    fn test_detect_spanish_short() {
        assert_eq!(detect_lang("hola", "es", "en", &model_dir()), "es");
    }

    #[test]
    fn test_detect_spanish_accented() {
        assert_eq!(detect_lang("cómo estás", "es", "en", &model_dir()), "es");
    }

    #[test]
    fn test_detect_english() {
        assert_eq!(detect_lang("good morning my love", "es", "en", &model_dir()), "en");
    }

    #[test]
    fn test_detect_english_sentence() {
        assert_eq!(detect_lang("the quick brown fox jumps over the lazy dog", "es", "en", &model_dir()), "en");
    }

    // --- Prompt format ---

    #[test]
    fn test_lang_name() {
        assert_eq!(lang_name("es"), "Spanish");
        assert_eq!(lang_name("en"), "English");
        assert_eq!(lang_name("xx"), "xx");
    }

    #[test]
    fn test_prompt_format() {
        let prompt = build_prompt("buenos dias", "es", "en");
        assert!(prompt.starts_with("<start_of_turn>user\n"));
        assert!(prompt.contains("from Spanish to English"));
        assert!(prompt.contains("buenos dias<end_of_turn>"));
        assert!(prompt.ends_with("<start_of_turn>model\n"));
    }

    // --- Translation (requires model) ---

    #[test]
    fn test_translate_simple_es_to_en() {
        if !require_model() { return; }
        let result = translate("buenos dias", "es", "en", &model_dir()).unwrap();
        eprintln!("'buenos dias' → '{result}'");
        assert_output_lang(&result, "en", "buenos dias");
    }

    #[test]
    fn test_translate_complex_es_to_en() {
        if !require_model() { return; }
        let result = translate("buenos noches, mi amor", "es", "en", &model_dir()).unwrap();
        eprintln!("'buenos noches, mi amor' → '{result}'");
        assert_output_lang(&result, "en", "buenos noches, mi amor");
    }

    #[test]
    fn test_translate_sentence_es_to_en() {
        if !require_model() { return; }
        let result = translate("me gustaría una cerveza por favor", "es", "en", &model_dir()).unwrap();
        eprintln!("'me gustaría una cerveza por favor' → '{result}'");
        assert_output_lang(&result, "en", "me gustaría una cerveza por favor");
    }

    #[test]
    fn test_translate_en_to_es() {
        if !require_model() { return; }
        let result = translate("good morning my love", "en", "es", &model_dir()).unwrap();
        eprintln!("'good morning my love' → '{result}'");
        assert_output_lang(&result, "es", "good morning my love");
    }

    #[test]
    fn test_translate_idiom_es_to_en() {
        if !require_model() { return; }
        let result = translate("no hay de qué", "es", "en", &model_dir()).unwrap();
        eprintln!("'no hay de qué' → '{result}'");
        assert_output_lang(&result, "en", "no hay de qué");
    }

    #[test]
    fn test_translate_short_phrase() {
        if !require_model() { return; }
        let result = translate("gracias", "es", "en", &model_dir()).unwrap();
        eprintln!("'gracias' → '{result}'");
        assert_output_lang(&result, "en", "gracias");
    }

    #[test]
    fn test_translate_limpieza() {
        if !require_model() { return; }
        // "limpieza" must be detected as Spanish and translated to English
        assert_eq!(detect_lang("limpieza", "es", "en", &model_dir()), "es");
        let result = translate("limpieza", "es", "en", &model_dir()).unwrap();
        eprintln!("'limpieza' → '{result}'");
        let lower = result.to_lowercase();
        assert!(
            lower.contains("clean") || lower.contains("tidy") || lower.contains("hygiene"),
            "'limpieza' should translate to something about cleaning, got: '{result}'"
        );
    }
}
