# Changelog

## v0.3.0 (unreleased)

macOS port — cross-platform scaffolding for Apple Silicon + Intel.

### Added

- **macOS text capture** — two-tier: Accessibility API (AXUIElement, clipboard-free) → Cmd+C simulation via CGEvent with pasteboard save/restore
- **macOS monitor detection** — CGDisplay API for overlay positioning
- **macOS espeak-ng paths** — Homebrew ARM (`/opt/homebrew`) and Intel (`/usr/local`) fallback
- **CoreML TTS** — added to GPU cascade: CUDA → DirectML → CoreML → CPU
- **Metal translation** — `llama-cpp-2/metal` feature flag, same `with_n_gpu_layers(999)` API as CUDA
- **Target-conditional GPU deps** — CUDA + DirectML on Windows, Metal + CoreML on macOS, automatic per platform (no feature flags needed)
- **CI matrix build** — Windows + macOS runners, per-platform manifest generation with merge job
- **DMG bundle target** — `"targets": "all"` in tauri.conf.json, `macOS.minimumSystemVersion: "12.0"`
- **Info.plist** — `NSAccessibilityUsageDescription` for Accessibility permission prompt
- **On-platform validation plan** — `docs/plan-macos-onplatform.md` for Mac-side testing and completion

### Changed

- **Audio playback** — replaced Windows-only `winmm PlaySoundW` with cross-platform `rodio` crate (dedicated audio thread, channel-based architecture)
- **Config field rename** — `start_with_windows` → `start_at_login` with `#[serde(alias)]` for migration
- **Settings UI** — "Start with Windows" → "Start at login"
- **Dev tooling** — `vite.config.js` and `scripts/download-models.mjs` now platform-aware (binary extensions, model paths)
- **Test data dirs** — `tts.rs`, `translate.rs`, `tts_cli.rs` test helpers resolve to `~/Library/Application Support/` on macOS

### Dependencies

- Added: `rodio 0.19`, `core-graphics 0.24` (macOS), `core-foundation 0.10` (macOS)
- GPU features now target-conditional: `llama-cpp-2/cuda` + `ort/cuda` + `ort/directml` on Windows, `llama-cpp-2/metal` + `ort/coreml` on macOS

## v0.2.1 (unreleased)

CI: cache Cargo registry/target, npm, and CUDA toolkit — cuts release builds from ~96min to ~15min.

## v0.2.0

OOB experience polish — smoother install-to-first-use journey.

### Added

- **Auto-updater** — checks for updates on launch, downloads and installs via NSIS (Tauri updater plugin with signed manifests)
- **Ready screen** — after model download, shows "You're all set" with hotkey reminder instead of vanishing
- **Start with Windows** — toggle in Settings > Startup, backed by tauri-plugin-autostart
- **Dynamic version** — Settings > About reads version from build config instead of hardcoded string
- **Tray tooltip** — shows "LinguaLens — Ctrl+Alt+L" (or current hotkey), updates live on hotkey change

### Fixed

- **Updater permissions** — added missing capability permissions for the updater plugin
- **CI manifest generation** — replaced broken heredoc with `jq --null-input` (heredoc indentation + EOF matching failed in YAML block scalars)
- **Clean shutdown** — `app.exit(0)` replaces `std::process::exit(0)`, runs destructors and releases CUDA/ONNX sessions properly
- **Model download paths** — Kokoro ONNX files moved to `onnx/` subdirectory upstream; updated HuggingFace paths
- **Download validation** — validate against HTTP Content-Length instead of hardcoded sizes (upstream voice files changed from 523KB to 522KB)

### Changed

- **Theme-aware download view** — download and ready screens use CSS variables (`--chrome-*`) with `prefers-color-scheme` media query, no longer hardcoded dark

## v0.1.0 (unreleased)

First release — ambient language learning overlay for Windows. Select text anywhere, get instant translation + IPA + TTS in a floating overlay.

### Architecture

- **Tauri 2 + Rust backend** — single binary, no runtime dependencies
- **All inference in-process** — TranslateGemma 4B (llama-cpp-2) + Kokoro 82M TTS (ort/ONNX Runtime)
- **GPU-accelerated** — CUDA → DirectML → CPU cascade for both translation and TTS
- **Vanilla JS frontend** — ~1000 lines across 6 files, no framework

### Features

- **Translation** — TranslateGemma 4B GGUF for phrase/sentence translation (55 language pairs), dictionary TSV fast-path for single words (Spanish↔English)
- **TTS** — Kokoro 82M ONNX with GPU cascade (CUDA ~30ms, DirectML ~80ms, CPU ~200ms), Web Speech API fallback
- **IPA transcription** — espeak-ng phonemization with punctuation re-injection
- **Text capture** — two-tier: UI Automation TextPattern (clipboard-free, ~80% of apps) → Ctrl+C simulation with sequence-number polling
- **Clipboard hygiene** — full clipboard save/restore, exclusion markers for Win+V and third-party clipboard managers
- **Language detection** — character signals → dictionary confidence → whatlang classifier
- **Settings panel** — target/native language, voice selection, auto-play, replay speed, theme (dark/light/system), IPA toggle, dismiss delay, hotkey capture
- **History** — SQLite-backed translation history with search and pagination
- **Multi-monitor** — overlay positions on the monitor containing the active window
- **System tray** — settings, debug tools, quit
- **Developer switches** — force CPU, force Web Speech, force dict-only, force clipboard capture (for testing degradation tiers)

### Development History

- **2026-03-05** — Initial implementation: Tauri + TranslateGemma + dictionary fast-path. Kokoro TTS integration (replaced browser-side opus-mt and Web Speech API with in-process Rust inference). 33 TTS tests passing.
- **2026-03-06** — Settings panel: configurable hotkey, theme toggle, voice selection, replay speed, IPA toggle, dismiss delay. History system with SQLite persistence, search, pagination.
- **2026-03-07** — Debug UI (test.html), voice browser, latency benchmarking. End-to-end testing and refinement.
- **2026-03-08** — Text capture upgrade: UIA TextPattern tier 1, clipboard sequence polling, full format preservation, clipboard manager exclusion markers.
- **2026-03-09** — Translation quality: output sanitization (strip preambles, markdown, language labels, quotes). Language detection refinement.
- **2026-03-10** — Developer switches for degradation tier testing. Graceful IPA fallback. Release preparation.
