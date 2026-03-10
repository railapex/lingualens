// TTS — Kokoro (Rust/GPU) primary, Web Speech API fallback

import { invoke } from '@tauri-apps/api/core';
import { playWavBytes, stopPlayback } from './audio.js';

// --- Web Speech API fallback ---

function getVoices() {
  return new Promise((resolve) => {
    const voices = speechSynthesis.getVoices();
    if (voices.length > 0) { resolve(voices); return; }
    const timeout = setTimeout(() => resolve([]), 3000);
    speechSynthesis.onvoiceschanged = () => {
      clearTimeout(timeout);
      resolve(speechSynthesis.getVoices());
    };
  });
}

async function findVoice(lang) {
  const voices = await getVoices();
  const prefix = lang.substring(0, 2).toLowerCase();
  const neural = voices.find(v =>
    v.lang.toLowerCase().startsWith(prefix) &&
    (v.name.includes('Neural') || v.name.includes('Natural') || v.name.includes('Online'))
  );
  return neural || voices.find(v => v.lang.toLowerCase().startsWith(prefix)) || null;
}

async function speakWebSpeech(text, lang) {
  speechSynthesis.cancel();
  const utterance = new SpeechSynthesisUtterance(text);
  utterance.lang = lang;
  utterance.rate = 0.9;
  const voice = await findVoice(lang);
  if (voice) utterance.voice = voice;
  return new Promise((resolve) => {
    utterance.onend = () => resolve();
    utterance.onerror = () => resolve();
    speechSynthesis.speak(utterance);
  });
}

// --- Public API ---

export async function speak(text, lang = 'es', { speed } = {}) {
  try {
    const wavBytes = await invoke('speak', { text, lang, speed: speed ?? null });
    return await playWavBytes(wavBytes);
  } catch (e) {
    console.warn('Kokoro TTS failed, falling back to Web Speech:', e);
    return speakWebSpeech(text, lang);
  }
}

export function stop() {
  stopPlayback();
  speechSynthesis.cancel();
}

export async function preload() {
  // Kokoro initializes lazily on first speak() call in Rust
  // Warm it up with a silent call to check status
  try {
    await invoke('get_tts_status');
  } catch (e) {
    console.warn('TTS status check failed:', e);
  }
}
