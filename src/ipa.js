// IPA transcription via espeak-ng (Rust command → CLI)
import { invoke } from '@tauri-apps/api/core';

/**
 * Get IPA transcription for text in the given language.
 * @param {string} text
 * @param {string} lang - e.g. 'es', 'en'
 * @returns {Promise<string>} IPA string
 */
export async function getIPA(text, lang = 'es') {
  try {
    const ipa = await invoke('get_ipa', { text, lang });
    return ipa || '[no IPA]';
  } catch (e) {
    console.error('IPA error:', e);
    return '[IPA unavailable]';
  }
}
