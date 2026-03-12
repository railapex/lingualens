# LinguaLens — macOS Port Plan

## Project Overview

**LinguaLens** is a desktop app for ambient language learning. Select text anywhere on screen → press a hotkey → floating overlay shows translation + IPA pronunciation + text-to-speech. All ML inference runs in-process — no cloud APIs.

**Repo**: `github.com/railapex/lingualens`
**Stack**: Tauri 2 (Rust backend + vanilla JS frontend)
**Models**:
- TranslateGemma 4B Q4_K_M (2.5GB GGUF) — translation via `llama-cpp-2` crate
- Kokoro 82M (325MB ONNX) — TTS via `ort` crate (ONNX Runtime)
- espeak-ng — IPA phonemization (bundled binary)

**Current state**: v0.2.1 shipping on Windows. NSIS installer, auto-updater, GitHub Actions CI. GPU-accelerated via CUDA → DirectML → CPU cascade.

**Goal**: Add macOS support — universal binary (Apple Silicon + Intel), Metal GPU acceleration for translation, CoreML for TTS, DMG distribution.

---

## What's Already Cross-Platform (No Changes Needed)

- **All frontend code** — HTML/CSS/JS, no platform detection, no hardcoded paths
- **Translation logic** — `src-tauri/src/translate.rs` — pure Rust + llama-cpp-2. GPU layer offload API (`with_n_gpu_layers(999)`) is identical for Metal and CUDA
- **TTS logic** — `src-tauri/src/tts.rs` — ONNX inference core. Execution provider cascade already handles missing providers gracefully (try → catch → fall back)
- **Config, history, dictionary, language detection** — pure Rust, no OS APIs
- **Tauri plugins** — global-shortcut, updater, autostart, log — all cross-platform
- **Hotkey registration** — Tauri plugin abstracts OS-level shortcut APIs
- **Model files** — GGUF and ONNX formats are universal, same files on all platforms
- **`main.rs`** — `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` is a no-op on macOS

---

## Changes Required

### 1. Cargo.toml — GPU Feature Flags

**Problem**: Current deps hardcode CUDA + DirectML features (Windows-only).

**Current** (`src-tauri/Cargo.toml` L27, 31):
```toml
llama-cpp-2 = { version = "0.1", features = ["cuda"] }
ort = { version = "2.0.0-rc.12", features = ["download-binaries", "cuda", "directml"] }
```

**Solution**: Platform-conditional dependencies via Cargo features.

```toml
[features]
default = []
gpu-windows = ["llama-cpp-2/cuda", "ort/cuda", "ort/directml"]
gpu-macos = ["llama-cpp-2/metal", "ort/coreml"]

[dependencies]
llama-cpp-2 = "0.1"
ort = { version = "2.0.0-rc.12", features = ["download-binaries"] }
```

**Key findings from research**:

**Metal (llama-cpp-2)**:
- `metal` feature exists: `llama-cpp-2 = { features = ["metal"] }` → passes through to `llama-cpp-sys-2/metal`
- **Auto-enabled on `aarch64-apple-darwin`** — the crate's own `Cargo.toml` has a target-specific dep that enables it. Explicitly adding it is harmless and clearer.
- Same API — `with_n_gpu_layers(999)` works identically for Metal and CUDA. **No code changes in `translate.rs`**.
- Build requires Xcode Command Line Tools (provides Metal compiler + framework headers). No separate SDK download.
- **Performance**: ~2.4x speedup over CPU. M1 base ≈ 20-25 tok/s, M2 ≈ 30-35 tok/s, M3 Pro ≈ 45-50 tok/s on a 4B Q4_K_M model. Translation latency ~300-500ms on M2 (vs ~2s CPU-only).
- **Known gotcha**: Must set `with_n_gpu_layers(999)` (project already does this at `translate.rs:75`). Default is 0 = CPU-only, which is why some users report slow Metal perf.

**CoreML (ort)**:
- `coreml` feature exists in ort 2.0.0-rc.12
- Same builder pattern: `ort::ep::CoreML::default().build()`
- Accepts ONNX models directly — **no offline model conversion needed**. CoreML EP converts ONNX ops to CoreML format during session creation.
- Available when `download-binaries` is used — prebuilt ONNX Runtime binary already includes CoreML EP on macOS. No extra toolchain.
- Requires macOS 12+ (Monterey) for MLProgram format
- **Performance**: 3-10x over CPU on Apple Silicon with Neural Engine. For Kokoro 82M, expect ~50-100ms (vs ~500ms CPU).
- Useful builder methods: `.with_model_cache_dir(path)` (cache compiled models — avoids recompilation on restart)
- Unsupported ops fall back to CPU automatically

**Code change in `tts.rs`** — Add CoreML to the execution provider cascade (`create_session()` L321–362):
```rust
// After DirectML attempt, before CPU fallback:
#[cfg(target_os = "macos")]
{
    // CoreML attempt
    let t0 = std::time::Instant::now();
    match Session::builder()
        .and_then(|b| b.with_execution_providers([ort::ep::CoreML::default().build()]))
        .and_then(|mut b| b.commit_from_file(fp32_path))
    {
        Ok(session) => {
            log::info!("[tts] CoreML session in {:.0?}", t0.elapsed());
            return Ok((session, "coreml".into()));
        }
        Err(e) => log::warn!("[tts] CoreML failed ({:.0?}): {}", t0.elapsed(), e),
    }
}
```

**CI builds**: `npx tauri build -- --features gpu-windows` on Windows, `npx tauri build -- --features gpu-macos` on macOS.

---

### 2. Text Capture (`lib.rs` L37–458) — **Biggest piece**

All text capture code is Windows-specific behind `#[cfg(target_os = "windows")]`. There's already a `#[cfg(not(target_os = "windows"))]` stub returning `Err("Not implemented")` at L454–457.

**Windows implementation** (for reference):
- **Tier 1**: UI Automation TextPattern — COM-based, clipboard-free (~80% of apps). Uses `IUIAutomation` → `GetFocusedElement()` → `GetCurrentPattern(UIA_TextPatternId)` → `GetSelection()`
- **Tier 2**: Clipboard simulation — save all clipboard formats → clear → simulate Ctrl+C via `SendInput` Win32 → poll clipboard sequence number → read text → restore formats + exclusion markers

**macOS implementation needed**:

**Tier 1 — Accessibility API** (`AXUIElement`):
```rust
#[cfg(target_os = "macos")]
fn get_selected_text_accessibility() -> Option<String> {
    // 1. Get system-wide accessibility element
    //    AXUIElementCreateSystemWide()
    // 2. Get focused application
    //    AXUIElementCopyAttributeValue(system, kAXFocusedApplicationAttribute)
    // 3. Get focused UI element
    //    AXUIElementCopyAttributeValue(app, kAXFocusedUIElementAttribute)
    // 4. Get selected text
    //    AXUIElementCopyAttributeValue(element, kAXSelectedTextAttribute)
}
```
- **Requires Accessibility permission** — user must grant in System Settings > Privacy & Security > Accessibility
- Add to Tauri's `Info.plist`: `NSAccessibilityUsageDescription: "LinguaLens needs accessibility access to read selected text from other applications."`
- Coverage: similar to Windows UIA (~80% of Cocoa apps)
- Crate options: `accessibility` crate wraps Core Foundation accessibility APIs, or use raw `core-foundation` + `objc2`

**Tier 2 — Clipboard + Cmd+C**:
```rust
#[cfg(target_os = "macos")]
fn simulate_cmd_c() {
    // CGEventCreateKeyboardEvent(nil, kVK_ANSI_C, true/false)
    // CGEventSetFlags(event, kCGEventFlagMaskCommand)
    // CGEventPost(kCGHIDEventTap, event)
}
```
- Save `NSPasteboard.generalPasteboard` contents → clear → Cmd+C → read → restore
- macOS pasteboard is simpler than Win32 clipboard — fewer format concerns for text
- No equivalent to Windows clipboard exclusion markers (Win+V history) — macOS doesn't have system-level clipboard history

**Crate dependencies** — Add to `Cargo.toml`:
```toml
[target.'cfg(target_os = "macos")'.dependencies]
core-graphics = "0.24"        # CGEvent keyboard simulation
core-foundation = "0.10"      # AXUIElement accessibility queries
# OR: accessibility = "0.1"   # Higher-level wrapper for both
```

**Structure**: Mirror the Windows pattern — `get_selected_text()` tries Tier 1 first, falls back to Tier 2. The `config.force_clipboard` dev switch should work identically (skip Tier 1, go straight to clipboard).

---

### 3. Audio Playback (`lib.rs` L602–639)

**Current**: Windows `PlaySoundW` from `winmm.dll` — WAV bytes in a static buffer, async playback.

**Recommended**: Replace with `rodio` crate — cross-platform, replaces both the Windows and (nonexistent) macOS implementations:

```toml
# Cargo.toml - replaces winmm entirely
rodio = "0.19"
```

```rust
// lib.rs - replaces both #[cfg] audio modules
mod audio {
    use std::io::Cursor;
    use std::sync::{Mutex, OnceLock};
    use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};

    static STATE: OnceLock<Mutex<(OutputStream, OutputStreamHandle, Sink)>> = OnceLock::new();

    fn get_or_init() -> &'static Mutex<(OutputStream, OutputStreamHandle, Sink)> {
        STATE.get_or_init(|| {
            let (stream, handle) = OutputStream::try_default().unwrap();
            let sink = Sink::try_new(&handle).unwrap();
            Mutex::new((stream, handle, sink))
        })
    }

    pub fn play(wav_bytes: Vec<u8>) {
        let guard = get_or_init().lock().unwrap();
        guard.2.stop(); // stop current
        let source = Decoder::new(Cursor::new(wav_bytes)).unwrap();
        guard.2.append(source);
    }

    pub fn stop() {
        if let Some(state) = STATE.get() {
            let guard = state.lock().unwrap();
            guard.2.stop();
        }
    }
}
```

This deletes the `#[cfg(target_os = "windows")]` and `#[cfg(not(target_os = "windows"))]` audio modules entirely. One implementation for all platforms.

---

### 4. Monitor Detection (`lib.rs` L462–494)

**Current**: Windows-specific `GetForegroundWindow` + `MonitorFromWindow` + `GetMonitorInfoW`. macOS stub returns `Err("Not implemented")`.

**Approach**: Try Tauri's cross-platform monitor API first:
```rust
fn get_active_monitor_center(window: &tauri::WebviewWindow) -> Result<[i32; 4], String> {
    let monitor = window.current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("No monitor found")?;
    let pos = monitor.position();
    let size = monitor.size();
    Ok([
        pos.x + (size.width as i32 / 2),
        pos.y + (size.height as i32 / 2),
        size.width as i32,
        size.height as i32,
    ])
}
```

**Caveat**: This gets the monitor containing the *LinguaLens window*, not the *focused app's window*. On Windows, `GetForegroundWindow` gets the app the user was just in. This matters for multi-monitor — the overlay should appear on the monitor where the user selected text, not where LinguaLens lives.

If this is wrong on macOS: use `NSWorkspace.sharedWorkspace.frontmostApplication` → get its window bounds → find containing screen. But try the simple approach first.

**Note**: The command signature changes — it needs the window handle. Update the Tauri command invocation in `lib.rs` L889 where `get_active_monitor_center()` is called from the hotkey handler.

---

### 5. espeak-ng Paths (`lib.rs` L1058–1079)

**Current**: Hardcoded `espeak-ng.exe` and `C:/Program Files/eSpeak NG/` fallback.

**Change**: Platform-conditional path resolution in `setup()`:

```rust
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

    // rest of probe logic is identical
}
```

**`CREATE_NO_WINDOW` flag** (`lib.rs` L502, `tts.rs` L181): Already guarded by `#[cfg(target_os = "windows")]`. Verify, but no change needed.

**CI bundling** (macOS): `brew install espeak-ng`, then copy:
```bash
mkdir -p src-tauri/resources/espeak-ng
cp /opt/homebrew/bin/espeak-ng src-tauri/resources/espeak-ng/
cp -r /opt/homebrew/share/espeak-ng-data src-tauri/resources/espeak-ng/
```

---

### 6. Config Field Rename

Rename `start_with_windows` → `start_at_login` throughout:

**Files to update**:
- `src-tauri/src/config.rs` L21, L43 — struct field + default
- `src-tauri/src/lib.rs` L593 — `set_config` handler
- `src-tauri/src/lib.rs` L1051 — startup sync
- `src/settings.js` — config key in `updateConfig()` call
- `src/settings.html` — label text (change "Start with Windows" → "Start at login")

**Migration**: Add a one-time migration in `config::init()` — if the loaded JSON has `start_with_windows` but not `start_at_login`, copy the value over. Or just let it default to `false` for existing users (it's a minor setting).

---

### 7. Dev Tooling

**`vite.config.js`** L7, L40 — Hardcoded `.exe` paths:

```js
// L7: fix tts_cli binary extension
const isWindows = process.platform === 'win32';
const ttsCli = resolve(__dirname, `src-tauri/target/debug/tts_cli${isWindows ? '.exe' : ''}`);

// L40: fix espeak-ng path in /api/phonemize middleware
const espeakBin = isWindows
    ? 'C:/Program Files/eSpeak NG/espeak-ng.exe'
    : (process.arch === 'arm64' ? '/opt/homebrew/bin/espeak-ng' : '/usr/local/bin/espeak-ng');
```

**`scripts/download-models.mjs`** L13–15 — Hardcoded `AppData/Roaming`:
```js
const MODELS_DIR = process.env.LINGUALENS_MODEL_DIR ||
    (process.platform === 'win32'
        ? join(homedir(), 'AppData/Roaming/com.lingualens.app/models')
        : join(homedir(), 'Library/Application Support/com.lingualens.app/models'));
```

---

### 8. Build Configuration

**`tauri.conf.json`**:
- Change `"targets": ["nsis"]` → `"targets": "all"` (Tauri picks NSIS on Windows, DMG on macOS)
- Add macOS minimum version:
```json
"bundle": {
    "targets": "all",
    "macOS": {
        "minimumSystemVersion": "12.0"
    }
}
```

**`Info.plist` entries** (Tauri config or custom plist):
- `NSAccessibilityUsageDescription` — required for AXUIElement text capture
- Optional: `NSMicrophoneUsageDescription` if future audio input features are planned

---

### 9. Universal Binary

**Tauri supports this natively**: `npx tauri build --target universal-apple-darwin`

**Setup**:
```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

Tauri runs two `cargo build` invocations (one per arch), then `lipo -create` to merge into a single fat Mach-O binary. Output is ~2x single-arch size.

**Gotchas**:
- **ort `download-binaries`**: May grab only the host arch's dylib. If the x86_64 build fails to find an x86_64 ONNX Runtime binary, switch to `load-dynamic` feature (ship both dylibs, load at runtime) or set `ORT_STRATEGY=system` with a pre-merged universal dylib.
- **llama-cpp-2**: cmake cross-compile generally works on macOS (Xcode clang handles both arches). build.rs disables `GGML_NATIVE` on cross-compile (correct behavior — native CPU optimizations must match target arch).
- **espeak-ng sidecar**: The bundled binary itself needs to be universal. Homebrew builds are host-arch-only. Options: build espeak-ng from source as universal, or ship two binaries and select at runtime.
- **Build time**: ~2x Rust compile time. Mitigate in CI by building both arches on separate runners, merging in a final job.

**Alternative**: Ship Apple Silicon only. Intel Macs are aging out (last models 2020). Simplifies everything. Can always add Intel later.

---

### 10. CI Matrix Build (`.github/workflows/release.yml`)

Current: single `windows-latest` job. Add macOS:

```yaml
jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: windows-latest
            features: gpu-windows
            espeak-install: choco install espeak-ng -y --no-progress
            espeak-src: C:/Program Files/eSpeak NG
            tauri-args: ""
          - os: macos-latest
            features: gpu-macos
            espeak-install: brew install espeak-ng
            espeak-src: /opt/homebrew
            tauri-args: --target universal-apple-darwin
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.os == 'macos-latest' && 'aarch64-apple-darwin,x86_64-apple-darwin' || '' }}
      - uses: actions/setup-node@v4
        with:
          node-version: '20'
          cache: 'npm'

      # Windows-only: CUDA toolkit
      - if: matrix.os == 'windows-latest'
        uses: Jimver/cuda-toolkit@v0.2.23
        with:
          cuda: '12.6.0'
          method: 'network'
          use-github-cache: true

      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: src-tauri
          cache-on-failure: true

      - name: Install espeak-ng
        run: ${{ matrix.espeak-install }}

      - run: npm ci

      - name: Copy espeak-ng resources
        shell: bash
        run: |
          mkdir -p src-tauri/resources/espeak-ng
          if [[ "$RUNNER_OS" == "Windows" ]]; then
            cp "${{ matrix.espeak-src }}/espeak-ng.exe" src-tauri/resources/espeak-ng/
            cp "${{ matrix.espeak-src }}/libespeak-ng.dll" src-tauri/resources/espeak-ng/
            cp -r "${{ matrix.espeak-src }}/espeak-ng-data" src-tauri/resources/espeak-ng/
          else
            cp "${{ matrix.espeak-src }}/bin/espeak-ng" src-tauri/resources/espeak-ng/
            cp -r "${{ matrix.espeak-src }}/share/espeak-ng-data" src-tauri/resources/espeak-ng/
          fi

      - name: Build Tauri app
        run: npx tauri build ${{ matrix.tauri-args }} -- --features ${{ matrix.features }}
        env:
          CUDAFLAGS: ${{ matrix.os == 'windows-latest' && '--allow-unsupported-compiler' || '' }}
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}

      # Update manifest — each platform contributes its entry
      - name: Generate update manifest
        shell: bash
        run: |
          VERSION="${GITHUB_REF_NAME#v}"
          TAG="${GITHUB_REF_NAME}"
          if [[ "$RUNNER_OS" == "Windows" ]]; then
            ARTIFACT=$(find src-tauri/target/release/bundle/nsis -name '*.nsis.zip' | head -1)
            PLATFORM="windows-x86_64"
          else
            ARTIFACT=$(find src-tauri/target/universal-apple-darwin/release/bundle/dmg -name '*.dmg' | head -1)
            PLATFORM="darwin-universal"
          fi
          SIGNATURE=$(cat "${ARTIFACT}.sig")
          FILENAME=$(basename "$ARTIFACT")
          jq --null-input \
            --arg version "$VERSION" \
            --arg notes "See https://github.com/railapex/lingualens/releases/tag/$TAG" \
            --arg pub_date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
            --arg sig "$SIGNATURE" \
            --arg url "https://github.com/railapex/lingualens/releases/download/$TAG/$FILENAME" \
            --arg platform "$PLATFORM" \
            '{version: $version, notes: $notes, pub_date: $pub_date, platforms: {($platform): {signature: $sig, url: $url}}}' \
            > "latest-$PLATFORM.json"

      - name: Upload release artifacts
        uses: softprops/action-gh-release@v2
        with:
          files: |
            src-tauri/target/release/bundle/nsis/*.exe
            src-tauri/target/release/bundle/nsis/*.nsis.zip
            src-tauri/target/universal-apple-darwin/release/bundle/dmg/*.dmg
            latest-*.json
          generate_release_notes: true

  # Merge per-platform manifests into single latest.json
  merge-manifest:
    needs: build
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - name: Download manifests
        uses: actions/download-artifact@v4
        # ... download latest-windows-x86_64.json and latest-darwin-universal.json
        # merge with jq into single latest.json
        # upload to release
```

**Note**: The manifest merge job is a sketch — the exact mechanism depends on how artifacts flow between jobs. May need `actions/upload-artifact` + `actions/download-artifact` instead of relying on the release assets. Work this out during implementation.

**Code signing**: Defer to follow-up. Unsigned DMGs work for testing (right-click > Open bypasses Gatekeeper). Production signing needs Apple Developer cert ($99/yr) + notarization via `xcrun notarytool`.

---

## Implementation Order

| Step | What | Gets you to... | Estimated effort |
|------|------|----------------|------------------|
| 1 | Cargo feature flags (§1) | Compiles on macOS (CPU-only) | 30 min |
| 2 | espeak-ng paths (§5) | IPA phonemization works | 30 min |
| 3 | Audio playback — rodio (§3) | TTS audio plays | 1 hr |
| 4 | Config rename (§6) | Cross-platform config | 30 min |
| 5 | Dev tooling (§7) | `npm run dev` works on macOS | 30 min |
| 6 | Text capture (§2) | **Core feature works** | 4-8 hrs |
| 7 | Monitor detection (§4) | Overlay positions correctly | 1-2 hrs |
| 8 | CoreML in TTS cascade (§1) | TTS accelerated on Apple Silicon | 1 hr |
| 9 | Build config + DMG (§8) | Produces installable .dmg | 1 hr |
| 10 | Universal binary (§9) | Runs on Intel + Apple Silicon | 2-4 hrs |
| 11 | CI matrix (§10) | Automated releases for both platforms | 2-4 hrs |

**Milestone 1** (steps 1-5): App launches on macOS, models load, TTS + translation work, audio plays. No text capture yet — test via debug tools (test.html).

**Milestone 2** (steps 6-8): Text capture works, overlay positions correctly, CoreML acceleration. **This is a usable app.**

**Milestone 3** (steps 9-11): Distribution. Universal binary, DMG, CI.

---

## Getting Started

### Prerequisites
- macOS 12+ (Monterey or later)
- Xcode Command Line Tools: `xcode-select --install`
- Rust: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- Both Rust targets: `rustup target add aarch64-apple-darwin x86_64-apple-darwin`
- Node 20+: `brew install node`
- espeak-ng: `brew install espeak-ng`

### First Steps
```bash
git clone https://github.com/railapex/lingualens.git
cd lingualens
npm install

# This will fail — that's your starting signal:
cd src-tauri && cargo check
# Error: can't find crate `windows` — expected!

# After implementing steps 1-2:
cargo check  # should compile
cd .. && npm run dev  # should launch
```

### Model Download
Models are ~2.9GB total, downloaded on first launch. For dev, pre-download:
```bash
# Set the env var OR fix the script path first (step 5)
export LINGUALENS_MODEL_DIR="$HOME/Library/Application Support/com.lingualens.app/models"
node scripts/download-models.mjs
```

---

## Verification Checklist

### Milestone 1 — Compiles + launches
- [ ] `cargo check` succeeds (no Windows crate errors)
- [ ] `npm run dev` launches — window appears, tray icon shows
- [ ] Tray menu works (Settings, Debug Tools, Quit)
- [ ] Model download writes to `~/Library/Application Support/com.lingualens.app/models/`
- [ ] Debug tools (test.html): can trigger translation manually
- [ ] Debug tools: TTS generates audio, plays through speakers
- [ ] espeak-ng phonemization works (IPA shows in overlay)
- [ ] Settings panel loads, all controls functional
- [ ] `diagnose_gpu` reports correct device (Metal or CPU for translate, CoreML or CPU for TTS)

### Milestone 2 — Core feature works
- [ ] Select text in Safari → press hotkey → overlay appears with translation
- [ ] Select text in Notes, VS Code, Terminal — same result
- [ ] Overlay appears on the correct monitor (multi-monitor setup)
- [ ] Accessibility permission prompt appears on first text capture attempt
- [ ] Clipboard contents preserved after text capture (paste returns original content)
- [ ] History records translations
- [ ] Settings > hotkey change works
- [ ] Light theme + dark theme both render correctly

### Milestone 3 — Distribution
- [ ] `npx tauri build --target universal-apple-darwin -- --features gpu-macos` succeeds
- [ ] Produced `.dmg` installs on a clean Mac
- [ ] App works on Apple Silicon Mac
- [ ] App works on Intel Mac (if building universal)
- [ ] CI workflow builds both Windows + macOS
- [ ] Release creates GitHub release with both NSIS + DMG artifacts
- [ ] Auto-updater finds + installs updates (test with version bump)

---

## Tests

### Existing Tests (33 total)

**`tts.rs`** — 18 tests:
- Unit tests (no models): phonemization (4), tokenizer (2), WAV encoding (2)
- Integration tests (need models): TTS synthesis (4), voice loading (2), GPU detection (1), WAV output (1), voice listing (1), style vector (1)

**`translate.rs`** — 15 tests:
- Unit tests: language detection (7), prompt formatting (1), language name mapping (1)
- Integration tests (need models): translation inference (6)

### What Breaks on macOS

Both test modules hardcode `APPDATA` for model paths:

```rust
// tts.rs L634 — APPDATA is Windows-only
PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| "C:/Users/chris/AppData/Roaming".into()))
    .join("com.lingualens.app")

// translate.rs L268 — APPDATA or panic
let appdata = std::env::var("APPDATA").expect("APPDATA not set");
PathBuf::from(appdata).join("com.lingualens.app")
```

### Fix Required (step 1, alongside Cargo changes)

Replace hardcoded `APPDATA` with cross-platform path resolution:

```rust
fn test_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("LINGUALENS_MODEL_DIR") {
        return PathBuf::from(dir).parent().unwrap().to_path_buf();
    }
    #[cfg(target_os = "windows")]
    {
        PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| {
            dirs::data_dir().unwrap().to_string_lossy().into()
        })).join("com.lingualens.app")
    }
    #[cfg(target_os = "macos")]
    {
        dirs::data_dir().unwrap().join("com.lingualens.app")
        // ~/Library/Application Support/com.lingualens.app
    }
}
```

Or simpler — use the `dirs` crate (already a transitive dep via Tauri):
```rust
fn test_data_dir() -> PathBuf {
    dirs::data_dir().unwrap().join("com.lingualens.app")
}
```
This returns `%APPDATA%` on Windows and `~/Library/Application Support` on macOS. Same result, zero platform code.

### Tests That Should Pass at Each Milestone

**Milestone 1** (compiles + launches):
- `cargo test` in `src-tauri/` — all unit tests pass (phonemization needs espeak-ng installed)
- Integration tests pass if models are downloaded to the correct macOS path
- Run: `cd src-tauri && cargo test` (no feature flag needed — tests don't use GPU features)

**Milestone 2** (text capture works):
- No new automated tests needed for text capture — it's inherently manual (requires focused window + selected text)
- Could add a basic smoke test for accessibility permission check

**Milestone 3** (distribution):
- CI should run `cargo test` on both matrix legs (Windows + macOS)
- Add to CI workflow after build step:
```yaml
- name: Run tests
  run: cd src-tauri && cargo test
```

### New Tests to Consider

Not blockers, but nice-to-have:
- `test_coreml_session_creation()` — verify CoreML EP loads without error on macOS
- `test_metal_gpu_layers()` — verify Metal offload reports >0 layers
- `test_rodio_playback()` — verify audio output device initializes
- `test_espeak_path_resolution()` — verify espeak-ng found on macOS paths

---

## Architecture Reference

### Key Files (platform-specific code lives here)

| File | What's there | What to change |
|------|-------------|----------------|
| `src-tauri/Cargo.toml` | CUDA/DirectML deps | Add feature flags, macOS deps |
| `src-tauri/src/lib.rs` L37-458 | Text capture (Windows) | Add macOS Accessibility + Cmd+C |
| `src-tauri/src/lib.rs` L462-494 | Monitor detection (Windows) | Cross-platform or macOS impl |
| `src-tauri/src/lib.rs` L602-639 | Audio playback (winmm) | Replace with rodio |
| `src-tauri/src/lib.rs` L1058-1079 | espeak-ng path resolution | Add macOS paths |
| `src-tauri/src/tts.rs` L321-362 | GPU cascade (CUDA→DML→CPU) | Add CoreML step |
| `src-tauri/src/config.rs` L21 | `start_with_windows` field | Rename to `start_at_login` |
| `src-tauri/tauri.conf.json` | NSIS-only bundle target | Add DMG, macOS config |
| `vite.config.js` L7, L40 | Hardcoded .exe paths | Platform detection |
| `scripts/download-models.mjs` L13-15 | Hardcoded AppData path | Platform detection |
| `.github/workflows/release.yml` | Windows-only CI | Matrix strategy |

### Files that need NO changes
- `src/index.html`, `src/settings.html`, `src/test.html`
- `src/main.js`, `src/settings.js`, `src/audio.js`, `src/ipa.js`, `src/lang.js`, `src/tts.js`
- `src/style.css`
- `src-tauri/src/translate.rs` (Metal uses same API as CUDA)
- `src-tauri/src/download.rs` (model URLs + validation are platform-agnostic)
- `src-tauri/src/history.rs` (SQLite via rusqlite bundled)
- `src-tauri/src/dict.rs`
- `src-tauri/src/main.rs`
