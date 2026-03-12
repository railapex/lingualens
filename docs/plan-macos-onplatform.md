# LinguaLens macOS — On-Platform Validation & Completion

**Context**: The `macos-port` branch has all cross-platform scaffolding done. Cargo feature flags, macOS text capture (Accessibility API + Cmd+C), rodio audio, CoreML TTS cascade, espeak-ng path resolution, CI matrix, DMG config — all implemented and compiling clean on Windows. None of the macOS `#[cfg(target_os = "macos")]` code paths have been tested on actual hardware.

**This doc**: Everything needed to validate, fix, test, and ship on a Mac.

**Branch**: `macos-port`
**Ref plan**: `docs/macos-port.md` (original architecture plan)

---

## Prerequisites

```bash
# Xcode Command Line Tools (Metal compiler + framework headers)
xcode-select --install

# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin

# Node 20+
brew install node

# espeak-ng (IPA phonemization)
brew install espeak-ng

# Clone + checkout
git clone https://github.com/railapex/lingualens.git
cd lingualens
git checkout macos-port
npm install
```

---

## Phase 1 — Does It Compile?

### 1.1 Cargo check

```bash
cd src-tauri
cargo check
```

**Expected**: Clean compile. All Windows-specific code is behind `#[cfg(target_os = "windows")]`.

**If it fails**: Most likely cause is `core-graphics` or `core-foundation` crate version issues. The `extern "C"` declarations for `AXUIElement*`, `CFRelease`, and `CGDisplay*` functions are hand-written — if the crate versions expose these natively, there may be symbol conflicts. Fix: remove our `extern "C"` block and use the crate's own bindings.

### 1.2 GPU features (automatic)

GPU deps are target-conditional in `Cargo.toml` — Metal + CoreML are enabled automatically on macOS, CUDA + DirectML on Windows. No feature flags needed. The `cargo check` above already validates this.

**Possible issues**:
- `llama-cpp-2` metal feature may need cmake + Metal framework headers. Xcode CLI Tools should provide these.
- `ort` coreml feature: the `download-binaries` feature should pull a macOS ONNX Runtime binary that includes CoreML EP. If it doesn't, you'll see a link error. Fix: `ORT_STRATEGY=system` + brew-install ONNX Runtime, or switch to `ort/load-dynamic`.

### 1.3 Full dev build

```bash
cd ..  # back to repo root
npm run dev
```

**Expected**: Vite dev server starts, Tauri window appears, tray icon shows in menu bar.

**Check**:
- [ ] Window appears (transparent, no decorations — the overlay window)
- [ ] Tray icon shows in menu bar
- [ ] Tray menu: Settings, Debug Tools, Quit all work
- [ ] Settings window opens, all controls functional
- [ ] "Start at login" toggle works (should use launchd via Tauri autostart plugin)

---

## Phase 2 — Core Feature Validation

### 2.1 Model download

```bash
# Pre-download models (or let the app do it on first launch)
export LINGUALENS_MODEL_DIR="$HOME/Library/Application Support/com.lingualens.app/models"
node scripts/download-models.mjs
```

Verify models land in `~/Library/Application Support/com.lingualens.app/models/`.

### 2.2 espeak-ng

Open Debug Tools (tray → Debug Tools → test.html). Test IPA generation.

**If IPA is empty**: Check logs. The espeak-ng path resolution (`lib.rs` setup) probes:
1. Bundled: `{resource_dir}/resources/espeak-ng/espeak-ng`
2. Homebrew ARM: `/opt/homebrew/bin/espeak-ng`
3. Homebrew Intel: `/usr/local/bin/espeak-ng`

In dev mode, bundled won't exist. Should fall back to Homebrew install. Verify:
```bash
which espeak-ng
espeak-ng -v es --ipa -q "hola"
```

### 2.3 Translation (Metal)

In Debug Tools, trigger a translation manually. Check logs for:
```
[translate] Model loaded on metal (N GPU layers)
```

If it falls back to CPU, check `diagnose_gpu` output (Debug Tools should show this). Metal requires `with_n_gpu_layers(999)` — this is already set at `translate.rs:77`.

**Performance baseline**: M1 base ≈ 20-25 tok/s, M2 ≈ 30-35 tok/s on the 4B Q4_K_M model. Translation latency ~300-500ms on M2 for a short phrase.

### 2.4 TTS (CoreML)

In Debug Tools, trigger TTS. Check logs for:
```
[tts] CoreML session created in Xms
```

**If CoreML fails**: Will fall back to CPU (logs will show `[tts] CoreML failed`). Common reasons:
- ONNX model has ops CoreML doesn't support — falls back gracefully, this is fine
- macOS < 12 — CoreML EP requires Monterey+
- ONNX Runtime binary doesn't include CoreML EP — see §1.2

**Performance**: CoreML on Apple Silicon: ~50-100ms for Kokoro 82M. CPU: ~500ms. Both are acceptable.

### 2.5 Audio playback (rodio)

After TTS generates audio, verify you hear it through speakers/headphones.

The `rodio` audio module runs on a dedicated thread (`lib.rs` audio module). If you hear nothing:
- Check macOS audio output device selection
- Check Console.app for `[audio] Failed to open output device` errors
- `rodio` uses `cpal` which uses CoreAudio on macOS — should work out of the box

### 2.6 Text capture — Accessibility API

1. Open any app (Safari, Notes, VS Code, Terminal)
2. Select some text
3. Press the hotkey (default: Ctrl+Alt+L)

**First time**: macOS will prompt for Accessibility permission. Grant it in System Settings → Privacy & Security → Accessibility → enable LinguaLens.

**Expected**: Overlay appears with translation of selected text.

**Check logs for**:
```
[capture] Accessibility success (N chars, X.Xms)
```

**If Accessibility fails** (logs show `Accessibility miss`): Falls back to Cmd+C clipboard simulation. This is normal for some apps. The `force_clipboard` dev switch in Settings can force Tier 2 for testing.

**Test across apps**:
- [ ] Safari — should work via Accessibility
- [ ] Notes — should work via Accessibility
- [ ] VS Code — may need clipboard fallback (Electron)
- [ ] Terminal — should work via Accessibility
- [ ] Preview (PDF) — may need clipboard fallback
- [ ] Chrome — may need clipboard fallback (Electron)

### 2.7 Text capture — Clipboard fallback (Cmd+C)

Enable `force_clipboard` in Settings → Developer. Select text, press hotkey.

**Check**:
- [ ] Text captured correctly
- [ ] Original clipboard contents preserved after capture (copy something first, capture, then paste — should get the original)
- [ ] No visible "flash" of Cmd+C in the target app (the modifier key release + 30ms delay should prevent this, but verify)

### 2.8 Monitor detection

**Single monitor**: Overlay should appear centered on screen.

**Multi-monitor**: Currently uses `CGMainDisplayID()` which returns the primary display (the one with the menu bar). If the overlay appears on the wrong screen, this needs the upgrade described in Phase 4.

### 2.9 Hotkey

- [ ] Default hotkey (Ctrl+Alt+L) registers and works
- [ ] Hotkey change (Settings → Shortcut → Change) captures new combo
- [ ] New hotkey works after save
- [ ] Consider: `Ctrl+Alt` isn't idiomatic macOS. `Cmd+Shift+L` or similar might be better as the default on Mac. Could detect platform in config default.

---

## Phase 3 — Tests

### 3.1 Existing tests

```bash
cd src-tauri
cargo test ```

**Expected passing** (no models/espeak needed):
- `test_encode_wav_header`
- `test_encode_wav_correct_length`
- `test_style_vector_selection`
- `test_list_voices` (needs voice files)
- `test_tokenizer_load` (needs tokenizer.json)
- `test_tokenize_ipa` (needs tokenizer.json)
- `test_voice_file_loading` (needs voice files)
- `test_lang_name`
- `test_detect_spanish_accented`
- `test_detect_spanish_sentence`

**Expected passing** (with espeak-ng installed):
- `test_phonemize_spanish`
- `test_phonemize_english`
- `test_phonemize_mi_not_my`
- `test_phonemize_empty`

These 4 currently fail on both platforms because the test calls `phonemize()` which calls `crate::espeak_exe()` which reads from a `OnceLock` that only `setup()` initializes. The tests need a way to initialize espeak path without running the full Tauri app setup.

**Fix**: Add a test helper that initializes the espeak statics:

```rust
// In lib.rs or a test_utils module
#[cfg(test)]
pub fn init_espeak_for_tests() {
    use std::path::PathBuf;

    #[cfg(target_os = "macos")]
    {
        let homebrew = if cfg!(target_arch = "aarch64") {
            PathBuf::from("/opt/homebrew")
        } else {
            PathBuf::from("/usr/local")
        };
        let _ = ESPEAK_PATH.set(homebrew.join("bin/espeak-ng"));
        let _ = ESPEAK_DATA.set(homebrew.join("share/espeak-ng-data"));
    }

    #[cfg(target_os = "windows")]
    {
        let _ = ESPEAK_PATH.set(PathBuf::from("C:/Program Files/eSpeak NG/espeak-ng.exe"));
        let _ = ESPEAK_DATA.set(PathBuf::from("C:/Program Files/eSpeak NG/espeak-ng-data"));
    }
}
```

Then in `tts::tests` and `translate::tests`, call `crate::init_espeak_for_tests()` at the start of tests that need it.

### 3.2 New macOS-specific tests to add

**Text capture** — hard to unit test (needs accessibility permission, a focused app). Better as manual verification (Phase 2). But you can add a basic smoke test:

```rust
#[cfg(target_os = "macos")]
#[test]
fn test_accessibility_api_available() {
    // Verify AXUIElementCreateSystemWide doesn't crash
    // (won't return useful data without accessibility permission, but shouldn't panic)
    let result = get_selected_text_accessibility();
    // Result is None without accessibility permission or focused element — that's fine
    assert!(result.is_none() || result.unwrap().len() > 0);
}
```

**Monitor detection**:

```rust
#[cfg(target_os = "macos")]
#[test]
fn test_monitor_detection_returns_sane_values() {
    let result = get_active_monitor_center();
    assert!(result.is_ok());
    let [cx, cy, w, h] = result.unwrap();
    assert!(w > 0 && h > 0, "Monitor dimensions must be positive");
    assert!(cx > 0 && cy > 0, "Center must be positive");
    assert!(w <= 8000 && h <= 8000, "Dimensions unreasonably large");
}
```

**Audio**:

```rust
#[test]
fn test_audio_play_stop_no_panic() {
    // Verify the audio thread initializes without crashing
    // (may not produce audible output in CI, but shouldn't panic)
    let wav = tts::encode_wav(&[0.0f32; 2400]).unwrap();
    audio::play(wav);
    std::thread::sleep(std::time::Duration::from_millis(100));
    audio::stop();
}
```

**espeak-ng path resolution**:

```rust
#[cfg(target_os = "macos")]
#[test]
fn test_espeak_homebrew_exists() {
    let arm_path = std::path::Path::new("/opt/homebrew/bin/espeak-ng");
    let intel_path = std::path::Path::new("/usr/local/bin/espeak-ng");
    assert!(
        arm_path.exists() || intel_path.exists(),
        "espeak-ng not found — install with: brew install espeak-ng"
    );
}
```

### 3.3 CI test feasibility

GitHub Actions `macos-latest` runners (M1) can run `cargo test` but:
- No accessibility permission → AX tests return None (not a failure)
- No audio output device → rodio may fail to open output (test should handle gracefully)
- No display → `CGMainDisplayID` may return a virtual display in CI

For CI, gate the interactive tests behind an env var:
```rust
fn is_interactive() -> bool {
    std::env::var("LINGUALENS_INTERACTIVE_TESTS").is_ok()
}
```

---

## Phase 4 — Known Improvements

These are things we know need work. Fix now or track for follow-up.

### 4.1 Multi-monitor overlay positioning

**Current**: `CGMainDisplayID()` → always returns primary display (menu bar screen).
**Correct**: Overlay should appear on the screen where the user selected text.

**Fix**: Use `NSScreen.mainScreen` which returns the screen containing the key window (the window receiving keyboard events — i.e., the app the user was just in).

```rust
#[cfg(target_os = "macos")]
fn get_active_monitor_center() -> Result<[i32; 4], String> {
    // NSScreen.mainScreen → screen with key window
    // Need objc2 or raw objc_msgSend for this
    unsafe {
        let cls = objc2::runtime::AnyClass::get("NSScreen").ok_or("NSScreen not found")?;
        let main_screen: *mut objc2::runtime::AnyObject = objc2::msg_send![cls, mainScreen];
        if main_screen.is_null() {
            return Err("No main screen".into());
        }
        let frame: core_graphics::geometry::CGRect = objc2::msg_send![main_screen, frame];
        let w = frame.size.width as i32;
        let h = frame.size.height as i32;
        let cx = frame.origin.x as i32 + w / 2;
        let cy = frame.origin.y as i32 + h / 2;
        Ok([cx, cy, w, h])
    }
}
```

**Dependencies**: May need `objc2` crate (or just `objc` for raw message sends). Evaluate whether the gain justifies adding a dependency vs. the CGDisplay approach being "good enough."

**Priority**: Medium. Only matters for multi-monitor setups.

### 4.2 Full pasteboard preservation

**Current**: `pbpaste`/`pbcopy` — text only. Images, RTF, file references on the clipboard get wiped during Cmd+C capture.

**Fix**: Use `NSPasteboard` directly via Objective-C FFI:

```rust
// Save all pasteboard items (types + data) before capture
// Restore after capture
// NSPasteboard.generalPasteboard.types → for each type, dataForType:
// After capture: clearContents, then setData:forType: for each saved item
```

**Complexity**: Moderate — needs `objc2` or raw message sends to NSPasteboard. The Tier 1 (Accessibility) path doesn't touch the clipboard at all, so this only matters when Accessibility fails (~20% of apps).

**Priority**: Medium-low. Most captures use Accessibility. Clipboard fallback is rare, and most users have text on their clipboard anyway.

### 4.3 Cmd+C clipboard polling

**Current**: Fixed 100ms sleep after simulating Cmd+C.
**Problem**: Too fast for slow apps (Electron), wasted time for fast apps.

**Fix**: Poll `NSPasteboard.changeCount` in a loop (macOS equivalent of Windows' `GetClipboardSequenceNumber`):

```rust
fn poll_pasteboard_change(count_before: i64, timeout_ms: u64) -> bool {
    // NSPasteboard.generalPasteboard.changeCount
    let start = std::time::Instant::now();
    loop {
        let current = get_pasteboard_change_count(); // objc call
        if current != count_before { return true; }
        if start.elapsed().as_millis() as u64 >= timeout_ms { return false; }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
```

**Priority**: Low. 100ms fixed sleep works for the vast majority of apps. Only matters if you notice "capture missed" in slow Electron apps.

### 4.4 `ort` load-dynamic for universal binary

**Current**: `ort` with `download-binaries` feature downloads a prebuilt ONNX Runtime dylib. For universal binary (arm64 + x86_64), each arch build downloads its own arch-specific dylib. The `lipo -create` step that merges the Rust binaries doesn't merge the ONNX Runtime dylib.

**Options**:
1. **`load-dynamic` feature**: Ship both `libonnxruntime.dylib` (arm64 + x86_64) in the bundle. At runtime, load the correct one based on `std::env::consts::ARCH`. Requires changing `ort` dependency:
   ```toml
   ort = { version = "2.0.0-rc.12", features = ["load-dynamic"] }
   ```
   And setting `ORT_DYLIB_PATH` at runtime to point to the correct dylib.

2. **Build a universal ONNX Runtime dylib**: `lipo -create libonnxruntime_arm64.dylib libonnxruntime_x86_64.dylib -output libonnxruntime.dylib`. Ship one fat dylib. This is what Apple's own frameworks do.

3. **Apple Silicon only**: Don't build universal. Ship arm64 only. Last Intel Mac sold in 2020 — they're aging out. Simplest option.

**Recommendation**: Start with option 3 (arm64 only). Add Intel later if there's demand. Saves significant CI complexity.

### 4.5 Default hotkey on macOS

`Ctrl+Alt+L` isn't idiomatic macOS. Users expect `Cmd+Shift+L` or similar.

**Fix**: Platform-conditional default in `config.rs`:

```rust
impl Default for Config {
    fn default() -> Self {
        Config {
            hotkey: if cfg!(target_os = "macos") {
                "super+shift+l".into()  // Cmd+Shift+L
            } else {
                "ctrl+alt+l".into()
            },
            // ...
        }
    }
}
```

Verify Tauri's global-shortcut plugin uses `super` for Cmd on macOS. Might be `command` or `meta` — check the Tauri docs.

### 4.6 macOS transparency / vibrancy

The overlay window uses `transparent: true` and `decorations: false`. On macOS, transparent windows may behave differently:
- Verify the overlay renders correctly (no white background flash, proper rounded corners)
- Consider adding `vibrancy` for the macOS frosted-glass effect (Tauri supports `"vibrancy": "under-window"` on macOS)
- Dark mode / light mode transitions should be tested

### 4.7 Info.plist merging

`src-tauri/Info.plist` was created with `NSAccessibilityUsageDescription`. Verify Tauri merges this with its auto-generated plist during build. If it doesn't:
- Move the key into `tauri.conf.json` under `bundle.macOS.infoPlist` (if Tauri 2 supports this)
- Or manually merge in a build script

Test: After building, inspect the app bundle's `Info.plist`:
```bash
cat src-tauri/target/release/bundle/macos/LinguaLens.app/Contents/Info.plist | grep -A1 Accessibility
```

---

## Phase 5 — Build & Package

### 5.1 Dev build (Apple Silicon only)

```bash
npm run dev
# or for a release build:
npx tauri build -- ```

Output: `src-tauri/target/release/bundle/dmg/LinguaLens_0.2.1_aarch64.dmg`

### 5.2 Universal binary

```bash
npx tauri build --target universal-apple-darwin -- ```

Output: `src-tauri/target/universal-apple-darwin/release/bundle/dmg/LinguaLens_0.2.1_universal.dmg`

**Known gotchas** (from the plan):
- `ort download-binaries` may only grab host-arch dylib — see §4.4
- `llama-cpp-2` cmake cross-compile: Xcode clang handles both arches, but watch for build.rs errors
- Bundled `espeak-ng` binary must match target arch. Homebrew builds are host-arch-only. For universal: either build espeak-ng from source as universal, or ship two binaries and select at runtime. For arm64-only: just bundle the Homebrew binary.
- Build time: ~2x. Two full Rust compile passes.

### 5.3 DMG testing

1. Mount the DMG
2. Drag to Applications
3. Launch from Applications folder
4. First-launch UX: model download dialog should appear
5. After download: hotkey should work, all features functional
6. Tray icon present in menu bar
7. Quit from tray menu

### 5.4 Unsigned DMG / Gatekeeper

Without code signing, macOS will block the app. Users must right-click → Open → Open to bypass Gatekeeper.

For production:
- Apple Developer account ($99/yr)
- Code signing: `codesign --deep --force --sign "Developer ID Application: ..." LinguaLens.app`
- Notarization: `xcrun notarytool submit LinguaLens.dmg --apple-id ... --password ... --team-id ...`
- Staple: `xcrun stapler staple LinguaLens.dmg`

This is a follow-up, not a blocker for testing.

---

## Checklist

### Compile & Launch
- [ ] `cargo check` succeeds (GPU features are target-conditional, no flags needed)
- [ ] `npm run dev` launches — window + tray icon appear
- [ ] Tray menu works (Settings, Debug Tools, Quit)
- [ ] Settings panel loads, all controls functional
- [ ] "Start at login" toggle works

### Models & Inference
- [ ] Model download writes to `~/Library/Application Support/com.lingualens.app/models/`
- [ ] espeak-ng phonemization works (IPA shows in Debug Tools)
- [ ] Translation works — check Metal acceleration in logs
- [ ] TTS works — check CoreML in logs
- [ ] Audio plays through speakers

### Text Capture
- [ ] Accessibility permission prompt on first capture
- [ ] Permission grant in System Settings works
- [ ] Safari: Accessibility tier captures text
- [ ] Notes: same
- [ ] VS Code: clipboard fallback works
- [ ] Terminal: works
- [ ] Clipboard contents preserved after capture
- [ ] `force_clipboard` dev switch works

### Build & Package
- [ ] `npx tauri build` produces .dmg
- [ ] DMG installs to /Applications
- [ ] Installed app launches and works
- [ ] `Info.plist` contains `NSAccessibilityUsageDescription`

### Tests
- [ ] Unit tests pass (`cargo test`)
- [ ] espeak test helper added and phonemize tests pass
- [ ] macOS-specific smoke tests added (see §3.2)

---

## Thoughts — Longer-Term Considerations

Things that don't need answers now but are worth thinking about as the Mac version matures.

- **Accessibility permission UX**: When the user denies or hasn't yet granted accessibility permission, the Accessibility tier silently returns None and falls back to Cmd+C. Should we detect this case and show a one-time prompt/banner guiding them to System Settings? `AXIsProcessTrusted()` returns whether we have permission.

- **Input Monitoring permission**: `CGEvent` keyboard simulation (Cmd+C) may require Input Monitoring permission on macOS 10.15+. If Cmd+C simulation doesn't work even after granting Accessibility, check if Input Monitoring is also needed. The Tauri global-shortcut plugin likely already requests this.

- **Sandbox considerations**: macOS apps distributed via the App Store must be sandboxed, which blocks Accessibility API access. DMG distribution outside the App Store avoids this. If App Store distribution is ever desired, the entire text capture approach would need rethinking (e.g., a companion helper tool outside the sandbox).

- **Retina / HiDPI**: `CGDisplayPixelsWide/High` returns logical pixels on Retina displays. If the overlay positioning is off by 2x, need to account for the display's backing scale factor via `CGDisplayScreenSize` or `NSScreen.backingScaleFactor`.

- **Menu bar icon**: Windows uses the `.ico` file for the tray icon. macOS menu bar icons should ideally be template images (monochrome, 22x22pt). Check that the current icon renders well in the menu bar — it may need a macOS-specific template icon.

- **Auto-updater on macOS**: The Tauri updater plugin works cross-platform, but the update manifest format needs the correct platform key. The CI workflow produces `darwin-universal` as the platform key — verify the updater plugin recognizes this. Tauri's expected keys are `darwin-aarch64`, `darwin-x86_64`, or `darwin-universal`.

- **Kokoro ONNX model conversion**: CoreML EP converts ONNX ops at session creation time. This adds a one-time delay (~5-10s) on first launch. CoreML caches the compiled model, so subsequent launches are fast. The `with_model_cache_dir()` builder method can control where this cache lives. Consider setting it to the app data dir so the cache persists across updates.

- **Apple Silicon GPU memory**: Unlike CUDA, Metal shares unified memory. TranslateGemma 4B Q4_K_M needs ~2.5GB — fine for 8GB M1, but tight with other apps. Monitor memory pressure via `os_proc_available_memory()` and log warnings if the system is under pressure.

- **espeak-ng bundle size**: The full `espeak-ng-data` directory is ~10MB. For a leaner DMG, could strip to only the languages LinguaLens supports (currently: es, en, fr, de, it, pt, ja, zh). Each language's data is ~100-500KB; the rest is unused.

- **Rosetta 2 performance**: If shipping arm64-only but someone runs on Intel via Rosetta: translation and TTS will be slower (no Metal, CPU-only under emulation). The app will work but may feel sluggish. Worth noting in release notes if not building universal.
