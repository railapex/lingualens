// Shared WAV playback — used by overlay (tts.js) and settings (history replay)

let audioCtx = null;
let currentSource = null;

export function getAudioCtx() {
  if (!audioCtx) audioCtx = new AudioContext();
  return audioCtx;
}

export async function playWavBytes(wavBytes) {
  const ctx = getAudioCtx();
  if (ctx.state === 'suspended') await ctx.resume();

  const buffer = new Uint8Array(wavBytes).buffer;
  const decoded = await ctx.decodeAudioData(buffer);

  return new Promise((resolve) => {
    const source = ctx.createBufferSource();
    source.buffer = decoded;
    source.connect(ctx.destination);
    source.onended = () => { currentSource = null; resolve(); };
    currentSource = source;
    source.start();
  });
}

export function stopPlayback() {
  if (currentSource) {
    try { currentSource.stop(); } catch {}
    currentSource = null;
  }
}
