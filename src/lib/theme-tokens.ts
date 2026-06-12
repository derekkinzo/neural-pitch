// Centralised hex literals for canvas/wavesurfer paint paths.
//
// Tailwind classes do not work for runtime canvas APIs (WaveSurfer's
// `waveColor`, `progressColor`, `<canvas>` `fillStyle`, etc.).
// Centralised so theme changes touch one file rather than every
// canvas/wavesurfer caller.

export const COLOR_SLATE_600 = "#475569";
export const COLOR_SLATE_700 = "#334155";
export const COLOR_CYAN_400 = "#22d3ee";
export const COLOR_SLATE_900 = "#0f172a";
export const COLOR_VOICED_FILL = "rgba(34, 211, 238, 0.12)";
