// Drill prompt synthesizer.
//
// The Phase 4 drills synthesise short sine prompts in the page itself —
// either two notes (intervals), three or four (chords), or a seven-note
// ascending pattern (scales). A real Tauri shell could ship pre-rendered
// WAV blobs from Rust, but the WebAudio path is universally available and
// keeps the IPC surface frozen (no new audio commands).
//
// Test-bridge contract: every prompt-play increments
// `__neuralPitchTestHooks.audioPlayCount` (top-level for the spec) and
// also `__neuralPitchTestHooks.training.audioPlayCount` (forward-compat
// with future drill-specific harness slots).
//
// AudioContext lifecycle: a single lazy module-level context is reused
// across all prompts. Chrome typically caps live AudioContexts at 6 —
// constructing a fresh one per `playDrillPrompt` (the previous shape)
// silently leaked a context every prompt and broke playback after 6
// drills. `closeDrillSynth()` is exposed so tests + drill unmount can
// drop the singleton.
//

import { midiToHz } from "@/lib/note-format";
import { getTestHooks } from "@/lib/test-hooks";

const FALLBACK_A4_HZ = 440;

interface TrainingHooksSlot {
  audioPlayCount?: number;
  [extra: string]: unknown;
}

interface HooksWithCounters {
  audioPlayCount?: number;
  training?: TrainingHooksSlot;
}

function bumpAudioPlayCount(): void {
  const hooks = getTestHooks() as (ReturnType<typeof getTestHooks> & HooksWithCounters) | undefined;
  if (hooks === undefined) return;
  hooks.audioPlayCount = (hooks.audioPlayCount ?? 0) + 1;
  const slot: TrainingHooksSlot = hooks.training ?? {};
  slot.audioPlayCount = (slot.audioPlayCount ?? 0) + 1;
  hooks.training = slot;
}

interface AudioContextLike {
  readonly currentTime: number;
  readonly destination: AudioNode;
  readonly state?: string;
  createOscillator: () => OscillatorNode;
  createGain: () => GainNode;
  resume?: () => Promise<void>;
  close?: () => Promise<void>;
}

let cachedContext: AudioContextLike | null = null;

function lazyAudioContext(): AudioContextLike | null {
  if (cachedContext !== null) return cachedContext;
  if (typeof window === "undefined") return null;
  type WindowWithAudio = Window & {
    AudioContext?: typeof AudioContext;
    webkitAudioContext?: typeof AudioContext;
  };
  const w = window as WindowWithAudio;
  const Ctor = w.AudioContext ?? w.webkitAudioContext;
  if (Ctor === undefined) return null;
  try {
    cachedContext = new Ctor();
  } catch {
    cachedContext = null;
  }
  return cachedContext;
}

/** Drop the cached AudioContext. Tests call this in `afterEach` so a
 *  spec that exercised playback does not leak an open context into the
 *  next spec. Production code can call this on Training-screen unmount
 *  to release the OS audio handle. */
export function closeDrillSynth(): void {
  const ctx = cachedContext;
  cachedContext = null;
  if (ctx?.close !== undefined) {
    void ctx.close().catch(() => {
      /* swallow: closing a context that already errored is fine */
    });
  }
}

/** Schedule one sine note `frequencyHz` from `tStart` for `durationS`. */
function scheduleNote(
  ctx: AudioContextLike,
  frequencyHz: number,
  tStart: number,
  durationS: number,
  gain: number,
): void {
  const osc = ctx.createOscillator();
  const env = ctx.createGain();
  osc.frequency.value = frequencyHz;
  osc.type = "sine";
  osc.connect(env);
  env.connect(ctx.destination);
  // Linear ramp envelope — short fade-in / fade-out so the prompt does
  // not click on start / stop.
  const attack = 0.01;
  const release = 0.05;
  env.gain.setValueAtTime(0, tStart);
  env.gain.linearRampToValueAtTime(gain, tStart + attack);
  env.gain.linearRampToValueAtTime(gain, tStart + durationS - release);
  env.gain.linearRampToValueAtTime(0, tStart + durationS);
  osc.start(tStart);
  osc.stop(tStart + durationS + 0.01);
}

export interface PlayPromptOptions {
  /** MIDI notes to render. */
  readonly midiNotes: readonly number[];
  /** A4 reference Hz. Defaults to 440. */
  readonly a4Hz?: number;
  /** Per-note duration in seconds. Defaults to 0.6. */
  readonly noteDurationS?: number;
  /** Gap between successive notes (sequential mode), seconds. Defaults to 0. */
  readonly gapS?: number;
  /** "sequential" plays notes one after another; "parallel" plays them all
   *  at once (chord mode). Defaults to "sequential". */
  readonly mode?: "sequential" | "parallel";
}

/**
 * Synthesise and play a short prompt. Always increments the test-hook
 * audio-play counter, even when WebAudio is unavailable — the harness
 * relies on the counter as a black-box contract independent of the
 * underlying playback path.
 */
export function playDrillPrompt(opts: PlayPromptOptions): void {
  bumpAudioPlayCount();
  const ctx = lazyAudioContext();
  if (ctx === null) return;
  if (ctx.resume !== undefined) {
    void ctx.resume();
  }
  const a4 = opts.a4Hz ?? FALLBACK_A4_HZ;
  const noteDuration = opts.noteDurationS ?? 0.6;
  const gap = opts.gapS ?? 0;
  const mode = opts.mode ?? "sequential";
  const t0 = ctx.currentTime + 0.05;
  if (mode === "parallel") {
    for (const midi of opts.midiNotes) {
      scheduleNote(ctx, midiToHz(midi, a4), t0, noteDuration, 0.18);
    }
  } else {
    let cursor = t0;
    for (const midi of opts.midiNotes) {
      scheduleNote(ctx, midiToHz(midi, a4), cursor, noteDuration, 0.22);
      cursor += noteDuration + gap;
    }
  }
}
