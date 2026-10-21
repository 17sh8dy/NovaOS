import type { SettingsData, SoundPack } from './schema';

export type SoundEvent = 'open' | 'close' | 'minimize' | 'restore' | 'click';

export interface Note {
  /** Hz */
  freq: number;
  /** seconds from the start of the sound */
  at: number;
  /** seconds */
  dur: number;
  gain: number;
}

type Wave = OscillatorType;

const PACK_WAVE: Record<SoundPack, Wave> = {
  soft: 'sine',
  click: 'triangle',
  chime: 'sine',
  retro: 'square',
};

/** Pitches per event; each pack plays them with its own waveform and feel. */
const BASE: Record<SoundEvent, number[]> = {
  open: [523.25, 659.25],
  close: [659.25, 440],
  minimize: [392, 330],
  restore: [330, 440],
  click: [880],
};

/** The notes for an event, without touching the audio device. Pure, so it is testable. */
export function soundPlan(event: SoundEvent, pack: SoundPack): Note[] {
  const pitches = BASE[event];
  const step = pack === 'chime' ? 0.11 : pack === 'retro' ? 0.07 : 0.06;
  const dur = event === 'click' ? 0.035 : pack === 'chime' ? 0.22 : pack === 'retro' ? 0.08 : 0.12;
  const level = pack === 'retro' ? 0.5 : pack === 'click' ? 0.9 : 0.8;
  return pitches.map((freq, i) => ({ freq, at: i * step, dur, gain: level / (i + 1) ** 0.3 }));
}

let ctx: AudioContext | null = null;

function audio(): AudioContext | null {
  if (typeof window === 'undefined') return null;
  const Ctor =
    window.AudioContext ??
    (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!Ctor) return null;
  try {
    ctx ??= new Ctor();
    if (ctx.state === 'suspended') void ctx.resume();
    return ctx;
  } catch {
    return null;
  }
}

/** Whether this event should sound at all under the current settings. */
export function shouldPlay(event: SoundEvent, s: SettingsData['sounds']): boolean {
  return s.enabled && s.volume > 0 && (event !== 'click' || s.clicks);
}

/** Synthesizes a short sound. Silent (and harmless) if sounds are off or audio is unavailable. */
export function playSound(event: SoundEvent, s: SettingsData['sounds']): boolean {
  if (!shouldPlay(event, s)) return false;
  const ac = audio();
  if (!ac) return false;
  const wave = PACK_WAVE[s.pack];
  const t0 = ac.currentTime + 0.005;
  for (const n of soundPlan(event, s.pack)) {
    const osc = ac.createOscillator();
    const gain = ac.createGain();
    osc.type = wave;
    osc.frequency.value = n.freq;
    const peak = 0.18 * s.volume * n.gain;
    gain.gain.setValueAtTime(0.0001, t0 + n.at);
    gain.gain.exponentialRampToValueAtTime(Math.max(peak, 0.0002), t0 + n.at + 0.01);
    gain.gain.exponentialRampToValueAtTime(0.0001, t0 + n.at + n.dur);
    osc.connect(gain).connect(ac.destination);
    osc.start(t0 + n.at);
    osc.stop(t0 + n.at + n.dur + 0.02);
  }
  return true;
}
