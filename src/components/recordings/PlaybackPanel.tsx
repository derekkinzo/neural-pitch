// PlaybackPanel — Phase 2.4 wavesurfer-driven recording playback.
//
// Mounts a WaveSurfer instance on the host div whenever
// `currentRecordingId` flips to a non-null value, exposes Play/Pause +
// scrubber + readout, and (lazily) toggles the spectrogram plugin into
// a sibling host. The hot path (audioprocess @ ~50 Hz) writes through
// `lib/playback-head` — NOT through Zustand — so ContourLine can read
// the head inside its own rAF loop without forcing React re-renders.
//
// Reduced-motion is captured at panel mount; an OS preference flip
// mid-session takes effect on the next recording selection. The static
// fallback (autoScroll/autoCenter off) satisfies the visual-only
// feedback contract for users who request reduced motion.

import { useEffect, useRef, useState, type ReactNode } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import WaveSurfer from "wavesurfer.js";
import { formatDurationShort } from "@/lib/duration-format";
import { publishIsPlaying, publishTime, resetPlaybackHead } from "@/lib/playback-head";
import { getTestHooks } from "@/lib/test-hooks";
import { COLOR_CYAN_400, COLOR_SLATE_600 } from "@/lib/theme-tokens";
import { useRecordingsStore } from "@/stores/recordingsStore";

const PLAYBACK_TIME_THROTTLE_MS = 33; // ~30 Hz visible readout

/**
 * Optional props.
 *
 * `audioPath` (Phase 5): when set, the panel skips the
 * `get_recording_path` IPC and resolves the supplied path directly via
 * `convertFileSrc()` (or the test-hook override). The Phase 5 stem
 * cards pass each FLAC's full path explicitly so the same panel
 * component covers both the mix (where the recording id implies the
 * path) and a per-stem playback (where the path is already known).
 *
 * `variant` (Phase 5): "stem" suppresses the playback-head publish path
 * AND the spectrogram toggle. The mix panel publishes into
 * `playback-head` so ContourLine consumes the cursor; stem panels stay
 * self-contained because no live overlay reads from them. The
 * spectrogram is reserved for the mix to keep the FFT cost bounded — a
 * single host, not five.
 */
export interface PlaybackPanelProps {
  readonly audioPath?: string;
  readonly variant?: "mix" | "stem";
}

interface SpectrogramPluginCtor {
  create: (opts: {
    container: string | HTMLElement;
    labels?: boolean;
    fftSamples?: number;
    scale?: "linear" | "logarithmic" | "mel" | "bark" | "erb";
    colorMap?: number[][] | "gray" | "igray" | "roseus";
    height?: number;
  }) => unknown;
}

/** Plugin instance shape for our cleanup path. WaveSurfer's GenericPlugin
 *  exposes `destroy()`; we cast at the call site. */
interface DestroyablePlugin {
  destroy?: () => void;
}

function resolveAssetUrl(path: string): string {
  // E2E mock can override the resolver via the centralised test-hooks
  // bridge (`src/lib/test-hooks.ts`). The production path goes through
  // Tauri's `convertFileSrc()`. Routing through `getTestHooks()` keeps
  // the harness shape edits in one file rather than four.
  const hook = getTestHooks()?.convertFileSrc;
  if (typeof hook === "function") return hook(path);
  return convertFileSrc(path);
}

export function PlaybackPanel(props: PlaybackPanelProps = {}): ReactNode {
  const audioPath = props.audioPath;
  const variant = props.variant ?? "mix";
  const isStemVariant = variant === "stem";
  const currentRecordingId = useRecordingsStore((s) => s.currentRecordingId);
  const setIsPlayingStore = useRecordingsStore((s) => s.setIsPlaying);
  const isPlayingStore = useRecordingsStore((s) => s.isPlaying);
  const setPlaybackTimeMs = useRecordingsStore((s) => s.setPlaybackTimeMs);
  const playbackTimeMsStore = useRecordingsStore((s) => s.playbackTimeMs);
  const showSpectrogram = useRecordingsStore((s) => s.showSpectrogram);
  const toggleSpectrogram = useRecordingsStore((s) => s.toggleSpectrogram);

  // Stem panels keep their own play-state + time-readout local so the
  // four cards do not crosstalk on the shared `recordingsStore` slot.
  // The mix panel keeps reading from the store so existing callers are
  // unaffected.
  const [localIsPlaying, setLocalIsPlaying] = useState<boolean>(false);
  const [localTimeMs, setLocalTimeMs] = useState<number>(0);
  const isPlaying = isStemVariant ? localIsPlaying : isPlayingStore;
  const playbackTimeMs = isStemVariant ? localTimeMs : playbackTimeMsStore;

  const containerRef = useRef<HTMLDivElement | null>(null);
  const spectrogramHostRef = useRef<HTMLDivElement | null>(null);
  const wsRef = useRef<WaveSurfer | null>(null);
  const lastStoreWriteRef = useRef<number>(0);
  const spectrogramPluginRef = useRef<DestroyablePlugin | null>(null);
  const wsReadyRef = useRef<boolean>(false);

  const [durationMs, setDurationMs] = useState<number>(0);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [wsReady, setWsReady] = useState<boolean>(false);

  // Mount / re-mount wavesurfer when the recording selection changes.
  useEffect(() => {
    // Reset transient panel state for both branches (null id or new id) so
    // the previous recording's slider/readout/error toast does not bleed
    // through while the new wavesurfer is mounting.
    setDurationMs(0);
    setErrorMsg(null);
    setWsReady(false);
    wsReadyRef.current = false;
    lastStoreWriteRef.current = 0;
    // Tear down any previous wavesurfer before mounting a new one (or
    // before parking the panel on a null selection). Without this the
    // null-id branch leaks the audio element and its decoded buffer
    // until the next selection, since `useEffect`'s cleanup only runs
    // on the *next* effect cycle.
    if (wsRef.current !== null) {
      try {
        wsRef.current.destroy();
      } catch (err: unknown) {
        if (typeof console !== "undefined") {
          console.warn("PlaybackPanel: wavesurfer destroy threw", err);
        }
      }
      wsRef.current = null;
    }
    spectrogramPluginRef.current = null;

    // Stem panels mount on `audioPath` directly — no recording id to
    // gate against, so a `null` `currentRecordingId` is fine. The mix
    // panel still parks on null until a row is selected.
    if (!isStemVariant && currentRecordingId === null) {
      resetPlaybackHead(0);
      return;
    }
    let cancelled = false;
    let ws: WaveSurfer | null = null;
    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

    // Single source of truth for the throttled `playbackTimeMs` write —
    // both `audioprocess` and `seeking` funnel through this so the panel
    // re-render rate stays bounded at ~30 Hz no matter which event drives
    // the time update.
    const writePlaybackTime = (tMs: number): void => {
      const now = typeof performance !== "undefined" ? performance.now() : Date.now();
      if (now - lastStoreWriteRef.current >= PLAYBACK_TIME_THROTTLE_MS) {
        lastStoreWriteRef.current = now;
        if (isStemVariant) {
          setLocalTimeMs(tMs);
        } else {
          setPlaybackTimeMs(tMs);
        }
      }
    };

    const mount = async (): Promise<void> => {
      try {
        // Stem panels supply the path explicitly; mix panel resolves it
        // via `get_recording_path` keyed on the current recording id.
        const path =
          audioPath !== undefined
            ? audioPath
            : await invoke<string>("get_recording_path", { id: currentRecordingId });
        if (cancelled) return;
        const url = resolveAssetUrl(path);
        const host = containerRef.current;
        if (host === null) return;
        ws = WaveSurfer.create({
          container: host,
          url,
          height: 64,
          waveColor: COLOR_SLATE_600,
          progressColor: COLOR_CYAN_400,
          cursorColor: COLOR_CYAN_400,
          barWidth: 2,
          normalize: true,
          autoScroll: !reducedMotion,
          autoCenter: !reducedMotion,
          interact: true,
        });
        wsRef.current = ws;

        ws.on("ready", () => {
          if (cancelled || ws === null) return;
          wsReadyRef.current = true;
          setWsReady(true);
          setDurationMs(ws.getDuration() * 1000);
        });
        ws.on("audioprocess", (t) => {
          if (cancelled) return;
          const tMs = t * 1000;
          // Only the mix panel publishes into the playback-head bus —
          // ContourLine's overlay only consumes the mix.
          if (!isStemVariant) publishTime(tMs);
          writePlaybackTime(tMs);
        });
        ws.on("play", () => {
          if (isStemVariant) {
            setLocalIsPlaying(true);
          } else {
            publishIsPlaying(true);
            setIsPlayingStore(true);
          }
        });
        ws.on("pause", () => {
          if (isStemVariant) {
            setLocalIsPlaying(false);
          } else {
            publishIsPlaying(false);
            setIsPlayingStore(false);
          }
        });
        ws.on("finish", () => {
          if (isStemVariant) {
            setLocalIsPlaying(false);
          } else {
            publishIsPlaying(false);
            setIsPlayingStore(false);
          }
        });
        ws.on("seeking", (t) => {
          const tMs = t * 1000;
          if (!isStemVariant) publishTime(tMs);
          // Same throttle as audioprocess — a slider drag fires `seeking`
          // at every pixel of motion, which would otherwise re-render
          // the entire panel at 60 Hz per drag-tick.
          writePlaybackTime(tMs);
        });
        ws.on("error", (err: unknown) => {
          if (cancelled) return;
          const msg = err instanceof Error ? err.message : String(err);
          setErrorMsg(msg);
        });
      } catch (err: unknown) {
        if (cancelled) return;
        const msg = err instanceof Error ? err.message : String(err);
        setErrorMsg(msg);
      }
    };

    void mount();

    return () => {
      cancelled = true;
      wsReadyRef.current = false;
      setWsReady(false);
      // Destroy the spectrogram plugin first so its hooks unwind before
      // wavesurfer's own teardown.
      const prevPlugin = spectrogramPluginRef.current;
      if (prevPlugin !== null && typeof prevPlugin.destroy === "function") {
        try {
          prevPlugin.destroy();
        } catch (err: unknown) {
          if (typeof console !== "undefined") {
            console.warn("PlaybackPanel: spectrogram destroy threw", err);
          }
        }
      }
      spectrogramPluginRef.current = null;
      if (wsRef.current !== null) {
        try {
          wsRef.current.destroy();
        } catch (err: unknown) {
          if (typeof console !== "undefined") {
            console.warn("PlaybackPanel: wavesurfer destroy threw", err);
          }
        }
        wsRef.current = null;
      }
      resetPlaybackHead(0);
    };
    // Zustand action refs are stable; only `currentRecordingId` should
    // re-mount wavesurfer. Including the actions in the dep array would
    // silently re-mount on every store change after a future refactor.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentRecordingId]);

  // Lazy-mount the spectrogram plugin the first time the user toggles
  // it on, AFTER wavesurfer is ready. Keep the chunk out of first paint.
  // When the user toggles OFF we destroy the plugin so its FFT loop
  // stops — keeping it registered would continue computing into a
  // hidden host every audio frame.
  useEffect(() => {
    const ws = wsRef.current;
    if (!showSpectrogram) {
      // Toggle-off: destroy the plugin if we have one. This keeps the
      // already-imported chunk warm in the JS heap (so the next toggle
      // is instant) but stops the per-frame FFT compute.
      const plugin = spectrogramPluginRef.current;
      if (plugin !== null && typeof plugin.destroy === "function") {
        try {
          plugin.destroy();
        } catch (err: unknown) {
          if (typeof console !== "undefined") {
            console.warn("PlaybackPanel: spectrogram destroy threw", err);
          }
        }
        spectrogramPluginRef.current = null;
      }
      return;
    }
    if (!wsReady) return;
    if (spectrogramPluginRef.current !== null) return;
    if (ws === null) return;
    const host = spectrogramHostRef.current;
    if (host === null) return;
    let cancelled = false;
    void (async (): Promise<void> => {
      try {
        const mod = (await import("wavesurfer.js/dist/plugins/spectrogram.esm.js")) as {
          default: SpectrogramPluginCtor;
        };
        if (cancelled) return;
        const plugin = mod.default.create({
          // Pass the resolved HTMLElement rather than a global `#id`
          // selector so a future feature that mounts a second
          // PlaybackPanel cannot accidentally bind the plugin to the
          // first host.
          container: host,
          labels: true,
          fftSamples: 512,
          scale: "logarithmic",
          colorMap: "roseus",
          height: 96,
        });
        // wavesurfer's registerPlugin returns the same plugin instance;
        // we keep a ref so toggling off → on does not double-register
        // and so the cleanup branch can call `destroy()`.
        const registered = ws.registerPlugin(plugin as never) as DestroyablePlugin;
        spectrogramPluginRef.current = registered;
      } catch (err: unknown) {
        if (cancelled) return;
        const msg = err instanceof Error ? err.message : String(err);
        setErrorMsg(`Spectrogram failed: ${msg}`);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [showSpectrogram, wsReady]);

  if (currentRecordingId === null) return null;

  const onTogglePlay = (): void => {
    const ws = wsRef.current;
    if (ws === null) return;
    void ws.playPause();
  };

  const onSeek = (e: React.ChangeEvent<HTMLInputElement>): void => {
    const ws = wsRef.current;
    if (ws === null || durationMs <= 0) return;
    const value = Number(e.currentTarget.value);
    if (!Number.isFinite(value)) return;
    const ratio = Math.max(0, Math.min(1, value / durationMs));
    // Let wavesurfer be the single source of truth: `seekTo` will fire
    // `seeking`, which writes the throttled store value and publishes
    // into the playback head. Skipping the explicit writes here avoids
    // the double-write per drag-tick that previously caused slider
    // jitter.
    ws.seekTo(ratio);
  };

  // Spacebar/k panel-level shortcut. We intercept only when the active
  // element is not a form control (the slider's native key behavior is
  // arrow keys only — space on a range input is no-op, but we still
  // skip if the focus target is an INPUT/SELECT/TEXTAREA so a future
  // textfield in the panel keeps native space behavior).
  const onPanelKeyDown = (e: React.KeyboardEvent<HTMLElement>): void => {
    if (e.key !== " " && e.key !== "k") return;
    const target = e.target as HTMLElement | null;
    const tag = target?.tagName ?? "";
    if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
    e.preventDefault();
    onTogglePlay();
  };

  const safeDuration = Math.max(0, Math.round(durationMs));
  const safeTime = Math.max(0, Math.min(safeDuration, Math.round(playbackTimeMs)));
  const ariaMax = Math.max(1, safeDuration);
  const controlsDisabled = !wsReady;

  return (
    <section
      data-testid="playback-panel"
      role="region"
      aria-label="Recording playback"
      aria-busy={!wsReady && errorMsg === null}
      onKeyDown={onPanelKeyDown}
      className="flex flex-col gap-2 rounded-md border border-slate-700 bg-slate-900/50 p-3"
    >
      <div className="text-xs font-medium uppercase tracking-wide text-slate-300">Playback</div>
      {errorMsg !== null ? (
        <div role="alert" className="text-xs text-rose-300">
          Could not load recording: {errorMsg}
        </div>
      ) : null}
      <div
        ref={containerRef}
        data-testid="waveform-host"
        className="w-full overflow-hidden rounded-sm bg-slate-950"
      />
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          data-testid="playback-toggle"
          aria-pressed={isPlaying}
          aria-label={isPlaying ? "Pause" : "Play"}
          onClick={onTogglePlay}
          disabled={controlsDisabled}
          className={[
            "rounded-md border border-slate-600 bg-slate-800 px-3 py-1 text-xs text-slate-100",
            "hover:bg-slate-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400",
            "disabled:cursor-not-allowed disabled:opacity-60",
          ].join(" ")}
        >
          {isPlaying ? "Pause" : "Play"}
        </button>
        <input
          type="range"
          role="slider"
          aria-label="Playback position"
          aria-valuemin={0}
          aria-valuemax={ariaMax}
          aria-valuenow={safeTime}
          min={0}
          max={ariaMax}
          value={safeTime}
          step={1}
          onChange={onSeek}
          disabled={controlsDisabled}
          className="flex-1 accent-cyan-400 disabled:cursor-not-allowed disabled:opacity-60"
        />
        <span data-testid="playback-time" className="font-mono text-xs tabular-nums text-slate-200">
          {`${formatDurationShort(safeTime)} / ${formatDurationShort(safeDuration)}`}
        </span>
        <button
          type="button"
          data-testid="spectrogram-toggle"
          aria-pressed={showSpectrogram}
          aria-controls="spectrogram-host"
          onClick={toggleSpectrogram}
          className={[
            "rounded-md border border-slate-600 bg-slate-800 px-3 py-1 text-xs text-slate-200",
            "hover:bg-slate-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400",
          ].join(" ")}
        >
          {showSpectrogram ? "Hide spectrogram" : "Show spectrogram"}
        </button>
      </div>
      <div
        ref={spectrogramHostRef}
        id="spectrogram-host"
        data-testid="spectrogram-host"
        aria-hidden={!showSpectrogram}
        hidden={!showSpectrogram}
        className="w-full overflow-hidden rounded-sm bg-slate-950"
      />
    </section>
  );
}
