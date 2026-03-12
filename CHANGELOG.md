# Changelog

## v0.2.0 (unreleased)

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
