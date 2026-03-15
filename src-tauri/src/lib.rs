use std::path::PathBuf;
use std::process::Command as StdCommand;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
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
/// Resolved dictionary directory (bundled resource or app data fallback).
static DICT_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Get the espeak-ng executable path (bundled or system).
pub fn espeak_exe() -> &'static PathBuf {
    ESPEAK_PATH.get().expect("espeak path not initialized")
}

/// Get the espeak-ng data directory.
pub fn espeak_data() -> &'static PathBuf {
    ESPEAK_DATA.get().expect("espeak data not initialized")
}

/// Get the resolved dictionary directory.
pub fn dict_dir() -> &'static PathBuf {
    DICT_DIR.get().expect("dict dir not initialized")
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

// --- macOS text capture ---

#[cfg(target_os = "macos")]
type AXUIElementRef = *const std::ffi::c_void;
#[cfg(target_os = "macos")]
type AXValueRef = *const std::ffi::c_void;

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AXCFRange {
    location: isize,
    length: isize,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AXCGPoint {
    x: f64,
    y: f64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AXCGSize {
    width: f64,
    height: f64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AXCGRect {
    origin: AXCGPoint,
    size: AXCGSize,
}

#[cfg(target_os = "macos")]
const K_AX_VALUE_CGRECT_TYPE: i32 = 3;
#[cfg(target_os = "macos")]
const K_AX_VALUE_CFRANGE_TYPE: i32 = 4;

/// Return the currently focused UI element via Accessibility API.
/// Caller owns the returned reference and must CFRelease it.
#[cfg(target_os = "macos")]
fn copy_focused_ui_element() -> Option<AXUIElementRef> {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;

    unsafe {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return None;
        }

        let focused_app_attr = CFString::new("AXFocusedApplication");
        let mut focused_app: core_foundation::base::CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            system,
            focused_app_attr.as_concrete_TypeRef(),
            &mut focused_app,
        );
        CFRelease(system as _);
        if err != 0 || focused_app.is_null() {
            return None;
        }

        let focused_elem_attr = CFString::new("AXFocusedUIElement");
        let mut focused_elem: core_foundation::base::CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            focused_app as AXUIElementRef,
            focused_elem_attr.as_concrete_TypeRef(),
            &mut focused_elem,
        );
        CFRelease(focused_app);
        if err != 0 || focused_elem.is_null() {
            return None;
        }

        Some(focused_elem as AXUIElementRef)
    }
}

/// Get selected text via macOS Accessibility API (AXUIElement).
/// Requires Accessibility permission in System Settings > Privacy & Security.
#[cfg(target_os = "macos")]
fn get_selected_text_accessibility() -> Option<String> {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;

    unsafe {
        let focused_elem = copy_focused_ui_element()?;

        let selected_text_attr = CFString::new("AXSelectedText");
        let mut selected_text: core_foundation::base::CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            focused_elem,
            selected_text_attr.as_concrete_TypeRef(),
            &mut selected_text,
        );
        CFRelease(focused_elem as _);
        if err != 0 || selected_text.is_null() {
            return None;
        }

        let cf_str: CFString = CFString::wrap_under_create_rule(selected_text as _);
        let result = cf_str.to_string();

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }
}

#[cfg(target_os = "macos")]
fn get_selected_text_bounds_accessibility() -> Option<[i32; 4]> {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;

    unsafe {
        let focused_elem = copy_focused_ui_element()?;

        let selected_range_attr = CFString::new("AXSelectedTextRange");
        let mut selected_range: core_foundation::base::CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            focused_elem,
            selected_range_attr.as_concrete_TypeRef(),
            &mut selected_range,
        );
        if err != 0 || selected_range.is_null() {
            CFRelease(focused_elem as _);
            return None;
        }

        let mut range = AXCFRange::default();
        let ok = AXValueGetValue(
            selected_range as AXValueRef,
            K_AX_VALUE_CFRANGE_TYPE,
            &mut range as *mut _ as *mut std::ffi::c_void,
        );
        if !ok {
            CFRelease(selected_range);
            CFRelease(focused_elem as _);
            return None;
        }

        // Zero-length range means caret-only; use a 1-char range to ask for bounds.
        if range.length <= 0 {
            range.length = 1;
        }

        let range_value =
            AXValueCreate(K_AX_VALUE_CFRANGE_TYPE, &range as *const _ as *const std::ffi::c_void);
        if range_value.is_null() {
            CFRelease(selected_range);
            CFRelease(focused_elem as _);
            return None;
        }

        let bounds_attr = CFString::new("AXBoundsForRange");
        let mut bounds_ref: core_foundation::base::CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyParameterizedAttributeValue(
            focused_elem,
            bounds_attr.as_concrete_TypeRef(),
            range_value as _,
            &mut bounds_ref,
        );

        CFRelease(range_value as _);
        CFRelease(selected_range);
        CFRelease(focused_elem as _);

        if err != 0 || bounds_ref.is_null() {
            return None;
        }

        let mut rect = AXCGRect::default();
        let ok = AXValueGetValue(
            bounds_ref as AXValueRef,
            K_AX_VALUE_CGRECT_TYPE,
            &mut rect as *mut _ as *mut std::ffi::c_void,
        );
        CFRelease(bounds_ref);

        if !ok || rect.size.width <= 0.0 || rect.size.height <= 0.0 {
            return None;
        }

        Some([
            rect.origin.x.round() as i32,
            rect.origin.y.round() as i32,
            rect.size.width.round().max(1.0) as i32,
            rect.size.height.round().max(1.0) as i32,
        ])
    }
}

#[cfg(target_os = "macos")]
extern "C" {
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: core_foundation::string::CFStringRef,
        value: *mut core_foundation::base::CFTypeRef,
    ) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: AXUIElementRef,
        attribute: core_foundation::string::CFStringRef,
        parameter: core_foundation::base::CFTypeRef,
        value: *mut core_foundation::base::CFTypeRef,
    ) -> i32;
    fn AXValueCreate(the_type: i32, value_ptr: *const std::ffi::c_void) -> AXValueRef;
    fn AXValueGetValue(
        value: AXValueRef,
        the_type: i32,
        value_ptr: *mut std::ffi::c_void,
    ) -> bool;
    fn CFRelease(cf: core_foundation::base::CFTypeRef);
}

/// Simulate Cmd+C on macOS via CGEvent keyboard events.
#[cfg(target_os = "macos")]
fn simulate_cmd_c() {
    use core_graphics::event::{CGEvent, CGEventFlags, CGKeyCode};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState);
    let source = match source {
        Ok(s) => s,
        Err(_) => return,
    };

    // Key code for 'C' is 8 on macOS
    const KC_C: CGKeyCode = 8;

    // Release all modifier keys first (user may still be holding the hotkey combo)
    for keycode in [55u16, 56, 58, 59, 54, 60, 61, 62] {
        // LCmd, LShift, LAlt, LCtrl, RCmd, RShift, RAlt, RCtrl
        if let Ok(ev) = CGEvent::new_keyboard_event(source.clone(), keycode, false) {
            ev.post(core_graphics::event::CGEventTapLocation::HID);
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(30));

    // Cmd down + C down
    if let Ok(ev) = CGEvent::new_keyboard_event(source.clone(), KC_C, true) {
        ev.set_flags(CGEventFlags::CGEventFlagCommand);
        ev.post(core_graphics::event::CGEventTapLocation::HID);
    }
    // Cmd up + C up
    if let Ok(ev) = CGEvent::new_keyboard_event(source.clone(), KC_C, false) {
        ev.set_flags(CGEventFlags::CGEventFlagCommand);
        ev.post(core_graphics::event::CGEventTapLocation::HID);
    }
}

/// Read text from macOS pasteboard.
#[cfg(target_os = "macos")]
fn read_pasteboard_text() -> Option<String> {
    use std::process::Command;
    // Use pbpaste for simplicity — avoids raw Objective-C FFI for NSPasteboard
    let output = Command::new("pbpaste").output().ok()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        if text.is_empty() { None } else { Some(text) }
    } else {
        None
    }
}

/// Save current pasteboard text content (for restore after capture).
#[cfg(target_os = "macos")]
fn save_pasteboard_text() -> Option<String> {
    read_pasteboard_text()
}

/// Clear and restore pasteboard text.
#[cfg(target_os = "macos")]
fn restore_pasteboard_text(saved: Option<String>) {
    use std::process::Command;
    match saved {
        Some(text) => {
            // Pipe saved text back into pbcopy
            use std::io::Write;
            if let Ok(mut child) = Command::new("pbcopy")
                .stdin(std::process::Stdio::piped())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
            }
        }
        None => {
            // Clear clipboard
            let _ = Command::new("pbcopy")
                .stdin(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut c| {
                    drop(c.stdin.take());
                    c.wait()
                });
        }
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
        }

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

    #[cfg(target_os = "macos")]
    {
        let t0 = std::time::Instant::now();

        // Tier 1: Accessibility API (clipboard-free) — skip if force_clipboard
        if !config::get().force_clipboard {
            if let Some(text) = get_selected_text_accessibility() {
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() {
                    log::info!(
                        "[capture] Accessibility success ({} chars, {:.1}ms)",
                        trimmed.len(),
                        t0.elapsed().as_secs_f64() * 1000.0
                    );
                    return Ok(trimmed);
                }
            }
        }

        // Tier 2: Clipboard simulation with Cmd+C
        log::info!(
            "[capture] Accessibility miss ({:.1}ms), falling back to clipboard",
            t0.elapsed().as_secs_f64() * 1000.0
        );

        let saved = save_pasteboard_text();

        // Clear pasteboard then simulate Cmd+C
        restore_pasteboard_text(None);
        simulate_cmd_c();

        // Give the target app time to respond to Cmd+C
        std::thread::sleep(std::time::Duration::from_millis(100));

        let captured = read_pasteboard_text().unwrap_or_default();

        // Restore original pasteboard contents
        restore_pasteboard_text(saved);

        log::info!(
            "[capture] clipboard ({} chars, {:.1}ms)",
            captured.len(),
            t0.elapsed().as_secs_f64() * 1000.0,
        );

        Ok(captured)
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
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

/// Get the center of the primary monitor on macOS.
/// Uses the main screen (where the frontmost app is) via NSScreen API.
#[cfg(target_os = "macos")]
#[tauri::command]
fn get_active_monitor_center() -> Result<[i32; 4], String> {
    // Primary display via CGDisplay — always the screen with the menu bar.
    // Multi-monitor: this picks the primary, not necessarily the one with focused app.
    // Good enough for single-monitor; revisit with NSScreen.mainScreen if needed.
    unsafe {
        let display = CGMainDisplayID();
        let w = CGDisplayPixelsWide(display) as i32;
        let h = CGDisplayPixelsHigh(display) as i32;
        Ok([w / 2, h / 2, w, h])
    }
}

#[cfg(target_os = "macos")]
extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayPixelsWide(display: u32) -> usize;
    fn CGDisplayPixelsHigh(display: u32) -> usize;
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[tauri::command]
fn get_active_monitor_center() -> Result<[i32; 4], String> {
    Err("Not implemented on this platform".into())
}

#[tauri::command]
fn get_ipa(text: String, lang: String) -> Result<String, String> {
    let mut cmd = StdCommand::new(espeak_exe());
    cmd.env("ESPEAK_DATA_PATH", espeak_data())
       .args(&["-v", &lang, "--ipa", "-q", &text]);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let output = cmd.output();

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
        if let Some(v) = updates.get("overlay_position_mode").and_then(|v| v.as_str()) {
            cfg.overlay_position_mode = if v.eq_ignore_ascii_case("center") {
                "center".to_string()
            } else {
                "cursor".to_string()
            };
        }
        if let Some(v) = updates.get("start_at_login").and_then(|v| v.as_bool()) {
            cfg.start_at_login = v;
        }
    })?;

    let _ = app_handle.emit("config-changed", &updated);
    Ok(updated)
}

// --- Cross-platform audio playback via rodio ---
// OutputStream is !Send+!Sync (cpal platform stream), so we run playback
// on a dedicated thread and communicate via channel.

mod audio {
    use std::io::Cursor;
    use std::sync::{mpsc, Mutex, OnceLock};

    enum AudioMsg {
        Play(Vec<u8>),
        Stop,
    }

    static SENDER: OnceLock<Mutex<mpsc::Sender<AudioMsg>>> = OnceLock::new();

    fn get_sender() -> &'static Mutex<mpsc::Sender<AudioMsg>> {
        SENDER.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<AudioMsg>();
            std::thread::spawn(move || {
                use rodio::{Decoder, OutputStream, Sink};

                let (stream, handle) = match OutputStream::try_default() {
                    Ok(pair) => pair,
                    Err(e) => {
                        log::error!("[audio] Failed to open output device: {e}");
                        return;
                    }
                };
                let sink = Sink::try_new(&handle).unwrap();

                for msg in rx {
                    match msg {
                        AudioMsg::Play(wav_bytes) => {
                            sink.stop();
                            match Decoder::new(Cursor::new(wav_bytes)) {
                                Ok(source) => sink.append(source),
                                Err(e) => log::warn!("[audio] Failed to decode WAV: {e}"),
                            }
                        }
                        AudioMsg::Stop => {
                            sink.stop();
                        }
                    }
                }

                // Keep stream alive until channel closes
                drop(stream);
            });
            Mutex::new(tx)
        })
    }

    pub fn play(wav_bytes: Vec<u8>) {
        if let Ok(tx) = get_sender().lock() {
            let _ = tx.send(AudioMsg::Play(wav_bytes));
        }
    }

    pub fn stop() {
        if let Some(sender) = SENDER.get() {
            if let Ok(tx) = sender.lock() {
                let _ = tx.send(AudioMsg::Stop);
            }
        }
    }
}

#[tauri::command]
fn stop_tts() {
    audio::stop();
}

#[tauri::command]
async fn speak(
    text: String,
    lang: String,
    voice: Option<String>,
    speed: Option<f32>,
    app_handle: tauri::AppHandle,
) -> Result<f32, String> {
    let data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;

    // Run inference on a blocking thread (don't tie up the async runtime)
    let wav_bytes = tokio::task::spawn_blocking(move || {
        tts::speak(&text, &lang, voice.as_deref(), speed, &data_dir)
    })
    .await
    .map_err(|e| format!("TTS task failed: {e}"))??;

    // Calculate duration: WAV = 44-byte header + 16-bit mono PCM at 24kHz
    let data_bytes = wav_bytes.len().saturating_sub(44);
    let duration = (data_bytes / 2) as f32 / 24000.0;

    log::info!("[tts] Playing {:.1}s audio ({} bytes)", duration, wav_bytes.len());

    // Play via OS audio API — no binary data over IPC
    audio::play(wav_bytes);

    Ok(duration)
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
async fn translate_text(
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

    // Model inference on blocking thread — don't tie up async runtime
    let t = text.clone();
    let sl = source_lang.clone();
    let tl = target_lang.clone();
    let result = tokio::task::spawn_blocking(move || {
        translate::translate(&t, &sl, &tl, &data_dir)
    })
    .await
    .map_err(|e| format!("Translation task failed: {e}"))??;

    let _ = history::insert(&text, &source_lang, &result, &target_lang, "model");
    Ok(result)
}

/// Capitalize first letter of each hotkey part for display (e.g. "ctrl+alt+l" → "Ctrl+Alt+L").
fn format_hotkey_display(hotkey: &str) -> String {
    hotkey.split('+')
        .map(|s| {
            let mut c = s.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(target_os = "macos")]
fn compute_overlay_window_position_for_selection(
    selection_bounds: [i32; 4],
    monitor: Option<[i32; 4]>,
    window_size: (i32, i32),
) -> Option<(i32, i32)> {
    let [sel_x, sel_y, sel_w, sel_h] = selection_bounds;
    let (win_w, win_h) = window_size;

    // Overlay card is top-anchored in CSS with a small inset.
    const VERTICAL_GAP: i32 = 10;
    const OVERLAY_TOP_INSET: i32 = 12;

    // Horizontal placement is stable regardless of coordinate origin assumptions.
    let mut x = sel_x + (sel_w / 2) - (win_w / 2);

    let ([_cx, _cy, mon_w, mon_h], mon_left, mon_top) = if let Some(m) = monitor {
        let left = m[0] - (m[2] / 2);
        let top = m[1] - (m[3] / 2);
        (m, left, top)
    } else {
        // No monitor info: best-effort placement from selection bounds only.
        let y = sel_y + sel_h + VERTICAL_GAP - OVERLAY_TOP_INSET;
        return Some((x, y));
    };

    let clamp_y = |raw_y: i32| {
        let max_y = (mon_top + mon_h - win_h).max(mon_top);
        let clamped = raw_y.clamp(mon_top, max_y);
        (clamped, (clamped - raw_y).abs())
    };

    let pick_vertical = |selection_top: i32| {
        let below = selection_top + sel_h + VERTICAL_GAP - OVERLAY_TOP_INSET;
        let above = selection_top - win_h - VERTICAL_GAP - OVERLAY_TOP_INSET;
        let preferred = if below + win_h <= mon_top + mon_h { below } else { above };
        clamp_y(preferred)
    };

    // AX coordinate conventions vary by source. Score both common interpretations and
    // choose the one that requires less clamp correction.
    let (y_top, penalty_top) = pick_vertical(sel_y);
    let sel_top_flipped = mon_top + mon_h - sel_y - sel_h;
    let (y_flipped, penalty_flipped) = pick_vertical(sel_top_flipped);
    let mut y = if penalty_flipped < penalty_top { y_flipped } else { y_top };

    let max_x = (mon_left + mon_w - win_w).max(mon_left);
    x = x.clamp(mon_left, max_x);
    y = y.clamp(mon_top, (mon_top + mon_h - win_h).max(mon_top));

    Some((x, y))
}

#[cfg(target_os = "macos")]
fn compute_overlay_window_position_for_point(
    point: (i32, i32),
    monitor: Option<[i32; 4]>,
    window_size: (i32, i32),
) -> Option<(i32, i32)> {
    compute_overlay_window_position_for_selection([point.0, point.1, 1, 1], monitor, window_size)
}

#[cfg(target_os = "macos")]
fn get_cursor_position_macos() -> Option<(i32, i32)> {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).ok()?;
    let event = CGEvent::new(source).ok()?;
    let point = event.location();
    Some((point.x.round() as i32, point.y.round() as i32))
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

    let cfg = config::get();
    let hotkey_display = format_hotkey_display(&cfg.hotkey);
    let tooltip = format!("LinguaLens \u{2014} {}", hotkey_display);

    let _tray = TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip(&tooltip)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| {
            let id = event.id().as_ref();

            if id == "open_settings" {
                open_or_focus_window(app, "settings", "LinguaLens Settings", "settings.html", 560.0, 520.0);
            } else if id == "open_debug" {
                open_or_focus_window(app, "test", "LinguaLens Debug", "test.html", 820.0, 700.0);
            } else if id == "quit" {
                app.exit(0);
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
            let h = handle.clone();
            // Move ALL work off the main thread to prevent "Not Responding".
            // Text capture (UIA/clipboard, up to 550ms) must not block the message pump.
            std::thread::spawn(move || {
                let runtime_cfg = config::get();
                let center_overlay = runtime_cfg.overlay_position_mode.eq_ignore_ascii_case("center");

                #[cfg(target_os = "macos")]
                let selection_bounds = if !center_overlay && !runtime_cfg.force_clipboard {
                    get_selected_text_bounds_accessibility()
                } else {
                    None
                };
                #[cfg(target_os = "macos")]
                let cursor_pos = if center_overlay {
                    None
                } else {
                    get_cursor_position_macos()
                };

                let text = get_selected_text().unwrap_or_default();
                let text = text.trim().to_string();
                log::info!("[hotkey] captured text: {:?}", &text[..text.len().min(50)]);

                let monitor = get_active_monitor_center().ok();

                if let Some(window) = h.get_webview_window("main") {
                    if let Ok(size) = window.inner_size() {
                        #[cfg(target_os = "macos")]
                        let logical_size = {
                            let scale = window.scale_factor().unwrap_or(1.0).max(1.0);
                            (
                                ((size.width as f64) / scale).round() as i32,
                                ((size.height as f64) / scale).round() as i32,
                            )
                        };

                        #[cfg(not(target_os = "macos"))]
                        let logical_size = (size.width as i32, size.height as i32);

                        let mut desired_position: Option<(i32, i32)> = None;

                        #[cfg(target_os = "macos")]
                        {
                            if center_overlay {
                                log::info!("[hotkey] overlay_position_mode=center");
                            } else if let Some(bounds) = selection_bounds {
                                log::info!("[hotkey] AX selection bounds: {:?}", bounds);
                                desired_position =
                                    compute_overlay_window_position_for_selection(bounds, None, logical_size);
                            } else if let Some(point) = cursor_pos {
                                log::warn!(
                                    "[hotkey] AX bounds unavailable; falling back to cursor point: {:?}",
                                    point
                                );
                                desired_position =
                                    compute_overlay_window_position_for_point(point, None, logical_size);
                            } else {
                                log::warn!("[hotkey] AX bounds + cursor unavailable; centering overlay");
                            }
                        }

                        #[cfg(not(target_os = "macos"))]
                        {
                            if let Some([cx, cy, _, _]) = monitor {
                                desired_position = Some((
                                    cx - (logical_size.0 / 2),
                                    cy - (logical_size.1 / 2),
                                ));
                            }
                        }

                        // Show first on macOS, then apply position to ensure updates while hidden windows.
                        if let Err(e) = window.show() {
                            log::warn!("[hotkey] show failed: {e}");
                        }

                        if desired_position.is_none() {
                            if let Some([cx, cy, _, _]) = monitor {
                                desired_position = Some((
                                    cx - (logical_size.0 / 2),
                                    cy - (logical_size.1 / 2),
                                ));
                            }
                        }

                        if let Some((x, y)) = desired_position {
                            log::info!("[hotkey] positioning overlay at ({x}, {y})");
                            #[cfg(target_os = "macos")]
                            {
                                if let Err(e) = window.set_position(tauri::Position::Logical(
                                    tauri::LogicalPosition::new(x as f64, y as f64),
                                )) {
                                    log::warn!("[hotkey] set_position failed: {e}");
                                }
                            }

                            #[cfg(not(target_os = "macos"))]
                            {
                                if let Err(e) =
                                    window.set_position(tauri::PhysicalPosition::new(x, y))
                                {
                                    log::warn!("[hotkey] set_position failed: {e}");
                                }
                            }
                        }
                    }
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

    // Update tray tooltip with new hotkey
    if let Some(tray) = app_handle.tray_by_id("main") {
        let display = format_hotkey_display(&new_hotkey);
        let _ = tray.set_tooltip(Some(&format!("LinguaLens \u{2014} {}", display)));
    }

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

/// Trigger model preloading (called by frontend after first-run download completes).
#[tauri::command]
fn preload_models(app_handle: tauri::AppHandle) {
    let data_dir = app_handle.path().app_data_dir().unwrap_or_default();
    std::thread::spawn(move || {
        tts::preload(&data_dir);
        translate::preload(&data_dir);
    });
}

#[tauri::command]
fn diagnose_gpu() -> serde_json::Value {
    let tts_device = tts::get_device();
    let translate_loaded = translate::is_loaded();
    serde_json::json!({
        "tts_device": tts_device,
        "translate_loaded": translate_loaded,
        "force_cpu": config::get().force_cpu,
    })
}

#[tauri::command]
async fn diagnose_tts(app_handle: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let data_dir = app_handle.path().app_data_dir().map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        let t0 = std::time::Instant::now();
        let wav = tts::speak("test", "en", None, None, &data_dir)?;
        Ok(serde_json::json!({
            "device": tts::get_device(),
            "latency_ms": t0.elapsed().as_millis(),
            "wav_bytes": wav.len(),
        }))
    }).await.map_err(|e| format!("{e}"))?
}

#[tauri::command]
async fn diagnose_translate(app_handle: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let data_dir = app_handle.path().app_data_dir().map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        let t0 = std::time::Instant::now();
        let result = translate::translate("hello", "en", "es", &data_dir)?;
        Ok(serde_json::json!({
            "latency_ms": t0.elapsed().as_millis(),
            "result": result,
        }))
    }).await.map_err(|e| format!("{e}"))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent, None
        ))
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .setup(|app| {
            // Initialize config before anything else
            let data_dir = app.path().app_data_dir().unwrap_or_default();
            config::init(&data_dir);

            // Sync autostart state with config
            {
                use tauri_plugin_autostart::ManagerExt;
                let autostart = app.autolaunch();
                if config::get().start_at_login {
                    let _ = autostart.enable();
                } else {
                    let _ = autostart.disable();
                }
            }

            // Resolve espeak-ng path: bundled resource → system fallback
            {
                let resource_dir = app.path().resource_dir().unwrap_or_default();

                #[cfg(target_os = "windows")]
                let (bundled_exe, bundled_data, system_exe, system_data) = (
                    resource_dir.join("resources/espeak-ng/espeak-ng.exe"),
                    resource_dir.join("resources/espeak-ng/espeak-ng-data"),
                    PathBuf::from("C:/Program Files/eSpeak NG/espeak-ng.exe"),
                    PathBuf::from("C:/Program Files/eSpeak NG/espeak-ng-data"),
                );

                #[cfg(target_os = "macos")]
                let (bundled_exe, bundled_data, system_exe, system_data) = {
                    let homebrew = if cfg!(target_arch = "aarch64") {
                        PathBuf::from("/opt/homebrew")
                    } else {
                        PathBuf::from("/usr/local")
                    };
                    (
                        resource_dir.join("resources/espeak-ng/espeak-ng"),
                        resource_dir.join("resources/espeak-ng/espeak-ng-data"),
                        homebrew.join("bin/espeak-ng"),
                        homebrew.join("share/espeak-ng-data"),
                    )
                };

                #[cfg(not(any(target_os = "windows", target_os = "macos")))]
                let (bundled_exe, bundled_data, system_exe, system_data) = (
                    resource_dir.join("resources/espeak-ng/espeak-ng"),
                    resource_dir.join("resources/espeak-ng/espeak-ng-data"),
                    PathBuf::from("/usr/bin/espeak-ng"),
                    PathBuf::from("/usr/share/espeak-ng-data"),
                );

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

            // Resolve dictionary path: bundled resource → app data dir fallback
            {
                let resource_dir = app.path().resource_dir().unwrap_or_default();
                let bundled_dict = resource_dir.join("resources/dict");
                let appdata_dict = data_dir.join("models").join("dict");

                if bundled_dict.join("es-en.tsv").exists() {
                    log::info!("[dict] Using bundled: {}", bundled_dict.display());
                    let _ = DICT_DIR.set(bundled_dict);
                } else if appdata_dict.join("es-en.tsv").exists() {
                    log::info!("[dict] Using app data: {}", appdata_dict.display());
                    let _ = DICT_DIR.set(appdata_dict);
                } else {
                    log::warn!("[dict] No dictionary files found — single-word lookups unavailable");
                    let _ = DICT_DIR.set(bundled_dict); // will load empty
                }
            }

            // Initialize history DB
            if let Err(e) = history::init(&data_dir) {
                log::error!("[history] Init failed: {e}");
            }

            // System tray
            setup_tray(app)?;

            // Preload models: dict (CPU, parallel), then TTS (GPU), then translate (GPU).
            // TTS and translate are sequential — loading both accelerators at once can
            // cause context conflicts and GPU fallback to CPU.
            {
                let data_dir = data_dir.clone();
                std::thread::spawn(move || {
                    let dd = data_dir.clone();
                    let dict_thread = std::thread::spawn(move || dict::preload(&dd));

                    let kokoro_dir = data_dir.join("models").join("kokoro");
                    if kokoro_dir.join("model.onnx").exists() || kokoro_dir.join("model_quantized.onnx").exists() {
                        tts::preload(&data_dir);
                    }

                    let _ = dict_thread.join();

                    if data_dir.join("models").join("translategemma-4b-it.Q4_K_M.gguf").exists() {
                        translate::preload(&data_dir);
                    }
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
            stop_tts,
            check_models,
            start_download,
            cancel_download,
            preload_models,
            diagnose_gpu,
            diagnose_tts,
            diagnose_translate,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
