// Recordings store — slow-path Zustand state for the recorder.
//
// Per the RingBuffer contract, hot-path frames
// (`PitchUpdate`) DO NOT pass through Zustand. Recording state is intrinsically
// slow (≤5 Hz progress ticks, single-take lifecycle, drawer mounts) so the
// store is the right home for the elapsed-counter, list, and current-recording
// selection.
//
// IPC surface (mirrors INTEGRATION-SPEC §2):
//   - `start_recording({ instrumentProfile, note? })` -> { recordingId }
//   - `stop_recording()`                              -> Recording
//   - `list_recordings()`                             -> Recording[]
//   - `delete_recording({ id })`                      -> null
//   - `rename_recording({ id, label })`               -> null
//
// The `recording-progress` event channel emits `RecordingProgress` payloads
// at ~5 Hz; we register a single listener that writes only `elapsedMs`
// so subscribers reading `(s) => s.elapsedMs` re-render in isolation.
//

import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { formatDurationToast } from "@/lib/duration-format";
import type { Recording, RecordingId, RecordingProgress } from "@/types/recording";

/** Snake_case wire-format mirroring the Rust serde output. The Rust shell
 *  may emit either form; both are accepted at the IPC boundary. */
interface WireRecording {
  id?: string;
  filename?: string;
  created_at?: number;
  createdAt?: number;
  duration_ms?: number;
  durationMs?: number;
  sample_rate_hz?: number;
  sampleRateHz?: number;
  channels?: number;
  bit_depth?: number;
  bitDepth?: number;
  a4_hz?: number;
  a4Hz?: number;
  instrument_profile?: string;
  instrumentProfile?: string;
  user_label?: string | null;
  userLabel?: string;
}

interface WireStartResponse {
  recording_id?: string;
  recordingId?: string;
}

function normaliseRecording(raw: WireRecording): Recording {
  const id = raw.id ?? "";
  const filename = raw.filename ?? `${id}.flac`;
  const createdAt = raw.createdAt ?? raw.created_at ?? Date.now();
  const durationMs = raw.durationMs ?? raw.duration_ms ?? 0;
  const sampleRateHz = raw.sampleRateHz ?? raw.sample_rate_hz ?? 48000;
  const channels = raw.channels ?? 1;
  const bitDepth = raw.bitDepth ?? raw.bit_depth ?? 24;
  const a4Hz = raw.a4Hz ?? raw.a4_hz ?? 440;
  const instrumentProfile = raw.instrumentProfile ?? raw.instrument_profile ?? "Generic";
  const userLabelRaw = raw.userLabel ?? raw.user_label ?? null;
  const userLabel = typeof userLabelRaw === "string" ? userLabelRaw : undefined;
  const base = {
    id,
    filename,
    createdAt,
    durationMs,
    sampleRateHz,
    channels,
    bitDepth,
    a4Hz,
    instrumentProfile,
  } as const;
  return userLabel === undefined ? base : { ...base, userLabel };
}

function sortDescByCreatedAt(items: readonly Recording[]): Recording[] {
  return items.slice().sort((a, b) => b.createdAt - a.createdAt);
}

export interface RecordingsState {
  /** Persisted takes, sorted descending by `createdAt`. */
  items: readonly Recording[];
  /** True between `start_recording` resolving and `stop_recording` resolving. */
  isRecording: boolean;
  /** True between `start_recording` invoke and its resolution. The button
   *  reads `data-state="saving"` while this is true so the visual affordance
   *  matches stop-time behaviour; `stopRecording` short-circuits while
   *  starting is true to prevent a Stop-during-Start race. */
  starting: boolean;
  /** Selection anchor for the PlaybackPanel. `null` collapses the panel. */
  currentRecordingId: RecordingId | null;
  /** Live elapsed-time tick driven by the `recording-progress` channel. */
  elapsedMs: number;
  /** True between `stop_recording` invoke and its resolution. */
  saving: boolean;
  /** Last error surfaced by start/stop. `null` clears the toast. */
  lastError: string | null;
  /** Transient toast text — set on stop, cleared by `dismissSavedToast`. */
  savedToastMessage: string | null;
  /** Slow-path mirror of the playback head — written ~30 Hz from
   *  PlaybackPanel's rAF-throttled publisher. Drives the `mm:ss` readout
   *  next to the transport controls. The hot path (ContourLine) reads
   *  `playbackHeadRef` directly and never re-renders for this. */
  playbackTimeMs: number;
  /** Reflects wavesurfer's `play` / `pause` / `finish` events. */
  isPlaying: boolean;
  /** Spectrogram visibility — false by default so the heavy plugin import
   *  stays out of first paint. */
  showSpectrogram: boolean;
  /** In-memory mapping from recording id to last-known playback position.
   *  Lives only for the session; no IPC, no localStorage. */
  persistedPlaybackPositionByRecording: ReadonlyMap<RecordingId, number>;
}

export interface RecordingsActions {
  refresh: () => Promise<void>;
  startRecording: (instrumentProfile: string) => Promise<void>;
  stopRecording: () => Promise<void>;
  select: (id: RecordingId | null) => void;
  applyProgress: (p: RecordingProgress) => void;
  setError: (msg: string | null) => void;
  dismissSavedToast: () => void;
  /** Throttled writer (~30 Hz) for the visible mm:ss readout. */
  setPlaybackTimeMs: (ms: number) => void;
  setIsPlaying: (v: boolean) => void;
  toggleSpectrogram: () => void;
  /** Test-only helper to seed deterministic state. Production code paths
   *  go through the IPC actions. */
  __setItemsForTest: (items: readonly Recording[]) => void;
}

export type RecordingsStore = RecordingsState & RecordingsActions;

export const useRecordingsStore = create<RecordingsStore>((set, get) => ({
  items: [],
  isRecording: false,
  starting: false,
  currentRecordingId: null,
  elapsedMs: 0,
  saving: false,
  lastError: null,
  savedToastMessage: null,
  playbackTimeMs: 0,
  isPlaying: false,
  showSpectrogram: false,
  persistedPlaybackPositionByRecording: new Map<RecordingId, number>(),

  refresh: async (): Promise<void> => {
    try {
      const raw = await invoke<WireRecording[] | null>("list_recordings");
      const list = Array.isArray(raw) ? raw.map(normaliseRecording) : [];
      set({ items: sortDescByCreatedAt(list) });
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      set({ lastError: msg });
    }
  },

  startRecording: async (instrumentProfile: string): Promise<void> => {
    if (get().isRecording || get().saving || get().starting) return;
    // Set `starting=true` before the IPC fires so a Stop click while a
    // Start is in flight is rejected by the `!get().starting` guard in
    // `stopRecording`. The button reads `data-state="saving"` while
    // `starting` is true (the same affordance used during stop), so the
    // UX cue is preserved without flipping `isRecording` optimistically.
    set({ starting: true, elapsedMs: 0, lastError: null });
    try {
      await invoke<WireStartResponse | null>("start_recording", { instrumentProfile });
      // Flip `isRecording` only after start_recording resolves. After this
      // point Stop is meaningful — the encoder thread is attached and the
      // DSP fan-out is wired.
      set({ starting: false, isRecording: true });
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      set({ starting: false, isRecording: false, lastError: msg });
    }
  },

  stopRecording: async (): Promise<void> => {
    // Hard-reject Stop while a start IPC is in flight — see
    // `startRecording` for the race this guards against.
    if (!get().isRecording || get().starting) return;
    set({ saving: true });
    try {
      const raw = await invoke<WireRecording | null>("stop_recording");
      const final = raw !== null && raw !== undefined ? normaliseRecording(raw) : null;
      // Refresh the list so the new row is visible. The mock handler also
      // mutates a shared array, so this re-read is what surfaces the row.
      await get().refresh();
      const message =
        final !== null
          ? `Saved ${formatDurationToast(final.durationMs)} recording`
          : `Saved ${formatDurationToast(get().elapsedMs)} recording`;
      set({
        isRecording: false,
        saving: false,
        elapsedMs: 0,
        savedToastMessage: message,
      });
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      set({ isRecording: false, saving: false, lastError: msg });
    }
  },

  select: (id) => {
    const prev = get();
    // Same-id click is a no-op: the selection invariants (id, isPlaying,
    // playbackTimeMs, persisted map) all already hold, so re-running the
    // restore-from-map path would clobber a live mid-playback offset.
    if (prev.currentRecordingId === id) return;
    // Stash the outgoing recording's playhead so a re-selection of the
    // same id resumes from the parked position. The map persists across
    // drawer collapse for the lifetime of the page session — selecting
    // `null` does NOT evict entries, so reopening the drawer and picking
    // the same row resumes from the parked offset.
    let nextMap: ReadonlyMap<RecordingId, number> | undefined;
    if (prev.currentRecordingId !== null) {
      const m = new Map(prev.persistedPlaybackPositionByRecording);
      m.set(prev.currentRecordingId, prev.playbackTimeMs);
      nextMap = m;
    }
    const restored =
      id !== null ? ((nextMap ?? prev.persistedPlaybackPositionByRecording).get(id) ?? 0) : 0;
    set({
      currentRecordingId: id,
      playbackTimeMs: restored,
      isPlaying: false,
      ...(nextMap !== undefined ? { persistedPlaybackPositionByRecording: nextMap } : {}),
    });
  },

  applyProgress: (p) => {
    // Only `elapsedMs` actually drives the view; ignoring the rest keeps
    // the selector graph small and avoids unnecessary re-renders.
    set({ elapsedMs: p.elapsedMs });
  },

  setError: (msg) => set({ lastError: msg }),

  dismissSavedToast: () => set({ savedToastMessage: null }),

  setPlaybackTimeMs: (ms) => set({ playbackTimeMs: ms }),

  setIsPlaying: (v) => set({ isPlaying: v }),

  toggleSpectrogram: () => set((s) => ({ showSpectrogram: !s.showSpectrogram })),

  __setItemsForTest: (items) => set({ items: sortDescByCreatedAt(items) }),
}));
