// Tauri IPC mock bridge for Playwright.
//
// Cross-references:
//   docs/design/TEST-PLAN.md §6.3 (Mock-Tauri bridge)
//   docs/adr/0019-tier-5-e2e-playwright-mcp.md ("page.addInitScript before React mounts")
//
// Implementation note: rather than dynamically importing
// `@tauri-apps/api/mocks` from the init script (which is async and races
// against the React `useEffect` that calls `invoke('greet')`), we inline the
// minimal synchronous shape of `mockIPC` directly. This mirrors what the
// upstream `mockIPC` function does — sets `window.__TAURI_INTERNALS__.invoke`
// plus the callback-registry fields — but does it before any application
// script runs, so the very first `invoke()` call sees the mock.
//
// Phase-1 will swap this for the bundled `mocks` module when the surface
// includes events / channels / windows that need the full upstream
// implementation.

import type { Page } from "@playwright/test";

/**
 * Per-command response. A function form receives the raw IPC payload
 * (the `args` object passed to `invoke`) and returns either the response
 * value or a Promise of it. Functions are serialised to source via
 * `Function.prototype.toString` and re-evaluated in the page context.
 */
export type TauriMockResponse =
  | unknown
  | ((args: Record<string, unknown>) => unknown | Promise<unknown>);

export type TauriMockResponses = Record<string, TauriMockResponse>;

/**
 * A serialisable description of a single mock entry: either a literal value
 * or a function-source string that the page-side init script will evaluate.
 */
interface SerialisedEntry {
  kind: "value" | "function";
  body: unknown;
}

/**
 * Phase-0 default responses. Adding entries here is how new Tauri commands
 * acquire a mock baseline that all specs share.
 */
export const defaultResponses: TauriMockResponses = {
  greet: (args: Record<string, unknown>) => {
    const name = typeof args["name"] === "string" ? (args["name"] as string) : "world";
    return `Hello, ${name}! NeuralPitch core says hi.`;
  },
};

function serialise(responses: TauriMockResponses): Record<string, SerialisedEntry> {
  const out: Record<string, SerialisedEntry> = {};
  for (const [k, v] of Object.entries(responses)) {
    if (typeof v === "function") {
      out[k] = { kind: "function", body: v.toString() };
    } else {
      out[k] = { kind: "value", body: v };
    }
  }
  return out;
}

/**
 * Install the mock-IPC bridge into the page before any script runs.
 *
 * The init script:
 *   1. Sets `window.__E2E__ = true` as a runtime sentinel.
 *   2. Stashes the response map on `window.__neuralPitchTestHooks` so
 *      `pushPitchUpdate` (below) and per-spec overrides can mutate it.
 *   3. Synchronously installs `window.__TAURI_INTERNALS__.invoke` to route
 *      every `invoke()` call through the response map.
 */
export async function installTauriMock(
  page: Page,
  responses: TauriMockResponses = {},
): Promise<void> {
  const merged: TauriMockResponses = { ...defaultResponses, ...responses };
  const serialised = serialise(merged);

  await page.addInitScript((entries: Record<string, SerialisedEntry>) => {
    type Handler = (args: Record<string, unknown>) => unknown | Promise<unknown>;
    type Internals = {
      invoke?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
      transformCallback?: (cb?: (data: unknown) => void, once?: boolean) => number;
      unregisterCallback?: (id: number) => void;
      runCallback?: (id: number, data: unknown) => void;
      callbacks?: Map<number, (data: unknown) => void>;
      metadata?: {
        currentWindow: { label: string };
        currentWebview: { windowLabel: string; label: string };
      };
    };
    type WindowWithHooks = Window & {
      __E2E__?: boolean;
      __TAURI_INTERNALS__?: Internals;
      __neuralPitchTestHooks?: {
        handlers: Map<string, Handler | unknown>;
        listeners: Map<string, Array<(payload: unknown) => void>>;
      };
    };
    const w = window as WindowWithHooks;
    w.__E2E__ = true;

    const handlers = new Map<string, Handler | unknown>();
    for (const [cmd, entry] of Object.entries(entries)) {
      if (entry.kind === "function") {
        // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
        const fn = new Function("return (" + (entry.body as string) + ")")() as Handler;
        handlers.set(cmd, fn);
      } else {
        handlers.set(cmd, entry.body);
      }
    }

    w.__neuralPitchTestHooks = {
      handlers,
      listeners: new Map(),
    };

    const internals: Internals = w.__TAURI_INTERNALS__ ?? {};
    const callbacks = new Map<number, (data: unknown) => void>();
    internals.callbacks = callbacks;
    internals.transformCallback = (cb, once) => {
      const id = Math.floor(Math.random() * 0xffffffff);
      callbacks.set(id, (data) => {
        if (once === true) callbacks.delete(id);
        if (cb) cb(data);
      });
      return id;
    };
    internals.unregisterCallback = (id) => {
      callbacks.delete(id);
    };
    internals.runCallback = (id, data) => {
      const fn = callbacks.get(id);
      if (fn) fn(data);
    };
    internals.invoke = async (cmd, args) => {
      const handler = handlers.get(cmd);
      if (handler === undefined) {
        throw new Error(`unmocked Tauri command: ${cmd}`);
      }
      if (typeof handler === "function") {
        return await (handler as Handler)(args ?? {});
      }
      return handler;
    };
    internals.metadata = {
      currentWindow: { label: "main" },
      currentWebview: { windowLabel: "main", label: "main" },
    };
    w.__TAURI_INTERNALS__ = internals;
  }, serialised);
}

/**
 * Push a simulated PitchUpdate frame to the page. Phase-1 will replace this
 * with a real Channel<PitchUpdate> shim; Phase-0 records the call onto
 * `window.__neuralPitchTestHooks.listeners.get('pitch-update')` so that a
 * future hook can subscribe.
 */
export async function pushPitchUpdate(
  page: Page,
  update: { f0Hz: number; cents: number; confidence: number; voiced: boolean },
): Promise<void> {
  await page.evaluate((frame) => {
    type WindowWithHooks = Window & {
      __neuralPitchTestHooks?: {
        listeners: Map<string, Array<(payload: unknown) => void>>;
      };
    };
    const w = window as WindowWithHooks;
    const listeners = w.__neuralPitchTestHooks?.listeners.get("pitch-update") ?? [];
    for (const fn of listeners) {
      fn(frame);
    }
  }, update);
}
