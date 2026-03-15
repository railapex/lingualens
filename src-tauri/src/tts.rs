// Kokoro TTS — 82M ONNX model, GPU-accelerated via ort
// Pipeline: text → espeak-ng IPA → tokenize → ONNX inference → WAV bytes
//
// GPU cascade:
// - macOS: CoreML
// - Windows: CUDA → DirectML
// - fallback: CPU
// Model files in app data dir: .../com.lingualens.app/models/kokoro/

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::sync::OnceLock;

use hound::{SampleFormat, WavSpec, WavWriter};
use ort::session::Session;
use ort::value::Tensor;
use serde::Serialize;

const SAMPLE_RATE: u32 = 24000;
const VECTOR_DIM: usize = 256;
const MAX_TOKENS: usize = 510; // voice files have 510 style vectors

// Default voices per language (Kokoro ONNX voice file stems)

// espeak-ng language codes
fn espeak_lang(lang: &str) -> &str {
    match lang {
        "es" => "es",
        "en" => "en-us",
        "fr" => "fr",
        "it" => "it",
        "pt" => "pt-br",
        "de" => "de",
        "ja" => "ja",
        "zh" => "cmn",
        _ => "en-us",
    }
}

fn default_voice(lang: &str) -> &str {
    match lang {
        "es" => "ef_dora",
        "en" => "af_heart",
        "fr" => "ff_siwis",
        "de" => "df_anna",
        "it" => "if_sara",
        "pt" => "pf_dora",
        "ja" => "jf_alpha",
        "zh" => "zf_xiaobei",
        _ => "af_heart", // fallback to English
    }
}

#[derive(Serialize, Clone)]
pub struct VoiceInfo {
    pub name: String,
    pub lang: String,
    pub gender: String,
}

#[derive(Serialize, Clone)]
pub struct TtsStatus {
    pub ready: bool,
    pub device: String,
    pub model_loaded: bool,
}

// --- Tokenizer ---

struct Tokenizer {
    vocab: HashMap<String, i64>,
    max_token_len: usize,
}

impl Tokenizer {
    fn from_json(path: &Path) -> Result<Self, String> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read tokenizer.json: {e}"))?;
        let json: serde_json::Value =
            serde_json::from_str(&data).map_err(|e| format!("Invalid tokenizer JSON: {e}"))?;

        let vocab_obj = json
            .get("model")
            .and_then(|m| m.get("vocab"))
            .and_then(|v| v.as_object())
            .ok_or("Missing model.vocab in tokenizer.json")?;

        let mut vocab = HashMap::new();
        let mut max_token_len = 0;
        for (key, val) in vocab_obj {
            if let Some(id) = val.as_i64() {
                let char_count = key.chars().count();
                if char_count > max_token_len {
                    max_token_len = char_count;
                }
                vocab.insert(key.clone(), id);
            }
        }

        Ok(Tokenizer {
            vocab,
            max_token_len,
        })
    }

    fn tokenize(&self, ipa: &str) -> Vec<i64> {
        let chars: Vec<char> = ipa.chars().collect();
        let mut tokens = vec![0i64]; // BOS = $
        let mut i = 0;

        while i < chars.len() {
            let mut matched = false;
            // Greedy longest match
            let max_len = self.max_token_len.min(chars.len() - i);
            for len in (1..=max_len).rev() {
                let substr: String = chars[i..i + len].iter().collect();
                if let Some(&id) = self.vocab.get(&substr) {
                    tokens.push(id);
                    i += len;
                    matched = true;
                    break;
                }
            }
            if !matched {
                // Skip unknown character
                i += 1;
            }
        }

        tokens.push(0i64); // EOS = $

        // Truncate if too long (preserve BOS/EOS)
        if tokens.len() > MAX_TOKENS {
            tokens.truncate(MAX_TOKENS - 1);
            tokens.push(0i64);
        }

        tokens
    }
}

// --- Voice loading ---

fn load_voice(path: &Path) -> Result<Vec<f32>, String> {
    let data =
        std::fs::read(path).map_err(|e| format!("Failed to read voice file {}: {e}", path.display()))?;
    if data.len() % 4 != 0 {
        return Err(format!(
            "Voice file {} has invalid size: {} bytes",
            path.display(),
            data.len()
        ));
    }
    let floats: Vec<f32> = data
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    Ok(floats)
}

fn get_style_vector(voice_data: &[f32], token_count: usize) -> Vec<f32> {
    let num_vectors = voice_data.len() / VECTOR_DIM;
    let index = if token_count < num_vectors {
        token_count
    } else {
        num_vectors - 1
    };
    let start = index * VECTOR_DIM;
    voice_data[start..start + VECTOR_DIM].to_vec()
}

// --- Phonemization ---

pub fn phonemize(text: &str, lang: &str) -> Result<String, String> {
    let espeak = espeak_lang(lang);
    let mut cmd = StdCommand::new(crate::espeak_exe());
    cmd.env("ESPEAK_DATA_PATH", crate::espeak_data())
       .args(["-v", espeak, "--ipa", "-q", text]);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let output = cmd.output()
        .map_err(|e| format!("Failed to run espeak-ng: {e}"))?;

    if output.status.success() {
        // espeak-ng may output multiple lines; join with space
        let ipa = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        // Re-inject punctuation — espeak strips it but Kokoro needs it for pauses.
        // Walk original text, find punctuation after words, insert into IPA at word boundaries.
        let ipa = inject_punctuation(text, &ipa);
        Ok(ipa)
    } else {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        Err(format!("espeak-ng error: {err}"))
    }
}

/// Re-inject punctuation from original text into IPA string.
/// espeak-ng strips ,.!?;: but Kokoro uses them as pause tokens.
fn inject_punctuation(original: &str, ipa: &str) -> String {
    // Extract trailing punctuation from each word in the original text
    let punct_after: Vec<Option<char>> = original
        .split_whitespace()
        .map(|word| {
            word.chars()
                .rev()
                .find(|c| matches!(c, ',' | '.' | '!' | '?' | ';' | ':' | '…'))
        })
        .collect();

    // Split IPA into words (space-separated) and append punctuation
    let ipa_words: Vec<&str> = ipa.split_whitespace().collect();
    let mut result = String::new();

    for (i, ipa_word) in ipa_words.iter().enumerate() {
        if i > 0 {
            result.push(' ');
        }
        result.push_str(ipa_word);
        if let Some(Some(punct)) = punct_after.get(i) {
            result.push(*punct);
        }
    }

    result
}

// --- WAV encoding ---

pub fn encode_wav(samples: &[f32]) -> Result<Vec<u8>, String> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer =
            WavWriter::new(&mut cursor, spec).map_err(|e| format!("WAV writer error: {e}"))?;

        // 0.1s silence prefix
        let silence_samples = (SAMPLE_RATE as usize) / 10;
        for _ in 0..silence_samples {
            writer
                .write_sample(0i16)
                .map_err(|e| format!("WAV write error: {e}"))?;
        }

        // Audio samples
        for &sample in samples {
            let clamped = sample.clamp(-1.0, 1.0);
            let amplitude = (clamped * i16::MAX as f32) as i16;
            writer
                .write_sample(amplitude)
                .map_err(|e| format!("WAV write error: {e}"))?;
        }

        // 0.1s silence suffix
        for _ in 0..silence_samples {
            writer
                .write_sample(0i16)
                .map_err(|e| format!("WAV write error: {e}"))?;
        }

        writer
            .finalize()
            .map_err(|e| format!("WAV finalize error: {e}"))?;
    }
    Ok(cursor.into_inner())
}

// --- ONNX Session ---

struct TtsState {
    session: Session,
    tokenizer: Tokenizer,
    voices: HashMap<String, Vec<f32>>,
    device: String,
    model_dir: PathBuf,
}

static STATE: OnceLock<Result<std::sync::Mutex<TtsState>, String>> = OnceLock::new();

fn models_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models").join("kokoro")
}

fn init_state(data_dir: &Path) -> Result<std::sync::Mutex<TtsState>, String> {
    let dir = models_dir(data_dir);
    let tokenizer_path = dir.join("tokenizer.json");
    let fp32_model = dir.join("model.onnx");
    let q8_model = dir.join("model_quantized.onnx");

    if !tokenizer_path.exists() {
        return Err("Kokoro tokenizer.json not found. Run model download first.".into());
    }

    let tokenizer = Tokenizer::from_json(&tokenizer_path)?;

    // Try GPU model (fp32) with platform-appropriate execution provider; fall back to CPU with q8
    let (session, device) = create_session(&fp32_model, &q8_model)?;

    log::info!("Kokoro TTS initialized on {device}");

    Ok(std::sync::Mutex::new(TtsState {
        session,
        tokenizer,
        voices: HashMap::new(),
        device,
        model_dir: dir,
    }))
}

fn create_session(fp32_path: &Path, q8_path: &Path) -> Result<(Session, String), String> {
    if !crate::config::get().force_cpu && fp32_path.exists() {
        #[cfg(all(target_os = "macos", feature = "gpu-macos"))]
        {
            // macOS: prefer CoreML first to avoid unnecessary provider probing latency.
            let t0 = std::time::Instant::now();
            match Session::builder()
                .map_err(|e| e.to_string())
                .and_then(|b| b.with_execution_providers([ort::ep::CoreML::default().build()]).map_err(|e| e.to_string()))
                .and_then(|mut b| b.commit_from_file(fp32_path).map_err(|e| e.to_string()))
            {
                Ok(session) => {
                    log::info!("[tts] CoreML session created in {:.0?}", t0.elapsed());
                    return Ok((session, "coreml".into()));
                }
                Err(e) => log::warn!("[tts] CoreML failed ({:.0?}): {}", t0.elapsed(), e),
            }
        }

        #[cfg(all(target_os = "windows", feature = "gpu-windows"))]
        {
            // CUDA attempt
            let t0 = std::time::Instant::now();
            match Session::builder()
                .map_err(|e| e.to_string())
                .and_then(|b| b.with_execution_providers([ort::ep::CUDA::default().build()]).map_err(|e| e.to_string()))
                .and_then(|mut b| b.commit_from_file(fp32_path).map_err(|e| e.to_string()))
            {
                Ok(session) => {
                    log::info!("[tts] CUDA session created in {:.0?}", t0.elapsed());
                    return Ok((session, "cuda".into()));
                }
                Err(e) => log::warn!("[tts] CUDA failed ({:.0?}): {}", t0.elapsed(), e),
            }

            // DirectML attempt
            let t0 = std::time::Instant::now();
            match Session::builder()
                .map_err(|e| e.to_string())
                .and_then(|b| b.with_execution_providers([ort::ep::DirectML::default().build()]).map_err(|e| e.to_string()))
                .and_then(|mut b| b.commit_from_file(fp32_path).map_err(|e| e.to_string()))
            {
                Ok(session) => {
                    log::info!("[tts] DirectML session created in {:.0?}", t0.elapsed());
                    return Ok((session, "directml".into()));
                }
                Err(e) => log::warn!("[tts] DirectML failed ({:.0?}): {}", t0.elapsed(), e),
            }
        }

        #[cfg(all(target_os = "macos", not(feature = "gpu-macos")))]
        log::warn!("[tts] gpu-macos feature disabled; CoreML unavailable");

        #[cfg(all(target_os = "windows", not(feature = "gpu-windows")))]
        log::warn!("[tts] gpu-windows feature disabled; CUDA/DirectML unavailable");
    }

    // CPU fallback
    let t0 = std::time::Instant::now();
    let model_path = if q8_path.exists() { q8_path } else if fp32_path.exists() { fp32_path } else {
        return Err("No Kokoro model file found. Download model.onnx or model_quantized.onnx.".into());
    };
    let session = Session::builder()
        .map_err(|e| format!("ONNX session builder: {e}"))
        .and_then(|mut b| b.commit_from_file(model_path).map_err(|e| format!("Kokoro load: {e}")))?;
    log::info!("[tts] CPU session in {:.0?} ({})", t0.elapsed(), model_path.display());
    Ok((session, "cpu".into()))
}

fn get_or_init_state(data_dir: &Path) -> Result<&'static std::sync::Mutex<TtsState>, String> {
    let result = STATE.get_or_init(|| init_state(data_dir));
    match result {
        Ok(mutex) => Ok(mutex),
        Err(e) => Err(e.clone()),
    }
}

fn ensure_voice_loaded(
    state: &mut TtsState,
    voice_name: &str,
) -> Result<(), String> {
    if state.voices.contains_key(voice_name) {
        return Ok(());
    }
    let voice_path = state.model_dir.join("voices").join(format!("{voice_name}.bin"));
    if !voice_path.exists() {
        return Err(format!("Voice file not found: {}", voice_path.display()));
    }
    let voice_data = load_voice(&voice_path)?;
    state.voices.insert(voice_name.to_string(), voice_data);
    Ok(())
}

/// Run a minimal inference to trigger GPU kernel compilation.
/// This takes 3-10 seconds on first call (provider kernel compile/JIT) but makes subsequent
/// inferences near-instant.
fn warmup_inference(state: &mut TtsState) -> Result<(), String> {
    let voice_data = state.voices.values().next()
        .ok_or("No voices loaded for warmup")?;

    // Minimal token sequence: BOS + one phoneme + EOS
    let tokens = vec![0i64, 1, 0];
    let style = get_style_vector(voice_data, tokens.len());

    let input_ids = Tensor::from_array(([1, tokens.len() as i64], tokens))
        .map_err(|e| format!("warmup tensor: {e}"))?;
    let style_tensor = Tensor::from_array(([1i64, VECTOR_DIM as i64], style))
        .map_err(|e| format!("warmup style: {e}"))?;
    let speed_tensor = Tensor::from_array(([1i64], vec![1.0f32]))
        .map_err(|e| format!("warmup speed: {e}"))?;

    let inputs = ort::inputs![
        "input_ids" => input_ids,
        "style" => style_tensor,
        "speed" => speed_tensor,
    ];

    let _ = state.session.run(inputs)
        .map_err(|e| format!("warmup inference: {e}"))?;

    Ok(())
}

// --- Public API ---

/// Pre-initialize ONNX session + voices for configured languages.
/// Call from a background thread on startup.
pub fn preload(data_dir: &Path) {
    let t0 = std::time::Instant::now();
    match get_or_init_state(data_dir) {
        Ok(mutex) => {
            // Preload voices for configured target + native languages
            let cfg = crate::config::get();
            let target_voice = cfg.tts_voice_target.as_deref()
                .unwrap_or_else(|| default_voice(&cfg.target_lang));
            let native_voice = cfg.tts_voice_native.as_deref()
                .unwrap_or_else(|| default_voice(&cfg.native_lang));

            if let Ok(mut state) = mutex.lock() {
                let _ = ensure_voice_loaded(&mut state, target_voice);
                if target_voice != native_voice {
                    let _ = ensure_voice_loaded(&mut state, native_voice);
                }

                // Warmup inference — triggers provider kernel compilation
                // so the first real speak() call doesn't pay the JIT tax (~5-10s).
                match warmup_inference(&mut state) {
                    Ok(()) => log::info!("[kokoro] Warmup inference complete"),
                    Err(e) => log::warn!("[kokoro] Warmup failed (non-fatal): {e}"),
                }
            }
            log::info!("[kokoro] Preloaded in {:.0?}", t0.elapsed());
        }
        Err(e) => {
            log::error!("[kokoro] Preload failed: {e}");
        }
    }
}

pub fn speak(text: &str, lang: &str, voice: Option<&str>, speed: Option<f32>, _data_dir: &Path) -> Result<Vec<u8>, String> {
    if text.trim().is_empty() {
        return Err("Empty text".into());
    }

    if crate::config::get().force_web_speech {
        return Err("Kokoro disabled (force_web_speech)".into());
    }

    // Fail-fast: if TTS isn't initialized yet (preload still running),
    // return error immediately so frontend falls back to Web Speech API.
    // Don't trigger lazy init from user requests — only preload() does that.
    let state_mutex = match STATE.get() {
        Some(Ok(m)) => m,
        Some(Err(e)) => return Err(e.clone()),
        None => return Err("TTS still loading".into()),
    };

    // Non-blocking lock — preload may hold this during warmup inference.
    // If busy, return error → Web Speech fallback. Kokoro takes over once warm.
    let mut state = match state_mutex.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::WouldBlock) => {
            return Err("TTS busy (warming up)".into());
        }
        Err(std::sync::TryLockError::Poisoned(e)) => {
            return Err(format!("TTS lock poisoned: {e}"));
        }
    };

    let voice_name = voice.unwrap_or_else(|| default_voice(lang));
    ensure_voice_loaded(&mut state, voice_name)?;

    let t_total = std::time::Instant::now();

    // 1. Phonemize
    let t0 = std::time::Instant::now();
    let ipa = phonemize(text, lang)?;
    log::info!("[tts] phonemize: {:.0?}", t0.elapsed());
    log::info!("[kokoro] {lang} IPA: {ipa}");

    // 2. Tokenize
    let tokens = state.tokenizer.tokenize(&ipa);
    let token_count = tokens.len();
    log::info!("[kokoro] {token_count} tokens");

    // 3. Get style vector
    let voice_data = state.voices.get(voice_name).unwrap();
    let style = get_style_vector(voice_data, token_count);

    // 4. ONNX inference
    let input_ids = Tensor::from_array(([1, token_count as i64], tokens))
        .map_err(|e| format!("input_ids tensor error: {e}"))?;

    let style_tensor = Tensor::from_array(([1i64, VECTOR_DIM as i64], style))
        .map_err(|e| format!("style tensor error: {e}"))?;

    let speed_val = speed.unwrap_or(1.0);
    let speed_tensor = Tensor::from_array(([1i64], vec![speed_val]))
        .map_err(|e| format!("speed tensor error: {e}"))?;

    let inputs = ort::inputs![
        "input_ids" => input_ids,
        "style" => style_tensor,
        "speed" => speed_tensor,
    ];

    let t0 = std::time::Instant::now();
    let outputs = state
        .session
        .run(inputs)
        .map_err(|e| format!("ONNX inference error: {e}"))?;
    log::info!("[tts] inference: {:.0?}", t0.elapsed());

    // 5. Extract audio samples
    let (_shape, samples_slice) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("Failed to extract output tensor: {e}"))?;

    let samples: Vec<f32> = samples_slice.to_vec();
    log::info!("[kokoro] Generated {} samples ({:.1}s audio)", samples.len(), samples.len() as f32 / SAMPLE_RATE as f32);

    // 6. Encode WAV
    let wav = encode_wav(&samples)?;
    log::info!("[tts] total: {:.0?}", t_total.elapsed());
    Ok(wav)
}

pub fn list_voices(data_dir: &Path) -> Vec<VoiceInfo> {
    let voices_dir = models_dir(data_dir).join("voices");
    let mut voices = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&voices_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "bin") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    let name = stem.to_string();
                    let lang = match name.chars().next() {
                        Some('a') | Some('b') => "en".to_string(),
                        Some('e') => "es".to_string(),
                        Some('f') => "fr".to_string(),
                        Some('h') => "hi".to_string(),
                        Some('i') => "it".to_string(),
                        Some('j') => "ja".to_string(),
                        Some('p') => "pt".to_string(),
                        Some('z') => "zh".to_string(),
                        _ => "unknown".to_string(),
                    };
                    let gender = match name.chars().nth(1) {
                        Some('f') => "female".to_string(),
                        Some('m') => "male".to_string(),
                        _ => "unknown".to_string(),
                    };
                    voices.push(VoiceInfo { name, lang, gender });
                }
            }
        }
    }

    voices.sort_by(|a, b| a.lang.cmp(&b.lang).then(a.name.cmp(&b.name)));
    voices
}

pub fn status(data_dir: &Path) -> TtsStatus {
    match STATE.get() {
        Some(Ok(mutex)) => {
            if let Ok(state) = mutex.lock() {
                TtsStatus {
                    ready: true,
                    device: state.device.clone(),
                    model_loaded: true,
                }
            } else {
                TtsStatus {
                    ready: false,
                    device: "unknown".into(),
                    model_loaded: false,
                }
            }
        }
        Some(Err(_)) => TtsStatus {
            ready: false,
            device: "error".into(),
            model_loaded: false,
        },
        None => {
            // Check if models exist but not yet loaded
            let dir = models_dir(data_dir);
            let has_model = dir.join("model.onnx").exists() || dir.join("model_quantized.onnx").exists();
            TtsStatus {
                ready: false,
                device: "not_initialized".into(),
                model_loaded: has_model,
            }
        }
    }
}

pub fn get_device() -> String {
    match STATE.get() {
        Some(Ok(mutex)) => {
            if let Ok(state) = mutex.lock() {
                state.device.clone()
            } else {
                "lock_error".into()
            }
        }
        Some(Err(_)) => "error".into(),
        None => "not_loaded".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_data_dir() -> PathBuf {
        // Use the actual app data dir for tests — platform-aware
        if cfg!(target_os = "windows") {
            PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| "C:/Users/chris/AppData/Roaming".into()))
                .join("com.lingualens.app")
        } else {
            // macOS: ~/Library/Application Support/com.lingualens.app
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
                .join("Library/Application Support/com.lingualens.app")
        }
    }

    fn kokoro_dir() -> PathBuf {
        test_data_dir().join("models").join("kokoro")
    }

    fn has_models() -> bool {
        let dir = kokoro_dir();
        (dir.join("model.onnx").exists() || dir.join("model_quantized.onnx").exists())
            && dir.join("tokenizer.json").exists()
            && dir.join("voices/ef_dora.bin").exists()
    }

    // --- Unit tests (no model needed) ---

    #[test]
    fn test_phonemize_spanish() {
        let ipa = phonemize("Buenos días", "es").unwrap();
        assert!(ipa.contains("bw"), "Expected 'bw' in Spanish IPA: {ipa}");
    }

    #[test]
    fn test_phonemize_english() {
        let ipa = phonemize("Good morning", "en").unwrap();
        assert!(
            ipa.contains("ɡ") || ipa.contains("g"),
            "Expected 'ɡ' in English IPA: {ipa}"
        );
    }

    #[test]
    fn test_phonemize_mi_not_my() {
        let ipa = phonemize("mi amor", "es").unwrap();
        assert!(ipa.contains("mi"), "Spanish 'mi' should be [mi], got: {ipa}");
        assert!(
            !ipa.contains("maɪ"),
            "Spanish 'mi' should NOT be English [maɪ], got: {ipa}"
        );
    }

    #[test]
    fn test_phonemize_empty() {
        // espeak-ng handles empty gracefully
        let result = phonemize("", "es");
        assert!(result.is_ok());
    }

    #[test]
    fn test_tokenizer_load() {
        let path = kokoro_dir().join("tokenizer.json");
        if !path.exists() {
            return; // skip if no tokenizer
        }
        let tok = Tokenizer::from_json(&path).unwrap();
        assert!(tok.vocab.len() > 100);
        assert_eq!(tok.vocab.get("$"), Some(&0)); // BOS/EOS
        assert_eq!(tok.vocab.get(" "), Some(&16));
    }

    #[test]
    fn test_tokenize_ipa() {
        let path = kokoro_dir().join("tokenizer.json");
        if !path.exists() {
            return;
        }
        let tok = Tokenizer::from_json(&path).unwrap();
        // Simple Spanish phonemes
        let tokens = tok.tokenize("ˈola");
        assert_eq!(tokens[0], 0); // BOS
        assert_eq!(*tokens.last().unwrap(), 0); // EOS
        assert!(tokens.len() >= 4); // BOS + at least 2 phoneme tokens + EOS
    }

    #[test]
    fn test_encode_wav_header() {
        let samples = vec![0.0f32; 2400]; // 0.1s at 24kHz
        let wav = encode_wav(&samples).unwrap();
        // Check RIFF header
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        // Check fmt chunk
        assert_eq!(&wav[12..16], b"fmt ");
    }

    #[test]
    fn test_encode_wav_correct_length() {
        let samples = vec![0.5f32; 24000]; // 1 second of audio
        let wav = encode_wav(&samples).unwrap();
        // 1s audio + 0.1s prefix + 0.1s suffix = 1.2s = 28800 samples
        // 28800 samples * 2 bytes = 57600 data bytes + 44 header
        let expected_data = 28800 * 2;
        let expected_total = expected_data + 44;
        assert_eq!(wav.len(), expected_total);
    }

    #[test]
    fn test_voice_file_loading() {
        let path = kokoro_dir().join("voices/ef_dora.bin");
        if !path.exists() {
            return;
        }
        let data = load_voice(&path).unwrap();
        assert_eq!(data.len() % VECTOR_DIM, 0);
        let num_vectors = data.len() / VECTOR_DIM;
        assert!(num_vectors > 0 && num_vectors <= 512);
    }

    #[test]
    fn test_style_vector_selection() {
        // Create fake voice data: 3 vectors of 256 floats
        let mut data = vec![0.0f32; 3 * VECTOR_DIM];
        data[0] = 1.0; // vector 0
        data[VECTOR_DIM] = 2.0; // vector 1
        data[2 * VECTOR_DIM] = 3.0; // vector 2

        let v0 = get_style_vector(&data, 0);
        assert_eq!(v0[0], 1.0);

        let v1 = get_style_vector(&data, 1);
        assert_eq!(v1[0], 2.0);

        // Out of bounds → last vector
        let v_oob = get_style_vector(&data, 100);
        assert_eq!(v_oob[0], 3.0);
    }

    #[test]
    fn test_list_voices() {
        let voices = list_voices(&test_data_dir());
        if kokoro_dir().join("voices").exists() {
            assert!(!voices.is_empty());
            // ef_dora should be listed
            assert!(voices.iter().any(|v| v.name == "ef_dora"));
        }
    }

    // --- Integration tests (require model) ---

    #[test]
    fn test_speak_spanish_short() {
        if !has_models() {
            eprintln!("Skipping: model files not found");
            return;
        }
        let wav = speak("hola", "es", None, None, &test_data_dir()).unwrap();
        assert!(wav.len() > 44, "WAV should have more than just a header");
        assert_eq!(&wav[0..4], b"RIFF");
    }

    #[test]
    fn test_speak_spanish_phrase() {
        if !has_models() {
            eprintln!("Skipping: model files not found");
            return;
        }
        let wav = speak("Buenos días, mi amor", "es", None, None, &test_data_dir()).unwrap();
        assert!(wav.len() > 1000, "WAV for a phrase should be substantial");
        // Rough check: at least 0.5s of audio (24000 * 0.5 * 2 bytes + header)
        assert!(wav.len() > 24000);
    }

    #[test]
    fn test_speak_english() {
        if !has_models() {
            eprintln!("Skipping: model files not found");
            return;
        }
        let wav = speak("Good morning", "en", None, None, &test_data_dir()).unwrap();
        assert!(wav.len() > 44);
        assert_eq!(&wav[0..4], b"RIFF");
    }

    #[test]
    fn test_speak_empty_returns_error() {
        if !has_models() {
            return;
        }
        let result = speak("", "es", None, None, &test_data_dir());
        assert!(result.is_err());
    }

    #[test]
    fn test_speak_writes_playable_wav() {
        if !has_models() {
            return;
        }
        let wav = speak("hola", "es", None, None, &test_data_dir()).unwrap();
        // Verify hound can parse the WAV we generated
        let cursor = Cursor::new(wav);
        let reader = hound::WavReader::new(cursor).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, SAMPLE_RATE);
        assert_eq!(spec.bits_per_sample, 16);
    }

    #[test]
    fn test_gpu_detection() {
        if !has_models() {
            return;
        }
        let status = status(&test_data_dir());
        // After speak() ran above (if tests run sequentially), should be ready
        // But if this runs first, might not be initialized yet
        println!("TTS status: ready={}, device={}", status.ready, status.device);
    }

    #[test]
    fn test_write_wav_to_disk() {
        if !has_models() {
            return;
        }
        let t0 = std::time::Instant::now();
        let wav = speak("Buenos días, mi amor", "es", None, None, &test_data_dir()).unwrap();
        let elapsed = t0.elapsed();
        eprintln!("[kokoro] Generated {} bytes WAV in {:.0?}", wav.len(), elapsed);

        let status = status(&test_data_dir());
        eprintln!("[kokoro] Device: {}", status.device);

        // Write to temp for manual listening
        let out_path = std::env::temp_dir().join("kokoro_test_es.wav");
        std::fs::write(&out_path, &wav).unwrap();
        eprintln!("[kokoro] Wrote: {}", out_path.display());

        // English too
        let wav_en = speak("Good morning, my love", "en", None, None, &test_data_dir()).unwrap();
        let out_en = std::env::temp_dir().join("kokoro_test_en.wav");
        std::fs::write(&out_en, &wav_en).unwrap();
        eprintln!("[kokoro] Wrote: {}", out_en.display());
    }
}
