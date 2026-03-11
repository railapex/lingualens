import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getIPA } from './ipa.js';
import { speak, stop as stopTTS, preload as preloadTTS } from './tts.js';
import { normalizeAccents } from './lang.js';

const overlay = document.getElementById('overlay');
const heroEl = document.getElementById('hero');
const ipaEl = document.getElementById('ipa');
const l1El = document.getElementById('l1');
const speakerBtn = document.getElementById('speaker');
const translateView = document.getElementById('translate-view');
const pickerView = document.getElementById('picker');

const appWindow = getCurrentWindow();
let dismissTimer = null;
let hideTimer = null;
let lastText = '';
let lastLang = '';
let lastSpeakText = '';
let lastSpeakLang = '';
let invocationId = 0;
let overlayVisible = false;

// Config — loaded from Rust on init, updated via config-changed events
let targetLang = 'es';
let nativeLang = 'en';
let config = {};

const MAX_LENGTH = 120;
const SHIMMER = '<span class="shimmer"></span>';

function formatHotkey(hotkey) {
  return hotkey.split('+').map(k => k.charAt(0).toUpperCase() + k.slice(1)).join('+');
}

// --- Config ---

async function loadConfig() {
  config = await invoke('get_config');
  targetLang = config.target_lang || 'es';
  nativeLang = config.native_lang || 'en';
  applyTheme(config.theme || 'system');
}

function applyTheme(theme) {
  if (theme === 'system') {
    const dark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    overlay.setAttribute('data-theme', dark ? 'dark' : 'light');
  } else {
    overlay.setAttribute('data-theme', theme);
  }
}

// Listen for system theme changes when in "system" mode
window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
  if (config.theme === 'system') {
    applyTheme('system');
  }
});

// --- Input validation ---

const URL_RE = /^https?:\/\/|^www\./i;
const JUNK_RE = /^[\d\s\-+().,:;@#$%^&*=\\/<>{}\[\]|~`]+$/;

function isUsableText(text) {
  if (!text || text.length < 2) return false;
  if (text.length > 500) return false;
  if (URL_RE.test(text)) return false;
  if (JUNK_RE.test(text)) return false;
  if (!/[a-z\u00C0-\u024F\u3040-\u309F\u30A0-\u30FF\u4E00-\u9FFF]/i.test(text)) return false;
  return true;
}

function truncate(text) {
  if (text.length <= MAX_LENGTH) return text;
  const cut = text.slice(0, MAX_LENGTH);
  const lastSpace = cut.lastIndexOf(' ');
  return (lastSpace > MAX_LENGTH * 0.6 ? cut.slice(0, lastSpace) : cut) + '\u2026';
}

// --- Overlay lifecycle ---

async function showOverlay(text) {
  text = truncate(text);
  const lang = await invoke('detect_language', { text });
  lastText = text;
  lastLang = lang;
  const thisInvocation = ++invocationId;

  // Normalize accents for Spanish text
  const shouldNormalize = (lang === 'es' || targetLang === 'es');
  const normalizedText = shouldNormalize ? normalizeAccents(text) : text;

  // Cancel any pending hide from previous dismissal
  if (hideTimer) { clearTimeout(hideTimer); hideTimer = null; }

  // Compact mode for short text
  overlay.classList.toggle('compact', normalizedText.length < 20);

  overlay.classList.remove('hidden');
  overlayVisible = true;

  if (dismissTimer) clearTimeout(dismissTimer);

  const isTargetLang = (lang === targetLang);

  if (isTargetLang) {
    // --- Source IS the target language ---
    // Hero = source text (immediate), IPA + translation in parallel
    heroEl.textContent = normalizedText;
    ipaEl.innerHTML = SHIMMER;
    l1El.innerHTML = SHIMMER;

    const ipaPromise = config.show_ipa !== false
      ? getIPA(normalizedText, lang).then(ipa => {
          if (thisInvocation === invocationId) ipaEl.textContent = ipa;
        }).catch(() => {
          if (thisInvocation === invocationId) ipaEl.textContent = '';
        })
      : Promise.resolve().then(() => { ipaEl.textContent = ''; });

    let translatedText = '';
    const translatePromise = invoke('translate_text', {
      text: normalizedText,
      sourceLang: lang,
      targetLang: nativeLang,
    }).then(result => {
      translatedText = result;
      if (thisInvocation === invocationId) l1El.textContent = result;
    }).catch((e) => {
      if (thisInvocation === invocationId) l1El.textContent = '';
      console.warn('Translation failed:', e);
    });

    await Promise.all([ipaPromise, translatePromise]);

    if (thisInvocation !== invocationId) return;

    // TTS: speak the target language text
    lastSpeakText = normalizedText;
    lastSpeakLang = lang;
    if (config.auto_play !== false) {
      speakAndDismiss(normalizedText, lang, thisInvocation);
    } else {
      scheduleDismiss();
    }

  } else {
    // --- Source is NOT the target language ---
    // L1 = source text (immediate), hero + IPA wait for translation
    heroEl.innerHTML = SHIMMER;
    ipaEl.innerHTML = SHIMMER;

    // Show source with language tag if not native
    if (lang !== nativeLang) {
      l1El.textContent = `${normalizedText} \u00B7 ${lang.toUpperCase()}`;
    } else {
      l1El.textContent = normalizedText;
    }

    // Translate to target language
    let heroText = '';
    try {
      heroText = await invoke('translate_text', {
        text: normalizedText,
        sourceLang: lang,
        targetLang: targetLang,
      });
      if (thisInvocation !== invocationId) return;
      heroEl.textContent = heroText;
    } catch (e) {
      if (thisInvocation !== invocationId) return;
      heroEl.textContent = '[translation unavailable]';
      console.warn('Translation failed:', e);
    }

    // IPA of the translated text (serial — depends on translation)
    if (heroText && config.show_ipa !== false) {
      try {
        const ipa = await getIPA(heroText, targetLang);
        if (thisInvocation === invocationId) ipaEl.textContent = ipa;
      } catch {
        if (thisInvocation === invocationId) ipaEl.textContent = '';
      }
    } else {
      ipaEl.textContent = '';
    }

    if (thisInvocation !== invocationId) return;

    // TTS: speak the target language translation
    lastSpeakText = heroText || normalizedText;
    lastSpeakLang = heroText ? targetLang : lang;
    if (heroText && config.auto_play !== false) {
      speakAndDismiss(heroText, targetLang, thisInvocation);
    } else {
      scheduleDismiss();
    }
  }

  // Fallback dismiss
  if (!dismissTimer) {
    dismissTimer = setTimeout(hideOverlay, 12000);
  }
}

function speakAndDismiss(text, lang, thisInvocation) {
  speakerBtn.classList.add('playing');
  speak(text, lang).then(() => {
    speakerBtn.classList.remove('playing');
    if (thisInvocation === invocationId) {
      scheduleDismiss();
    }
  }).catch(() => {
    speakerBtn.classList.remove('playing');
  });
}

function scheduleDismiss() {
  const delay = config.dismiss_delay_ms || 2000;
  if (dismissTimer) clearTimeout(dismissTimer);
  dismissTimer = setTimeout(hideOverlay, delay);
}

async function hideOverlay() {
  stopTTS();
  speakerBtn.classList.remove('playing');
  overlay.classList.add('hidden');
  overlayVisible = false;
  if (dismissTimer) {
    clearTimeout(dismissTimer);
    dismissTimer = null;
  }
  if (hideTimer) clearTimeout(hideTimer);
  hideTimer = setTimeout(async () => {
    await appWindow.hide();
    hideTimer = null;
  }, 180);
}

async function replaySlower() {
  if (!lastSpeakText) return;
  stopTTS();
  speakerBtn.classList.remove('playing');
  if (dismissTimer) clearTimeout(dismissTimer);
  const speed = config.replay_speed || 0.7;
  speakerBtn.classList.add('playing');
  speak(lastSpeakText, lastSpeakLang, { speed }).then(() => {
    speakerBtn.classList.remove('playing');
    scheduleDismiss();
  }).catch(() => {
    speakerBtn.classList.remove('playing');
  });
}

async function showEmpty(text) {
  if (hideTimer) { clearTimeout(hideTimer); hideTimer = null; }
  heroEl.textContent = text
    ? 'Not translatable'
    : 'No text selected';
  ipaEl.textContent = '';
  l1El.textContent = text
    ? 'Select a word or phrase to translate'
    : `Highlight text and press ${formatHotkey(config.hotkey || 'ctrl+alt+l')}`;
  overlay.classList.remove('hidden');
  overlay.classList.remove('compact');
  overlayVisible = true;

  if (dismissTimer) clearTimeout(dismissTimer);
  dismissTimer = setTimeout(hideOverlay, 2000);
}

// --- First-run picker ---

async function showPicker() {
  if (hideTimer) { clearTimeout(hideTimer); hideTimer = null; }
  translateView.style.display = 'none';
  pickerView.style.display = 'block';
  overlay.classList.remove('hidden');
  overlay.classList.remove('compact');
  overlayVisible = true;
}

function hidePicker() {
  pickerView.style.display = 'none';
  translateView.style.display = 'block';
}

// --- Event handlers ---

speakerBtn.addEventListener('click', (e) => {
  e.stopPropagation();
  if (lastSpeakText) {
    speakerBtn.classList.add('playing');
    speak(lastSpeakText, lastSpeakLang).then(() => {
      speakerBtn.classList.remove('playing');
    }).catch(() => {
      speakerBtn.classList.remove('playing');
    });
  }
});

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') hideOverlay();
});

// Dismiss when window loses focus
appWindow.onFocusChanged(({ payload: focused }) => {
  if (!focused && overlayVisible) hideOverlay();
});

// Pause auto-dismiss while mouse is over the overlay
overlay.addEventListener('mouseenter', () => {
  if (dismissTimer) { clearTimeout(dismissTimer); dismissTimer = null; }
});

overlay.addEventListener('mouseleave', () => {
  if (overlayVisible) {
    scheduleDismiss();
  }
});

// Picker button handlers
pickerView.addEventListener('click', async (e) => {
  const btn = e.target.closest('.lang-option');
  if (!btn) return;
  const lang = btn.dataset.lang;
  await invoke('set_config', { updates: { target_lang: lang } });
  await loadConfig();
  hidePicker();

  // Show confirmation
  heroEl.textContent = lang === 'es' ? 'Bueno.' : 'Got it.';
  ipaEl.textContent = '';
  l1El.textContent = `Highlight text and press ${formatHotkey(config.hotkey || 'ctrl+alt+l')}`;

  if (dismissTimer) clearTimeout(dismissTimer);
  dismissTimer = setTimeout(hideOverlay, 2500);
});

// --- Model download ---

const downloadView = document.getElementById('download-view');
const downloadBar = document.getElementById('download-bar');
const downloadModelName = document.getElementById('download-model-name');
const downloadStats = document.getElementById('download-stats');
const downloadSkip = document.getElementById('download-skip');
const downloadRetry = document.getElementById('download-retry');

function formatBytes(bytes) {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(0)} MB`;
  return `${(bytes / 1e3).toFixed(0)} KB`;
}

async function checkAndDownloadModels() {
  const missing = await invoke('check_models');
  if (missing.length === 0) return true; // all models present

  const totalBytes = missing.reduce((s, m) => s + m.size_bytes, 0);
  console.log(`[lingualens] ${missing.length} models missing (${formatBytes(totalBytes)})`);

  // Show download UI
  downloadView.style.display = 'block';
  downloadModelName.textContent = `${missing.length} files \u00B7 ${formatBytes(totalBytes)}`;
  await appWindow.show();
  await appWindow.setFocus();

  return new Promise((resolve) => {
    let unlistenProgress = null;
    let unlistenComplete = null;

    function cleanup() {
      if (unlistenProgress) unlistenProgress.then(fn => fn());
      if (unlistenComplete) unlistenComplete.then(fn => fn());
      unlistenProgress = null;
      unlistenComplete = null;
    }

    // Skip button — proceed without models
    downloadSkip.addEventListener('click', () => {
      invoke('cancel_download').catch(() => {});
      cleanup();
      downloadView.style.display = 'none';
      appWindow.hide();
      resolve(false);
    }, { once: true });

    // Progress listener
    unlistenProgress = listen('download-progress', (event) => {
      const p = event.payload;
      const pct = p.overall_bytes_total > 0
        ? ((p.overall_bytes_downloaded / p.overall_bytes_total) * 100)
        : 0;
      downloadBar.style.width = `${pct.toFixed(1)}%`;
      downloadModelName.textContent = p.name;
      downloadStats.textContent = `${formatBytes(p.overall_bytes_downloaded)} / ${formatBytes(p.overall_bytes_total)} \u00B7 ${pct.toFixed(0)}%`;
    });

    // Complete listener
    unlistenComplete = listen('download-complete', () => {
      cleanup();
      downloadView.style.display = 'none';
      appWindow.hide();
      // Trigger model preloading now that files exist
      invoke('preload_models').catch(() => {});
      resolve(true);
    });

    function startDownload() {
      downloadModelName.textContent = 'Starting download...';
      downloadBar.style.width = '0%';
      downloadRetry.style.display = 'none';

      invoke('start_download').catch((e) => {
        console.warn('[lingualens] download error:', e);
        downloadModelName.textContent = 'Download failed';
        downloadStats.textContent = String(e);
        downloadRetry.style.display = '';
      });
    }

    // Retry button
    downloadRetry.addEventListener('click', startDownload);

    startDownload();
  });
}

// --- Init ---

let firstRun = false;

async function init() {
  console.log('[lingualens] init()');

  await loadConfig();

  // Check for missing models and download if needed
  await checkAndDownloadModels();

  firstRun = await invoke('is_first_run');

  // Listen for config changes (from tray menu or settings)
  await listen('config-changed', async (event) => {
    config = event.payload;
    targetLang = config.target_lang || 'es';
    nativeLang = config.native_lang || 'en';
    applyTheme(config.theme || 'system');
    console.log('[lingualens] config updated:', targetLang, nativeLang);
  });

  await listen('hotkey-text', async (event) => {
    const text = (event.payload || '').trim();
    console.log('[lingualens] hotkey text:', text?.substring(0, 50));

    // First run: show picker
    if (firstRun) {
      showPicker();
      firstRun = false; // only show once per session
      return;
    }

    // If overlay is actively showing content, replay at slower speed
    if (overlayVisible && lastText) {
      console.log('[lingualens] replay slower');
      replaySlower();
      return;
    }

    if (text && isUsableText(text)) {
      showOverlay(text);
    } else {
      showEmpty(text);
    }
  });
}

init();
preloadTTS();
