// Duration formatters for the recordings UI.
//
// Two surfaces share this module:
//   1. The `mm:ss` elapsed counter rendered next to the RecordButton while
//      a recording is active (also the reduced-motion visual cue).
//   2. The "Saved 1m23s recording" toast surfaced on stop.
//
// Cross-references:
//   docs/design/DESIGN.md §7.5 (Phase 2.0 frontend additions)

const MS_PER_SECOND = 1000;
const SECONDS_PER_MINUTE = 60;

function clampNonNegative(ms: number): number {
  if (!Number.isFinite(ms) || ms < 0) return 0;
  return ms;
}

/**
 * Format a duration as `m:ss` (or `mm:ss` for >=10 minute takes). Always
 * renders the seconds component zero-padded so the visual width is stable
 * frame-to-frame and the pulse-vs-static fallback path stays jitter-free.
 */
export function formatElapsed(elapsedMs: number): string {
  const totalSeconds = Math.floor(clampNonNegative(elapsedMs) / MS_PER_SECOND);
  const minutes = Math.floor(totalSeconds / SECONDS_PER_MINUTE);
  const seconds = totalSeconds % SECONDS_PER_MINUTE;
  const ss = seconds < 10 ? `0${seconds}` : String(seconds);
  return `${minutes}:${ss}`;
}

/**
 * Compact human-friendly duration used by the row metadata — sub-minute
 * takes render as `13s`, longer takes as `m:ss` (`1:23`, `12:05`). The
 * `m:ss` shape lines up with the elapsed-counter formatter so the recorder
 * never visually flips between "1m23s" while live and "1:23" while saved.
 * Maximum width is 6 characters at 99:59 — well within the 13-char cap
 * called out in the brief.
 */
export function formatDurationShort(durationMs: number): string {
  const totalSeconds = Math.floor(clampNonNegative(durationMs) / MS_PER_SECOND);
  const minutes = Math.floor(totalSeconds / SECONDS_PER_MINUTE);
  const seconds = totalSeconds % SECONDS_PER_MINUTE;
  if (minutes === 0) return `${seconds}s`;
  const ss = seconds < 10 ? `0${seconds}` : String(seconds);
  return `${minutes}:${ss}`;
}

/**
 * Toast-flavoured duration renderer used by the "Saved 1m23s recording"
 * toast on stop. Sub-minute takes render as `13s`; longer takes use the
 * compact `1m23s` shape (matching the in-flight elapsed counter's spoken
 * cadence rather than the row's `1:23` shape, so screen readers don't
 * announce it as a clock value).
 */
export function formatDurationToast(durationMs: number): string {
  const totalSeconds = Math.floor(clampNonNegative(durationMs) / MS_PER_SECOND);
  const minutes = Math.floor(totalSeconds / SECONDS_PER_MINUTE);
  const seconds = totalSeconds % SECONDS_PER_MINUTE;
  if (minutes === 0) return `${seconds}s`;
  const ss = seconds < 10 ? `0${seconds}` : String(seconds);
  return `${minutes}m${ss}s`;
}

/** Relative-time renderer used in RecordingsList. Resolves to "just now",
 *  "5m ago", "2h ago", or a YYYY-MM-DD date for older takes. Pure — the
 *  caller passes the reference `now` so tests can pin time. */
export function formatRelative(timestampMs: number, nowMs: number): string {
  const deltaMs = nowMs - timestampMs;
  if (!Number.isFinite(deltaMs)) return "—";
  const deltaSeconds = Math.max(0, Math.floor(deltaMs / MS_PER_SECOND));
  if (deltaSeconds < 30) return "just now";
  const minutes = Math.floor(deltaSeconds / SECONDS_PER_MINUTE);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  // Fall back to a plain ISO calendar date so the row stays readable
  // without dragging a locale-aware formatter into Phase 2.0.
  const d = new Date(timestampMs);
  const yyyy = d.getUTCFullYear().toString().padStart(4, "0");
  const mm = (d.getUTCMonth() + 1).toString().padStart(2, "0");
  const dd = d.getUTCDate().toString().padStart(2, "0");
  return `${yyyy}-${mm}-${dd}`;
}
