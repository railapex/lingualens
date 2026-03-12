// Settings window — config management, voice selection, hotkey capture, history

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getVersion } from '@tauri-apps/api/app';
import { enable as enableAutostart, disable as disableAutostart } from '@tauri-apps/plugin-autostart';
import { playWavBytes, stopPlayback } from './audio.js';

// --- DOM refs ---

const tabs = document.querySelectorAll('.tab');
const tabContents = document.querySelectorAll('.tab-content');

const targetLangEl = document.getElementById('target-lang');
const nativeLangEl = document.getElementById('native-lang');
const voiceEl = document.getElementById('voice');
const autoPlayEl = document.getElementById('auto-play');
const replaySpeedEl = document.getElementById('replay-speed');
const replaySpeedVal = document.getElementById('replay-speed-val');
const themeEl = document.getElementById('theme');
const showIpaEl = document.getElementById('show-ipa');
const dismissDelayEl = document.getElementById('dismiss-delay');
const dismissDelayVal = document.getElementById('dismiss-delay-val');

const forceCpuEl = document.getElementById('force-cpu');
const forceWebSpeechEl = document.getElementById('force-web-speech');
const forceDictOnlyEl = document.getElementById('force-dict-only');
const forceClipboardEl = document.getElementById('force-clipboard');
const startAtLoginEl = document.getElementById('start-at-login');

const hotkeyDisplay = document.getElementById('hotkey-display');
const hotkeyChangeBtn = document.getElementById('hotkey-change');
const hotkeyCaptureEl = document.getElementById('hotkey-capture');
const hotkeyPreview = document.getElementById('hotkey-preview');
const hotkeySaveBtn = document.getElementById('hotkey-save');
const hotkeyCancelBtn = document.getElementById('hotkey-cancel');
const hotkeyError = document.getElementById('hotkey-error');

const historySearch = document.getElementById('history-search');
const historyList = document.getElementById('history-list');
const historyPagination = document.getElementById('history-pagination');
const historyPrev = document.getElementById('history-prev');
const historyNext = document.getElementById('history-next');
const historyPageInfo = document.getElementById('history-page-info');

// --- State ---

let config = {};
let allVoices = [];
let capturedHotkey = '';
let capturing = false;
let historyPage = 0;
const PAGE_SIZE = 50;

// --- Tabs ---

tabs.forEach(tab => {
  tab.addEventListener('click', () => {
    tabs.forEach(t => t.classList.remove('active'));
    tabContents.forEach(tc => tc.classList.remove('active'));
    tab.classList.add('active');
    document.getElementById(`tab-${tab.dataset.tab}`).classList.add('active');
    if (tab.dataset.tab === 'history') loadHistory();
  });
});

// --- Theme ---

function applyTheme(theme) {
  if (theme === 'system') {
    const dark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    document.body.setAttribute('data-theme', dark ? 'dark' : 'light');
  } else {
    document.body.setAttribute('data-theme', theme);
  }
}

window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
  if (config.theme === 'system') applyTheme('system');
});

// --- Config load/sync ---

async function loadConfig() {
  config = await invoke('get_config');
  populateControls();
  applyTheme(config.theme || 'system');
}

function populateControls() {
  targetLangEl.value = config.target_lang || 'es';
  nativeLangEl.value = config.native_lang || 'en';
  autoPlayEl.checked = config.auto_play !== false;
  replaySpeedEl.value = config.replay_speed ?? 0.7;
  replaySpeedVal.textContent = `${parseFloat(replaySpeedEl.value).toFixed(2)}x`;
  themeEl.value = config.theme || 'system';
  showIpaEl.checked = config.show_ipa !== false;
  dismissDelayEl.value = config.dismiss_delay_ms ?? 2000;
  dismissDelayVal.textContent = `${(parseInt(dismissDelayEl.value) / 1000).toFixed(1)}s`;
  hotkeyDisplay.textContent = formatHotkey(config.hotkey || 'ctrl+alt+l');

  // Startup
  startAtLoginEl.checked = config.start_at_login || false;

  // Dev switches
  forceCpuEl.checked = config.force_cpu || false;
  forceWebSpeechEl.checked = config.force_web_speech || false;
  forceDictOnlyEl.checked = config.force_dict_only || false;
  forceClipboardEl.checked = config.force_clipboard || false;

  // Voice dropdown — filter by target lang
  filterVoices(config.target_lang || 'es');
  if (config.tts_voice_target) {
    voiceEl.value = config.tts_voice_target;
  }
}

function formatHotkey(hotkey) {
  return hotkey.split('+').map(k =>
    k.charAt(0).toUpperCase() + k.slice(1)
  ).join('+');
}

async function updateConfig(updates) {
  config = await invoke('set_config', { updates });
}

// --- Voice dropdown ---

async function loadVoices() {
  try {
    allVoices = await invoke('list_voices');
  } catch (e) {
    console.warn('Failed to load voices:', e);
    allVoices = [];
  }
  filterVoices(config.target_lang || 'es');
}

function filterVoices(lang) {
  voiceEl.innerHTML = '<option value="">Default</option>';
  for (const v of allVoices.filter(v => v.lang === lang)) {
    const opt = document.createElement('option');
    opt.value = v.name;
    opt.textContent = `${v.name} (${v.gender})`;
    voiceEl.appendChild(opt);
  }
}

// --- Control handlers ---

targetLangEl.addEventListener('change', async () => {
  const newTarget = targetLangEl.value;
  // Swap if same as native
  if (newTarget === nativeLangEl.value) {
    nativeLangEl.value = config.target_lang;
    await updateConfig({ native_lang: config.target_lang });
  }
  await updateConfig({ target_lang: newTarget });
  filterVoices(newTarget);
  voiceEl.value = '';
  await updateConfig({ tts_voice_target: null });
});

nativeLangEl.addEventListener('change', async () => {
  const newNative = nativeLangEl.value;
  // Swap if same as target
  if (newNative === targetLangEl.value) {
    targetLangEl.value = config.native_lang;
    await updateConfig({ target_lang: config.native_lang });
    filterVoices(config.native_lang);
    voiceEl.value = '';
    await updateConfig({ tts_voice_target: null });
  }
  await updateConfig({ native_lang: newNative });
});

voiceEl.addEventListener('change', () => {
  const val = voiceEl.value || null;
  updateConfig({ tts_voice_target: val });
});

autoPlayEl.addEventListener('change', () => updateConfig({ auto_play: autoPlayEl.checked }));

replaySpeedEl.addEventListener('input', () => {
  replaySpeedVal.textContent = `${parseFloat(replaySpeedEl.value).toFixed(2)}x`;
});
replaySpeedEl.addEventListener('change', () => {
  updateConfig({ replay_speed: parseFloat(replaySpeedEl.value) });
});

themeEl.addEventListener('change', () => {
  updateConfig({ theme: themeEl.value });
  applyTheme(themeEl.value);
});

showIpaEl.addEventListener('change', () => updateConfig({ show_ipa: showIpaEl.checked }));

dismissDelayEl.addEventListener('input', () => {
  dismissDelayVal.textContent = `${(parseInt(dismissDelayEl.value) / 1000).toFixed(1)}s`;
});
dismissDelayEl.addEventListener('change', () => {
  updateConfig({ dismiss_delay_ms: parseInt(dismissDelayEl.value) });
});

// --- Dev switches ---

forceCpuEl.addEventListener('change', () => updateConfig({ force_cpu: forceCpuEl.checked }));
forceWebSpeechEl.addEventListener('change', () => updateConfig({ force_web_speech: forceWebSpeechEl.checked }));
forceDictOnlyEl.addEventListener('change', () => updateConfig({ force_dict_only: forceDictOnlyEl.checked }));
forceClipboardEl.addEventListener('change', () => updateConfig({ force_clipboard: forceClipboardEl.checked }));

// --- Autostart ---

startAtLoginEl.addEventListener('change', async () => {
  const enabled = startAtLoginEl.checked;
  try {
    if (enabled) await enableAutostart(); else await disableAutostart();
    await updateConfig({ start_at_login: enabled });
  } catch (e) {
    console.warn('Autostart toggle failed:', e);
    startAtLoginEl.checked = !enabled; // revert
  }
});

// --- Hotkey capture ---

hotkeyChangeBtn.addEventListener('click', async () => {
  try {
    await invoke('unregister_hotkey');
  } catch (e) {
    console.warn('Failed to unregister hotkey:', e);
  }
  capturing = true;
  capturedHotkey = '';
  hotkeyChangeBtn.style.display = 'none';
  hotkeyCaptureEl.style.display = 'block';
  hotkeyPreview.textContent = '';
  hotkeySaveBtn.disabled = true;
  hotkeyError.textContent = '';
});

document.addEventListener('keydown', (e) => {
  if (!capturing) return;
  e.preventDefault();
  e.stopPropagation();

  // Build modifier list
  const parts = [];
  if (e.ctrlKey) parts.push('ctrl');
  if (e.altKey) parts.push('alt');
  if (e.shiftKey) parts.push('shift');
  if (e.metaKey) parts.push('super');

  // Get the non-modifier key
  const key = e.key;
  const isModifier = ['Control', 'Alt', 'Shift', 'Meta'].includes(key);

  if (isModifier) {
    hotkeyPreview.textContent = parts.map(k => k.charAt(0).toUpperCase() + k.slice(1)).join('+') + '+...';
    hotkeySaveBtn.disabled = true;
    return;
  }

  if (parts.length === 0) {
    hotkeyError.textContent = 'At least one modifier (Ctrl, Alt, Shift) required';
    return;
  }

  // Normalize key name for Tauri
  let tauriKey = key.toLowerCase();
  if (key === ' ') tauriKey = 'space';
  if (key.length === 1) tauriKey = key.toLowerCase();

  parts.push(tauriKey);
  capturedHotkey = parts.join('+');
  hotkeyPreview.textContent = formatHotkey(capturedHotkey);
  hotkeySaveBtn.disabled = false;
  hotkeyError.textContent = '';
});

hotkeySaveBtn.addEventListener('click', async () => {
  if (!capturedHotkey) return;
  try {
    await invoke('update_hotkey', { newHotkey: capturedHotkey });
    hotkeyDisplay.textContent = formatHotkey(capturedHotkey);
    exitCapture();
  } catch (e) {
    hotkeyError.textContent = `Invalid shortcut: ${e}`;
    // Restore old hotkey
    try { await invoke('restore_hotkey'); } catch {}
    exitCapture();
  }
});

hotkeyCancelBtn.addEventListener('click', async () => {
  try { await invoke('restore_hotkey'); } catch {}
  exitCapture();
});

function exitCapture() {
  capturing = false;
  capturedHotkey = '';
  hotkeyCaptureEl.style.display = 'none';
  hotkeyChangeBtn.style.display = '';
}

// --- History ---

let searchDebounce = null;
historySearch.addEventListener('input', () => {
  clearTimeout(searchDebounce);
  searchDebounce = setTimeout(() => {
    historyPage = 0;
    loadHistory();
  }, 300);
});

historyPrev.addEventListener('click', () => {
  if (historyPage > 0) { historyPage--; loadHistory(); }
});

historyNext.addEventListener('click', () => {
  historyPage++;
  loadHistory();
});

async function loadHistory() {
  const search = historySearch.value.trim() || null;
  try {
    const [entries, total] = await Promise.all([
      invoke('get_history', { limit: PAGE_SIZE, offset: historyPage * PAGE_SIZE, search }),
      invoke('get_history_count', { search }),
    ]);

    if (entries.length === 0 && historyPage === 0) {
      historyList.innerHTML = '<div class="history-empty">No translations yet</div>';
      historyPagination.style.display = 'none';
      return;
    }

    historyList.innerHTML = '';
    for (const entry of entries) {
      historyList.appendChild(createHistoryEntry(entry));
    }

    // Pagination
    const totalPages = Math.ceil(total / PAGE_SIZE);
    if (totalPages > 1) {
      historyPagination.style.display = 'flex';
      historyPrev.disabled = historyPage === 0;
      historyNext.disabled = historyPage >= totalPages - 1;
      historyPageInfo.textContent = `${historyPage + 1} / ${totalPages}`;
    } else {
      historyPagination.style.display = 'none';
    }
  } catch (e) {
    console.warn('Failed to load history:', e);
    historyList.innerHTML = '<div class="history-empty">Failed to load history</div>';
  }
}

function createHistoryEntry(entry) {
  const el = document.createElement('div');
  el.className = 'history-entry';

  const time = new Date(entry.timestamp + 'Z').toLocaleString(undefined, {
    month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit',
  });

  el.innerHTML = `
    <div class="entry-texts">
      <div class="entry-source">${escapeHtml(entry.source_text)} &middot; ${entry.source_lang.toUpperCase()}</div>
      <div class="entry-target">${escapeHtml(entry.target_text)}</div>
    </div>
    <div class="entry-meta">
      <span class="entry-time">${time}</span>
      <span class="entry-method ${entry.method}">${entry.method}</span>
      <button class="entry-play" title="Replay">
        <svg viewBox="0 0 24 24"><path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02z"/></svg>
      </button>
    </div>
  `;

  el.querySelector('.entry-play').addEventListener('click', async () => {
    stopPlayback();
    try {
      const wavBytes = await invoke('speak', {
        text: entry.target_text,
        lang: entry.target_lang,
      });
      await playWavBytes(wavBytes);
    } catch (e) {
      console.warn('Replay failed:', e);
    }
  });

  return el;
}

function escapeHtml(text) {
  const div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}

// --- Config-changed listener (tray may change config while settings open) ---

listen('config-changed', (event) => {
  config = event.payload;
  populateControls();
  applyTheme(config.theme || 'system');
});

// --- Init ---

async function init() {
  await loadConfig();
  await loadVoices();

  // Dynamic version from build config
  try {
    const version = await getVersion();
    document.getElementById('app-version').textContent = `v${version}`;
  } catch {}
}

init();
