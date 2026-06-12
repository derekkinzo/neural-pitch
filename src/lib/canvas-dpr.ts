// Canvas DPR scaling helper.
//
// All three of `CentsMeter`, `HistoryStrip`, and `ContourLine` paint a
// canvas at the device pixel ratio so HiDPI screens stay crisp without
// blurring. They all want the same algorithm: read CSS dimensions, scale
// up the backing-store size by `devicePixelRatio`, and reset the 2D
// transform so callers can keep painting in CSS-pixel coordinates.
//
// Centralised so all canvas DPR scaling consumers share one code path.

/**
 * Resize the canvas backing store to the current CSS dimensions multiplied
 * by `devicePixelRatio`, then reset the 2D context transform so callers
 * keep painting in CSS-pixel coordinates.
 */
export function scaleForDpr(canvas: HTMLCanvasElement, ctx: CanvasRenderingContext2D): void {
  const dpr = window.devicePixelRatio || 1;
  const cssW = canvas.clientWidth;
  const cssH = canvas.clientHeight;
  canvas.width = Math.max(1, Math.round(cssW * dpr));
  canvas.height = Math.max(1, Math.round(cssH * dpr));
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
}
