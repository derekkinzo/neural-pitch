// Radiogroup keyboard navigation helper.
//
// WAI-ARIA APG radiogroup pattern requires arrow-key + Home / End focus
// movement between radios. The drill choice grids (Interval /
// Chord / Scale) all share the same shape — a flat list of `role="radio"`
// buttons with a `data-testid` prefix per choice — so we centralise the
// key-handler logic here. Each drill passes `containerRef` (the element
// that owns the radios) and the helper walks the descendants in DOM
// order, picking the next/prev/first/last radio relative to the
// currently-focused one and moving focus there.
//
// Behaviour:
//   - ArrowRight, ArrowDown → next radio (wraps to first)
//   - ArrowLeft,  ArrowUp   → prev radio (wraps to last)
//   - Home                  → first radio
//   - End                   → last radio
//
// Activation is left to the existing click handler — the radio's
// `onClick` already runs when the user presses Space or Enter on a
// focused button, so we do not double-handle activation here.

import type { KeyboardEvent } from "react";

const NAV_KEYS = new Set(["ArrowRight", "ArrowDown", "ArrowLeft", "ArrowUp", "Home", "End"]);

/**
 * Move focus between sibling `role="radio"` buttons in response to the
 * arrow / Home / End keys. Returns true when the event was handled and
 * the caller should stop further propagation.
 */
export function handleRadioGroupKeydown(
  event: KeyboardEvent<HTMLDivElement>,
  container: HTMLElement | null,
): boolean {
  if (container === null) return false;
  if (!NAV_KEYS.has(event.key)) return false;

  const radios = Array.from(container.querySelectorAll<HTMLElement>('[role="radio"]'));
  if (radios.length === 0) return false;

  const active = document.activeElement as HTMLElement | null;
  const currentIdx = active === null ? -1 : radios.indexOf(active);
  let nextIdx = currentIdx;

  switch (event.key) {
    case "ArrowRight":
    case "ArrowDown":
      nextIdx = currentIdx < 0 ? 0 : (currentIdx + 1) % radios.length;
      break;
    case "ArrowLeft":
    case "ArrowUp":
      nextIdx = currentIdx <= 0 ? radios.length - 1 : currentIdx - 1;
      break;
    case "Home":
      nextIdx = 0;
      break;
    case "End":
      nextIdx = radios.length - 1;
      break;
    default:
      return false;
  }

  const target = radios[nextIdx];
  if (target === undefined) return false;
  event.preventDefault();
  target.focus();
  return true;
}
