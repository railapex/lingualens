// Standalone CLI for Kokoro TTS — used by Vite middleware for browser testing
// Usage: tts_cli <text> <lang> [voice]
// Outputs WAV bytes to stdout

use std::io::Write;
use std::path::PathBuf;

// Re-use the library's tts module
use app_lib::tts;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: tts_cli <text> <lang> [voice]");
        std::process::exit(1);
    }

    let text = &args[1];
    let lang = &args[2];
    let voice = args.get(3).map(|s| s.as_str());

    let data_dir = if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| "C:/Users/chris/AppData/Roaming".into()))
            .join("com.lingualens.app")
    } else {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
            .join("Library/Application Support/com.lingualens.app")
    };

    match tts::speak(text, lang, voice, None, &data_dir) {
        Ok(wav) => {
            std::io::stdout().write_all(&wav).unwrap();
        }
        Err(e) => {
            eprintln!("TTS error: {e}");
            std::process::exit(1);
        }
    }
}
