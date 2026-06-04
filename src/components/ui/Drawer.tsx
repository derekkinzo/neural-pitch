// Drawer — right-side slide-in dialog with focus trap and Escape-to-close.
//
// Vendored under `components/ui/` per ADR-0003. The shadcn `Drawer` primitive
// wraps Radix Dialog; we implement the same surface in ~80 lines because the
// Phase 1.2 use case is exactly one drawer, exactly one trigger, exactly one
// modal pattern. Phase 1.4 polish may switch to Radix once we add a second
// drawer surface (e.g. Phase-2 metronome side-panel).
//
// Implementation:
//   - role="dialog" aria-modal="true" on the panel.
//   - aria-labelledby points at the supplied title.
//   - On open, the first focusable descendant inside the panel receives focus.
//   - Tab cycles within the panel (focus trap).
//   - Escape closes; clicking the backdrop also closes.
//
// Cross-references:
//   docs/design/DESIGN.md §6 (right-side sheet, focus-trapped)

import { useCallback, useEffect, useId, useRef, type ReactNode } from "react";

export interface DrawerProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Required visible label rendered as the heading at the top of the panel
   *  (also referenced by aria-labelledby). */
  title: string;
  children: ReactNode;
}

const FOCUSABLE_SELECTORS =
  'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), ' +
  'select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function Drawer({ open, onOpenChange, title, children }: DrawerProps): ReactNode {
  const titleId = useId();
  const panelRef = useRef<HTMLDivElement | null>(null);
  const lastActiveBeforeOpen = useRef<HTMLElement | null>(null);

  const close = useCallback(() => onOpenChange(false), [onOpenChange]);

  useEffect(() => {
    if (!open) return undefined;

    lastActiveBeforeOpen.current = (document.activeElement as HTMLElement | null) ?? null;
    const panel = panelRef.current;

    // Make EVERY top-level sibling of the drawer-root inert so AT virtual
    // cursors (VoiceOver, NVDA browse-mode) cannot navigate to anything
    // outside the modal dialog (WAI-ARIA APG modal-dialog pattern). We
    // walk the body's children rather than only `<main>` because the
    // application mounts other top-level surfaces (PermissionNotice,
    // DeviceDisconnectToast) as siblings of `<main>`, and a disconnect
    // event firing while the drawer is open MUST NOT yield a focusable
    // Reconnect button outside the focus trap.
    type InertElement = HTMLElement & { inert?: boolean };
    // The drawer-root is the backdrop div with `data-testid="drawer-root"`;
    // it wraps the panel. We walk every ancestor of the drawer-root and
    // inert every sibling of that ancestor up to (but not including)
    // `<body>`. That covers all top-level mounts under React's `<div id="root">`
    // (PermissionNotice, DeviceDisconnectToast, the main content, and any
    // future portal targets) without inerting the drawer's own subtree.
    interface InertedNode {
      readonly el: HTMLElement;
      readonly priorInert: boolean | undefined;
      readonly priorAriaHidden: string | null;
    }
    const inerted: InertedNode[] = [];
    const drawerRoot = panel?.closest('[data-testid="drawer-root"]') ?? null;
    let cursor: HTMLElement | null = drawerRoot as HTMLElement | null;
    while (cursor !== null && cursor.parentElement !== null && cursor !== document.body) {
      const parent = cursor.parentElement;
      for (const sibling of Array.from(parent.children)) {
        if (!(sibling instanceof HTMLElement)) continue;
        if (sibling === cursor) continue;
        const inertCapable = sibling as InertElement;
        inerted.push({
          el: sibling,
          priorInert: inertCapable.inert,
          priorAriaHidden: sibling.getAttribute("aria-hidden"),
        });
        inertCapable.inert = true;
        sibling.setAttribute("aria-hidden", "true");
      }
      cursor = parent;
      if (parent === document.body) break;
    }

    // Defer focus until the panel is in the DOM.
    const id = window.setTimeout(() => {
      const first = panel?.querySelector<HTMLElement>(FOCUSABLE_SELECTORS);
      first?.focus();
    }, 0);

    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
        return;
      }
      if (e.key !== "Tab" || panel === null) return;
      const focusables = Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTORS));
      if (focusables.length === 0) {
        e.preventDefault();
        return;
      }
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      if (first === undefined || last === undefined) return;
      const active = document.activeElement as HTMLElement | null;
      if (e.shiftKey && active === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    };

    document.addEventListener("keydown", onKey);
    return () => {
      window.clearTimeout(id);
      document.removeEventListener("keydown", onKey);
      // Restore each previously-inerted sibling. `false` is the resting
      // value so we default to that when the original was `undefined`
      // (browsers without `inert` reflect the undefined as no-op).
      for (const { el, priorInert, priorAriaHidden } of inerted) {
        const inertCapable = el as InertElement;
        inertCapable.inert = priorInert ?? false;
        if (priorAriaHidden === null) el.removeAttribute("aria-hidden");
        else el.setAttribute("aria-hidden", priorAriaHidden);
      }
      lastActiveBeforeOpen.current?.focus?.();
    };
  }, [open, close]);

  if (!open) return null;

  return (
    <div
      data-testid="drawer-root"
      className="fixed inset-0 z-50 flex justify-end bg-black/40"
      onClick={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="flex h-full w-[360px] max-w-full flex-col overflow-y-auto bg-slate-900 p-6 shadow-xl"
      >
        <div className="mb-4 flex items-center justify-between">
          <h2 id={titleId} className="text-lg font-semibold text-slate-100">
            {title}
          </h2>
          <button
            type="button"
            aria-label="Close settings"
            onClick={close}
            className="rounded p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400"
          >
            <span aria-hidden="true">×</span>
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}
