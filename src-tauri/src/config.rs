// App configuration — persisted to config.json in app data dir

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub target_lang: String,
    pub native_lang: String,
    pub theme: String,
    pub auto_play: bool,
    pub show_ipa: bool,
    pub dismiss_delay_ms: u32,
    pub replay_speed: f32,
    pub tts_voice_target: Option<String>,
    pub tts_voice_native: Option<String>,
    pub hotkey: String,

    // Dev/testing overrides (default: false = use best available)
    pub force_cpu: bool,           // Skip CUDA/DirectML, force CPU inference
    pub force_web_speech: bool,    // Skip Kokoro, force Web Speech API for TTS
    pub force_dict_only: bool,     // Skip TranslateGemma, dictionary-only translation
    pub force_clipboard: bool,     // Skip UIA, force clipboard simulation for text capture
}

impl Default for Config {
    fn default() -> Self {
        Config {
            target_lang: "es".into(),
            native_lang: "en".into(),
            theme: "system".into(),
            auto_play: true,
            show_ipa: true,
            dismiss_delay_ms: 2000,
            replay_speed: 0.7,
            tts_voice_target: None,
            tts_voice_native: None,
            hotkey: "ctrl+alt+l".into(),
            force_cpu: false,
            force_web_speech: false,
            force_dict_only: false,
            force_clipboard: false,
        }
    }
}

static CONFIG: OnceLock<RwLock<Config>> = OnceLock::new();

const CONFIG_FILENAME: &str = "config.json";

fn config_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join(CONFIG_FILENAME)
}

/// Check if this is first run (no config file exists).
pub fn is_first_run(data_dir: &Path) -> bool {
    !config_path(data_dir).exists()
}

/// Load config from disk, or return defaults if not found.
pub fn load(data_dir: &Path) -> Config {
    let path = config_path(data_dir);
    if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str(&json) {
                Ok(config) => return config,
                Err(e) => log::warn!("[config] Parse error, using defaults: {e}"),
            },
            Err(e) => log::warn!("[config] Read error, using defaults: {e}"),
        }
    }
    Config::default()
}

/// Save config to disk.
pub fn save(config: &Config, data_dir: &Path) -> Result<(), String> {
    let path = config_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("serialize failed: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("write failed: {e}"))?;
    Ok(())
}

/// Initialize the global config from disk. Call once at startup.
pub fn init(data_dir: &Path) {
    let config = load(data_dir);
    let _ = CONFIG.set(RwLock::new(config));
}

/// Get a clone of the current config.
pub fn get() -> Config {
    CONFIG
        .get()
        .expect("config not initialized")
        .read()
        .expect("config lock poisoned")
        .clone()
}

/// Update config in-place and save to disk.
pub fn update(data_dir: &Path, f: impl FnOnce(&mut Config)) -> Result<Config, String> {
    let lock = CONFIG.get().expect("config not initialized");
    let mut config = lock.write().map_err(|e| format!("config lock: {e}"))?;
    f(&mut config);
    save(&config, data_dir)?;
    let updated = config.clone();
    Ok(updated)
}
