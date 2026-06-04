// Recordings store — slow-path Zustand state for the Phase 2.0 recorder.
//
// Per ADR-0003 and the Phase 1.2 RingBuffer contract, hot-path frames
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
// Cross-references:
//   docs/design/DESIGN.md §7.5 (Phase 2.0 frontend additions)
//   docs/design/DESIGN.md §8.3 (recordings DB schema)

import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
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
}

export interface RecordingsActions {
  refresh: () => Promise<void>;
  startRecording: (instrumentProfile: string) => Promise<void>;
  stopRecording: () => Promise<void>;
  select: (id: RecordingId | null) => void;
  applyProgress: (p: RecordingProgress) => void;
  setError: (msg: string | null) => void;
  dismissSavedToast: () => void;
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
    // Show a "starting…" state instead of optimistically flipping
    // `isRecording`. The previous code flipped `isRecording=true` before
    // the IPC resolved, which made it possible for a fast user to press
    // Stop while the start was still in-flight: `stopRecording`'s
    // `if (!get().isRecording) return;` guard let the call through, and
    // the front-end fired `stop_recording` against a take the Rust side
    // never started. Gating on `starting` (set here, cleared in the
    // resolve / reject paths) closes the race without losing the UX
    // affordance — the button reads `data-state="saving"` while
    // `starting` is true (the same affordance used during stop).
    set({ starting: true, elapsedMs: 0, lastError: null });
    try {
      await invoke<WireStartResponse | null>("start_recording", { instrumentProfile });
      // Only flip `isRecording` after the start IPC resolves. The Rust
      // side has now spawned the encoder thread and attached the DSP
      // fan-out; from this point Stop is a meaningful action.
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
          ? `Saved ${formatToastDuration(final.durationMs)} recording`
          : `Saved ${formatToastDuration(get().elapsedMs)} recording`;
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

  select: (id) => set({ currentRecordingId: id }),

  applyProgress: (p) => {
    // Only `elapsedMs` actually drives the view; ignoring the rest keeps
    // the selector graph small and avoids unnecessary re-renders.
    set({ elapsedMs: p.elapsedMs });
  },

  setError: (msg) => set({ lastError: msg }),

  dismissSavedToast: () => set({ savedToastMessage: null }),

  __setItemsForTest: (items) => set({ items: sortDescByCreatedAt(items) }),
}));

/** Local copy of `formatDurationShort` to avoid a circular import at the
 *  module-load layer (the store is imported from many components). */
function formatToastDuration(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor((Number.isFinite(ms) ? ms : 0) / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes === 0) return `${seconds}s`;
  const ss = seconds < 10 ? `0${seconds}` : String(seconds);
  return `${minutes}m${ss}s`;
}
