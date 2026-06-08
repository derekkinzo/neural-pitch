// Centralised hex literals for canvas/wavesurfer paint paths.
//
// Tailwind classes do not work for runtime canvas APIs (WaveSurfer's
// `waveColor`, `progressColor`, `<canvas>` `fillStyle`, etc.). Mirroring
// the Tailwind theme tokens here keeps the duplication in one place so a
// future theming pass touches a single file rather than every component.

export const COLOR_SLATE_600 = "#475569";
export const COLOR_SLATE_700 = "#334155";
export const COLOR_CYAN_400 = "#22d3ee";
export const COLOR_SLATE_900 = "#0f172a";
export const COLOR_VOICED_FILL = "rgba(34, 211, 238, 0.12)";
