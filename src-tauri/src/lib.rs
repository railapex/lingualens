use std::path::PathBuf;
use std::process::Command as StdCommand;
use std::sync::OnceLock;
use tauri::{Emitter, Manager};

pub mod config;
pub mod dict;
pub mod download;
pub mod history;
pub mod translate;
pub mod tts;

/// Resolved espeak-ng executable path (bundled resource or system fallback).
static ESPEAK_PATH: OnceLock<PathBuf> = OnceLock::new();
/// Resolved espeak-ng data directory.
static ESPEAK_DATA: OnceLock<PathBuf> = OnceLock::new();

/// Get the espeak-ng executable path (bundled or system).
pub fn espeak_exe() -> &'static PathBuf {
    ESPEAK_PATH.get().expect("espeak path not initialized")
}

/// Get the espeak-ng data directory.
pub fn espeak_data() -> &'static PathBuf {
    ESPEAK_DATA.get().expect("espeak data not initialized")
}

#[cfg(target_os = "windows")]
fn simulate_ctrl_c() {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;

    // Release all modifier keys first (user may still be holding the hotkey combo)
    let release_mods = [
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0x11), // VK_CONTROL
                    dwFlags: KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0x12), // VK_ALT
                    dwFlags: KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0x10), // VK_SHIFT
                    dwFlags: KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0x5B), // VK_LWIN
                    dwFlags: KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        },
    ];

    unsafe {
        SendInput(&release_mods, std::mem::size_of::<INPUT>() as i32);
    }

    // Small delay for modifier release to propagate
    std::thread::sleep(std::time::Duration::from_millis(30));

    // Now send clean Ctrl+C
    let copy = [
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0x11), // VK_CONTROL
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0x43), // VK_C
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0x43), // VK_C
                    dwFlags: KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0x11), // VK_CONTROL
                    dwFlags: KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        },
    ];

    unsafe {
        SendInput(&copy, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Try to get selected text via UI Automation TextPattern (clipboard-free).
/// Returns None if the focused element doesn't support TextPattern or has no selection.
#[cfg(target_os = "windows")]
fn get_selected_text_uia() -> Option<String> {
    use windows::core::Interface;
    use windows::Win32::System::Com::*;
    use windows::Win32::UI::Accessibility::*;

    unsafe {
        // COM init (idempotent — returns S_FALSE if already initialized on this thread)
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        // CUIAutomation8 required for TextPattern on Win32 Edit controls (Notepad etc.)
        let automation: IUIAutomation = CoCreateInstance(
            &CUIAutomation8,
            None,
            CLSCTX_INPROC_SERVER,
        )
        .ok()?;

        let focused = automation.GetFocusedElement().ok()?;

        let text_pattern: IUIAutomationTextPattern =
            focused.GetCurrentPattern(UIA_TextPatternId).ok()?.cast().ok()?;

        let ranges = text_pattern.GetSelection().ok()?;
        let count = ranges.Length().ok()?;
        if count == 0 {
            return None;
        }

        let mut result = String::new();
        for i in 0..count {
            if let Ok(range) = ranges.GetElement(i) {
                if let Ok(bstr) = range.GetText(-1) {
                    let text: String = bstr.to_string();
                    result.push_str(&text);
                }
            }
        }

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }
}

#[cfg(target_os = "windows")]
fn read_clipboard_text() -> Result<String, String> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, OpenClipboard,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_UNICODETEXT;

    unsafe {
        OpenClipboard(None).map_err(|e| format!("OpenClipboard: {}", e))?;

        let result = (|| -> Result<String, String> {
            let handle = GetClipboardData(CF_UNICODETEXT.0 as u32)
                .map_err(|e| format!("GetClipboardData: {}", e))?;

            let hmem = HGLOBAL(handle.0);
            let ptr = GlobalLock(hmem) as *const u16;
            if ptr.is_null() {
                return Err("GlobalLock returned null".into());
            }

            let mut len = 0;
            while *ptr.add(len) != 0 {
                len += 1;
            }

            let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
            let _ = GlobalUnlock(hmem);
            Ok(text)
        })();

        let _ = CloseClipboard();
        result
    }
}

/// Saved clipboard format data for full clipboard preservation.
#[cfg(target_os = "windows")]
struct SavedClipboardFormat {
    format: u32,
    data: Vec<u8>,
}

/// Save all clipboard formats (for full restore after capture).
/// Skips non-GlobalAlloc formats (bitmaps, metafiles, palettes).
#[cfg(target_os = "windows")]
fn save_clipboard_all() -> Vec<SavedClipboardFormat> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;

    let mut formats = Vec::new();

    unsafe {
        if OpenClipboard(None).is_err() {
            return formats;
        }

        let mut fmt = EnumClipboardFormats(0);
        while fmt != 0 {
            // Skip non-GlobalAlloc formats (GDI handles — can't save/restore raw bytes)
            // CF_BITMAP=2, CF_METAFILEPICT=3, CF_PALETTE=9, CF_ENHMETAFILE=14
            if !matches!(fmt, 2 | 3 | 9 | 14) {
                if let Ok(handle) = GetClipboardData(fmt) {
                    let hmem = HGLOBAL(handle.0);
                    let size = GlobalSize(hmem);
                    if size > 0 {
                        let ptr = GlobalLock(hmem) as *const u8;
                        if !ptr.is_null() {
                            let data = std::slice::from_raw_parts(ptr, size).to_vec();
                            formats.push(SavedClipboardFormat { format: fmt, data });
                            let _ = GlobalUnlock(hmem);
                        }
                    }
                }
            }
            fmt = EnumClipboardFormats(fmt);
        }

        let _ = CloseClipboard();
    }

    formats
}

/// Restore saved clipboard formats with exclusion markers for clipboard managers.
/// If `formats` is empty, just clears clipboard with exclusion formats.
#[cfg(target_os = "windows")]
fn restore_clipboard_all_excluded(formats: &[SavedClipboardFormat]) -> Result<(), String> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;

    unsafe {
        OpenClipboard(None).map_err(|e| format!("OpenClipboard: {e}"))?;

        let result = (|| -> Result<(), String> {
            EmptyClipboard().map_err(|e| format!("EmptyClipboard: {e}"))?;

            for saved in formats {
                if let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, saved.data.len()) {
                    let ptr = GlobalLock(HGLOBAL(hmem.0)) as *mut u8;
                    if !ptr.is_null() {
                        std::ptr::copy_nonoverlapping(
                            saved.data.as_ptr(),
                            ptr,
                            saved.data.len(),
                        );
                        let _ = GlobalUnlock(HGLOBAL(hmem.0));
                        let _ = SetClipboardData(
                            saved.format,
                            Some(windows::Win32::Foundation::HANDLE(hmem.0)),
                        );
                    }
                }
            }

            set_clipboard_exclusion_formats();
            Ok(())
        })();

        let _ = CloseClipboard();
        result
    }
}

/// Set clipboard formats that tell clipboard managers to ignore this write.
/// Must be called while clipboard is open (between OpenClipboard/CloseClipboard).
#[cfg(target_os = "windows")]
unsafe fn set_clipboard_exclusion_formats() {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;
    use windows::core::w;

    // Format 1: ExcludeClipboardContentFromMonitorProcessing (empty data)
    // Windows 10+ clipboard history (Win+V) respects this
    let fmt1 = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
    if fmt1 != 0 {
        if let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, 1) {
            let _ = SetClipboardData(fmt1, Some(windows::Win32::Foundation::HANDLE(hmem.0)));
        }
    }

    // Format 2: CanIncludeInClipboardHistory (DWORD = 0)
    let fmt2 = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
    if fmt2 != 0 {
        if let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, 4) {
            let ptr = GlobalLock(HGLOBAL(hmem.0)) as *mut u32;
            if !ptr.is_null() {
                *ptr = 0;
                let _ = GlobalUnlock(HGLOBAL(hmem.0));
            }
            let _ = SetClipboardData(fmt2, Some(windows::Win32::Foundation::HANDLE(hmem.0)));
        }
    }

    // Format 3: Clipboard Viewer Ignore (legacy — Ditto and other third-party managers)
    let fmt3 = RegisterClipboardFormatW(w!("Clipboard Viewer Ignore"));
    if fmt3 != 0 {
        if let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, 1) {
            let _ = SetClipboardData(fmt3, Some(windows::Win32::Foundation::HANDLE(hmem.0)));
        }
    }
}

/// Clear clipboard contents (used before Ctrl+C to detect fresh selection).
#[cfg(target_os = "windows")]
fn clear_clipboard() -> Result<(), String> {
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard,
    };

    unsafe {
        OpenClipboard(None).map_err(|e| format!("OpenClipboard: {e}"))?;
        let result = EmptyClipboard().map_err(|e| format!("EmptyClipboard: {e}"));
        let _ = CloseClipboard();
        result
    }
}

/// Poll clipboard sequence number until it changes or timeout.
/// Returns true if clipboard changed, false on timeout.
#[cfg(target_os = "windows")]
fn poll_clipboard_change(seq_before: u32, timeout_ms: u64) -> bool {
    use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;

    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(timeout_ms);

    loop {
        if unsafe { GetClipboardSequenceNumber() } != seq_before {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Two-tier text capture: UIA TextPattern first, clipboard simulation fallback.
#[tauri::command]
fn get_selected_text() -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        let t0 = std::time::Instant::now();

        // Tier 1: UI Automation (clipboard-free, ~80% of apps) — skip if force_clipboard
        if !config::get().force_clipboard {
        if let Some(text) = get_selected_text_uia() {
            let trimmed = text.trim().to_string();
            if !trimmed.is_empty() {
                log::info!(
                    "[capture] UIA success ({} chars, {:.1}ms)",
                    trimmed.len(),
                    t0.elapsed().as_secs_f64() * 1000.0
                );
                return Ok(trimmed);
            }
        }
        } // end force_clipboard check

        // Tier 2: Clipboard simulation with sequence-number polling
        log::info!(
            "[capture] UIA miss ({:.1}ms), falling back to clipboard",
            t0.elapsed().as_secs_f64() * 1000.0
        );

        let saved = save_clipboard_all();
        let _ = clear_clipboard();

        let seq_before = unsafe {
            windows::Win32::System::DataExchange::GetClipboardSequenceNumber()
        };

        simulate_ctrl_c();

        // Poll for clipboard change instead of fixed sleep — fast apps resolve in ~10ms,
        // slow apps (Electron) get up to 500ms
        let changed = poll_clipboard_change(seq_before, 500);

        let captured = if changed {
            read_clipboard_text().unwrap_or_default()
        } else {
            String::new()
        };

        // Restore all original formats with exclusion markers
        if !saved.is_empty() || !captured.is_empty() {
            let _ = restore_clipboard_all_excluded(&saved);
        }

        log::info!(
            "[capture] clipboard {} ({} chars, {:.1}ms, {} formats saved)",
            if changed { "success" } else { "timeout" },
            captured.len(),
            t0.elapsed().as_secs_f64() * 1000.0,
            saved.len(),
        );

        Ok(captured)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Not implemented on this platform".into())
    }
}

/// Get the center coordinates of the monitor containing the foreground window.
/// Returns [x, y, monitorWidth, monitorHeight].
#[cfg(target_os = "windows")]
#[tauri::command]
fn get_active_monitor_center() -> Result<[i32; 4], String> {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    unsafe {
        let hwnd = GetForegroundWindow();
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return Err("GetMonitorInfo failed".into());
        }

        let rc = info.rcMonitor;
        let cx = (rc.left + rc.right) / 2;
        let cy = (rc.top + rc.bottom) / 2;
        let w = rc.right - rc.left;
        let h = rc.bottom - rc.top;
        Ok([cx, cy, w, h])
    }
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
fn get_active_monitor_center() -> Result<[i32; 4], String> {
    Err("Not implemented on this platform".into())
}

#[tauri::command]
fn get_ipa(text: String, lang: String) -> Result<String, String> {
    let output = StdCommand::new(espeak_exe())
        .env("ESPEAK_DATA_PATH", espeak_data())
        .args(&["-v", &lang, "--ipa", "-q", &text])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let ipa = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Ok(ipa)
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).to_string();
            log::warn!("[ipa] espeak-ng error: {err}");
            Ok(String::new()) // graceful degradation — no IPA rather than error
        }
        Err(e) => {
            log::warn!("[ipa] espeak-ng not available: {e}");
            Ok(String::new()) // graceful degradation
        }
    }
}

#[tauri::command]
fn detect_language(text: String, app_handle: tauri::AppHandle) -> String {
    let data_dir = app_handle.path().app_data_dir().unwrap_or_default();
    let cfg = config::get();
    detect_lang(&text, &cfg.target_lang, &cfg.native_lang, &data_dir)
}

#[tauri::command]
fn get_config() -> config::Config {
    config::get()
}

#[tauri::command]
fn is_first_run(app_handle: tauri::AppHandle) -> bool {
    let data_dir = app_handle.path().app_data_dir().unwrap_or_default();
    config::is_first_run(&data_dir)
}

#[tauri::command]
fn set_config(
    updates: serde_json::Value,
    app_handle: tauri::AppHandle,
) -> Result<config::Config, String> {
    let data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;

    let updated = config::update(&data_dir, |cfg| {
        if let Some(v) = updates.get("target_lang").and_then(|v| v.as_str()) {
            cfg.target_lang = v.to_string();
        }
        if let Some(v) = updates.get("native_lang").and_then(|v| v.as_str()) {
            cfg.native_lang = v.to_string();
        }
        if let Some(v) = updates.get("theme").and_then(|v| v.as_str()) {
            cfg.theme = v.to_string();
        }
        if let Some(v) = updates.get("auto_play").and_then(|v| v.as_bool()) {
            cfg.auto_play = v;
        }
        if let Some(v) = updates.get("show_ipa").and_then(|v| v.as_bool()) {
            cfg.show_ipa = v;
        }
        if let Some(v) = updates.get("dismiss_delay_ms").and_then(|v| v.as_u64()) {
            cfg.dismiss_delay_ms = v as u32;
        }
        if let Some(v) = updates.get("replay_speed").and_then(|v| v.as_f64()) {
            cfg.replay_speed = v as f32;
        }
        if let Some(v) = updates.get("tts_voice_target") {
            cfg.tts_voice_target = v.as_str().map(|s| s.to_string());
        }
        if let Some(v) = updates.get("tts_voice_native") {
            cfg.tts_voice_native = v.as_str().map(|s| s.to_string());
        }
        if let Some(v) = updates.get("hotkey").and_then(|v| v.as_str()) {
            cfg.hotkey = v.to_string();
        }
        if let Some(v) = updates.get("force_cpu").and_then(|v| v.as_bool()) {
            cfg.force_cpu = v;
        }
        if let Some(v) = updates.get("force_web_speech").and_then(|v| v.as_bool()) {
            cfg.force_web_speech = v;
        }
        if let Some(v) = updates.get("force_dict_only").and_then(|v| v.as_bool()) {
            cfg.force_dict_only = v;
        }
        if let Some(v) = updates.get("force_clipboard").and_then(|v| v.as_bool()) {
            cfg.force_clipboard = v;
        }
    })?;

    let _ = app_handle.emit("config-changed", &updated);
    Ok(updated)
}

#[tauri::command]
fn speak(
    text: String,
    lang: String,
    voice: Option<String>,
    speed: Option<f32>,
    app_handle: tauri::AppHandle,
) -> Result<Vec<u8>, String> {
    let data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    tts::speak(&text, &lang, voice.as_deref(), speed, &data_dir)
}

#[tauri::command]
fn list_voices(app_handle: tauri::AppHandle) -> Result<Vec<tts::VoiceInfo>, String> {
    let data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    Ok(tts::list_voices(&data_dir))
}

#[tauri::command]
fn get_tts_status(app_handle: tauri::AppHandle) -> tts::TtsStatus {
    let data_dir = app_handle
        .path()
        .app_data_dir()
        .unwrap_or_default();
    tts::status(&data_dir)
}

/// Character-based language signal: does the text contain characters specific to the given language?
fn has_lang_chars(text: &str, lang: &str) -> bool {
    match lang {
        "es" => text.chars().any(|c| matches!(c,
            'á'|'é'|'í'|'ó'|'ú'|'ñ'|'¿'|'¡'|'ü'
            |'Á'|'É'|'Í'|'Ó'|'Ú'|'Ñ'|'Ü')),
        "fr" => text.chars().any(|c| matches!(c,
            'é'|'É'|'è'|'È'|'ê'|'Ê'|'ë'|'Ë'
            |'à'|'À'|'â'|'Â'|'î'|'Î'|'ô'|'Ô'|'û'|'Û'|'ù'|'Ù'
            |'ç'|'Ç'|'œ'|'Œ'|'æ'|'Æ'|'ÿ'|'Ÿ'|'ï'|'Ï')),
        "de" => text.chars().any(|c| matches!(c,
            'ä'|'Ä'|'ö'|'Ö'|'ü'|'Ü'|'ß')),
        "pt" => text.chars().any(|c| matches!(c,
            'ã'|'Ã'|'õ'|'Õ'|'ç'|'Ç')),
        "it" => text.chars().any(|c| matches!(c,
            'à'|'À'|'è'|'È'|'ì'|'Ì'|'ò'|'Ò'|'ù'|'Ù')),
        "ja" => text.chars().any(|c| ('\u{3040}'..='\u{309F}').contains(&c)  // Hiragana
            || ('\u{30A0}'..='\u{30FF}').contains(&c)                        // Katakana
            || ('\u{4E00}'..='\u{9FFF}').contains(&c)),                      // CJK
        "zh" => text.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c)),
        _ => false,
    }
}

/// Map whatlang::Lang to our 2-letter code.
fn whatlang_to_code(lang: whatlang::Lang) -> Option<&'static str> {
    match lang {
        whatlang::Lang::Spa => Some("es"),
        whatlang::Lang::Eng => Some("en"),
        whatlang::Lang::Fra => Some("fr"),
        whatlang::Lang::Deu => Some("de"),
        whatlang::Lang::Ita => Some("it"),
        whatlang::Lang::Por => Some("pt"),
        whatlang::Lang::Jpn => Some("ja"),
        whatlang::Lang::Cmn => Some("zh"),
        _ => None,
    }
}

fn detect_lang(text: &str, target_lang: &str, native_lang: &str, data_dir: &std::path::Path) -> String {
    // 1. Target-language-specific characters are instant signal
    if has_lang_chars(text, target_lang) {
        return target_lang.into();
    }

    // 2. Dictionary-based detection (es<>en fast path)
    if target_lang == "es" || native_lang == "es" {
        let confidence = dict::spanish_confidence(text, data_dir);
        if confidence >= 0.5 {
            return "es".into();
        }
    }

    // 3. Whatlang — binary: is it target, or native?
    use whatlang::detect;
    if let Some(info) = detect(text) {
        if let Some(code) = whatlang_to_code(info.lang()) {
            if code == target_lang {
                return target_lang.into();
            }
        }
    }

    // 4. Unknown/ambiguous defaults to native (will translate to target)
    native_lang.into()
}

#[tauri::command]
fn translate_text(
    text: String,
    source_lang: String,
    target_lang: String,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    let data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;

    // Dev switch: dictionary-only mode (skip TranslateGemma)
    if config::get().force_dict_only {
        if let Some(result) = dict::try_translate(&text, &source_lang, &data_dir) {
            log::info!("[translate] dict hit (force_dict_only): '{}' → '{}'", text, result);
            let _ = history::insert(&text, &source_lang, &result, &target_lang, "dict");
            return Ok(result);
        }
        return Ok(String::new()); // no model available in dict-only mode
    }

    // Fast path: dictionary lookup for short Spanish text
    if let Some(result) = dict::try_translate(&text, &source_lang, &data_dir) {
        log::info!("[translate] dict hit: '{}' → '{}'", text, result);
        let _ = history::insert(&text, &source_lang, &result, &target_lang, "dict");
        return Ok(result);
    }

    // Slow path: TranslateGemma model
    let result = translate::translate(&text, &source_lang, &target_lang, &data_dir)?;
    let _ = history::insert(&text, &source_lang, &result, &target_lang, "model");
    Ok(result)
}

fn open_or_focus_window(app: &tauri::AppHandle, label: &str, title: &str, url: &str, width: f64, height: f64) {
    use tauri::WebviewWindowBuilder;
    if let Some(win) = app.get_webview_window(label) {
        let _ = win.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(url.into()))
        .title(title)
        .inner_size(width, height)
        .resizable(true)
        .center()
        .build();
}

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    use tauri::tray::TrayIconBuilder;

    let settings = MenuItemBuilder::new("Settings...").id("open_settings").build(app)?;
    let debug = MenuItemBuilder::new("Debug Tools").id("open_debug").build(app)?;
    let quit = MenuItemBuilder::new("Quit").id("quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&settings)
        .item(&debug)
        .separator()
        .item(&quit)
        .build()?;

    let _tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("LinguaLens")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| {
            let id = event.id().as_ref();

            if id == "open_settings" {
                open_or_focus_window(app, "settings", "LinguaLens Settings", "settings.html", 560.0, 520.0);
            } else if id == "open_debug" {
                open_or_focus_window(app, "test", "LinguaLens Debug", "test.html", 820.0, 700.0);
            } else if id == "quit" {
                std::process::exit(0);
            }
        })
        .build(app)?;

    Ok(())
}

fn register_hotkey(app_handle: &tauri::AppHandle, shortcut: &str) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

    let handle = app_handle.clone();
    app_handle.global_shortcut().on_shortcut(shortcut, move |_app, _shortcut, event| {
        if event.state() == ShortcutState::Pressed {
            log::info!("[hotkey] pressed");
            let text = get_selected_text().unwrap_or_default();
            let text = text.trim().to_string();
            log::info!("[hotkey] captured text: {:?}", &text[..text.len().min(50)]);

            let monitor = get_active_monitor_center().ok();

            let h = handle.clone();
            std::thread::spawn(move || {
                if let Some(window) = h.get_webview_window("main") {
                    if let Some([cx, cy, _, _]) = monitor {
                        if let Ok(size) = window.inner_size() {
                            let x = cx - (size.width as i32 / 2);
                            let y = cy - (size.height as i32 / 2);
                            let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
                        }
                    }
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                if let Err(e) = h.emit("hotkey-text", text) {
                    log::error!("[hotkey] Failed to emit: {e}");
                }
            });
        }
    }).map_err(|e| format!("Failed to register shortcut: {e}"))
}

#[tauri::command]
fn unregister_hotkey(app_handle: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let current = config::get().hotkey;
    app_handle.global_shortcut().unregister(current.as_str())
        .map_err(|e| format!("Unregister failed: {e}"))
}

#[tauri::command]
fn update_hotkey(new_hotkey: String, app_handle: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    // Unregister old shortcut first (may already be unregistered from capture mode — ignore errors)
    let old = config::get().hotkey;
    let _ = app_handle.global_shortcut().unregister(old.as_str());
    register_hotkey(&app_handle, &new_hotkey)?;
    let data_dir = app_handle.path().app_data_dir().map_err(|e| e.to_string())?;
    let _ = config::update(&data_dir, |cfg| { cfg.hotkey = new_hotkey; });
    Ok(())
}

#[tauri::command]
fn restore_hotkey(app_handle: tauri::AppHandle) -> Result<(), String> {
    let current = config::get().hotkey;
    register_hotkey(&app_handle, &current)
}

#[tauri::command]
fn get_history(
    limit: Option<u32>,
    offset: Option<u32>,
    search: Option<String>,
) -> Result<Vec<history::HistoryEntry>, String> {
    history::query_recent(
        limit.unwrap_or(50),
        offset.unwrap_or(0),
        search.as_deref(),
    )
}

#[tauri::command]
fn get_history_count(search: Option<String>) -> Result<u32, String> {
    history::count(search.as_deref())
}

#[tauri::command]
fn check_models(app_handle: tauri::AppHandle) -> Vec<download::MissingModel> {
    let data_dir = app_handle.path().app_data_dir().unwrap_or_default();
    download::check_models(&data_dir)
}

#[tauri::command]
async fn start_download(app_handle: tauri::AppHandle) -> Result<(), String> {
    let data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    download::download_models(data_dir, app_handle).await
}

#[tauri::command]
fn cancel_download() {
    download::cancel();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .setup(|app| {
            // Initialize config before anything else
            let data_dir = app.path().app_data_dir().unwrap_or_default();
            config::init(&data_dir);

            // Resolve espeak-ng path: bundled resource → system fallback
            {
                let resource_dir = app.path().resource_dir().unwrap_or_default();
                let bundled_exe = resource_dir.join("resources/espeak-ng/espeak-ng.exe");
                let bundled_data = resource_dir.join("resources/espeak-ng/espeak-ng-data");
                let system_exe = PathBuf::from("C:/Program Files/eSpeak NG/espeak-ng.exe");
                let system_data = PathBuf::from("C:/Program Files/eSpeak NG/espeak-ng-data");

                if bundled_exe.exists() {
                    log::info!("[espeak] Using bundled: {}", bundled_exe.display());
                    let _ = ESPEAK_PATH.set(bundled_exe);
                    let _ = ESPEAK_DATA.set(bundled_data);
                } else if system_exe.exists() {
                    log::info!("[espeak] Using system install");
                    let _ = ESPEAK_PATH.set(system_exe);
                    let _ = ESPEAK_DATA.set(system_data);
                } else {
                    log::warn!("[espeak] Not found — IPA and Kokoro TTS will be unavailable");
                    let _ = ESPEAK_PATH.set(bundled_exe); // will fail gracefully on use
                    let _ = ESPEAK_DATA.set(bundled_data);
                }
            }

            // Initialize history DB
            if let Err(e) = history::init(&data_dir) {
                log::error!("[history] Init failed: {e}");
            }

            // System tray
            setup_tray(app)?;

            // Preload models: dict + TTS in parallel (dict=CPU, TTS=CUDA),
            // then TranslateGemma after TTS finishes (both want CUDA).
            {
                let data_dir = data_dir.clone();
                std::thread::spawn(move || {
                    let dd = data_dir.clone();
                    let dict_thread = std::thread::spawn(move || dict::preload(&dd));
                    tts::preload(&data_dir);
                    let _ = dict_thread.join();
                    translate::preload(&data_dir);
                });
            }

            // Register hotkey from config
            let cfg = config::get();
            register_hotkey(app.handle(), &cfg.hotkey)?;
            log::info!("[hotkey] {} registered from Rust", cfg.hotkey);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_ipa,
            get_selected_text,
            get_active_monitor_center,
            detect_language,
            translate_text,
            speak,
            list_voices,
            get_tts_status,
            get_config,
            set_config,
            is_first_run,
            unregister_hotkey,
            update_hotkey,
            restore_hotkey,
            get_history,
            get_history_count,
            check_models,
            start_download,
            cancel_download,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
