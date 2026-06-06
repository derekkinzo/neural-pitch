// Playback-head broadcast — process-global ref + subscribe for the rAF
// hot path between PlaybackPanel (publisher) and ContourLine (consumer).
//
// Phase 2.4 hot-path contract: wavesurfer's `audioprocess` event fires
// at ~50 Hz; routing every tick through Zustand would dwarf
// ContourLine's static-paint budget. Mirroring the RingBuffer pattern
// used by HistoryStrip, we publish into a process-global ref and notify
// rAF subscribers — React state stays untouched.

export interface PlaybackHead {
  /** Current playback time, in milliseconds. */
  tMs: number;
  /** Whether the audio element is currently playing. */
  isPlaying: boolean;
}

/** Process-global ref. Reads happen INSIDE rAF callbacks; writes happen
 *  in `publish()` below. The object identity is stable across the
 *  module's lifetime so subscribers can hold a reference. */
export const playbackHeadRef: { current: PlaybackHead } = {
  current: { tMs: 0, isPlaying: false },
};

type Subscriber = (head: PlaybackHead) => void;
const subscribers = new Set<Subscriber>();

/** Subscribe to publish notifications. The subscriber MUST itself drive
 *  the rAF loop; this module only fans out the "data changed" signal.
 *  Returns a teardown closure. */
export function subscribe(fn: Subscriber): () => void {
  subscribers.add(fn);
  return () => {
    subscribers.delete(fn);
  };
}

/** Notify every subscriber, isolating exceptions so a single broken
 *  subscriber (e.g. paint into a detached canvas during teardown) does
 *  not break the broadcast for the rest. We iterate a snapshot of the
 *  Set so an unsubscribe inside a callback cannot mutate the live
 *  iteration. */
function notifyAll(head: PlaybackHead): void {
  for (const fn of Array.from(subscribers)) {
    try {
      fn(head);
    } catch (err: unknown) {
      if (typeof console !== "undefined") {
        console.warn("playback-head subscriber threw", err);
      }
    }
  }
}

/** Write the latest playback time and notify subscribers. Caller passes
 *  `tMs` and (optionally) the current play/pause state; the second
 *  argument lets the publisher avoid a separate setter for the boolean. */
export function publishTime(tMs: number, isPlaying?: boolean): void {
  playbackHeadRef.current = {
    tMs,
    isPlaying: isPlaying ?? playbackHeadRef.current.isPlaying,
  };
  notifyAll(playbackHeadRef.current);
}

/** Update only the play/pause flag and notify. */
export function publishIsPlaying(isPlaying: boolean): void {
  playbackHeadRef.current = { ...playbackHeadRef.current, isPlaying };
  notifyAll(playbackHeadRef.current);
}

/** Reset to a parked head — used when the recording selection changes
 *  or the recording reaches its end. */
export function resetPlaybackHead(tMs = 0): void {
  playbackHeadRef.current = { tMs, isPlaying: false };
  notifyAll(playbackHeadRef.current);
}
