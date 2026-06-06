// Test-hook bridge for the Tier-5 E2E mock harness.
//
// The mock harness in `tests/e2e/helpers/tauri-mock.ts` installs a
// `window.__neuralPitchTestHooks.listeners` map keyed by channel name. Each
// production-side hook (usePitchStream, useDeviceEvents, useRecordingProgress,
// useAnalysisProgress) registers a fanout function that the spec-side helpers
// (`pushPitchUpdate`, `pushDeviceEvent`, `pushRecordingProgress`,
// `pushAnalysisProgress`) invoke. Centralising the registration shape here
// keeps the four hooks in lockstep — extending the wire-format only requires
// one edit, not four.

type Listener = (payload: unknown) => void;

interface TestHooks {
  listeners: Map<string, Listener[]>;
}

interface WindowWithHooks extends Window {
  __neuralPitchTestHooks?: TestHooks;
}

/** Read-only accessor for the harness handle. Returns `undefined` in
 *  production (no harness installed). */
export function getTestHooks(): TestHooks | undefined {
  return (window as WindowWithHooks).__neuralPitchTestHooks;
}

/**
 * Register a listener on the harness fanout for a given channel. Returns a
 * teardown closure that removes the listener. If the harness is not present
 * (production), the registration is a no-op and the teardown is also a
 * no-op.
 */
export function registerTestListener(channel: string, fn: (payload: unknown) => void): () => void {
  const hooks = getTestHooks();
  if (hooks === undefined) {
    return () => {
      /* no-op: production runs without the harness */
    };
  }
  const list = hooks.listeners.get(channel) ?? [];
  list.push(fn);
  hooks.listeners.set(channel, list);
  return () => {
    const cur = hooks.listeners.get(channel) ?? [];
    hooks.listeners.set(
      channel,
      cur.filter((f) => f !== fn),
    );
  };
}
