// Thin wrapper around the Tauri file-open dialog.
//
// The ImportButton needs to ask the user to pick a `.wav` /
// `.flac` file. The Tauri 2 plugin lives behind the
// `plugin:dialog|open` IPC command and ships as a separate npm package
// (`@tauri-apps/plugin-dialog`) which we do NOT install — keeping the
// front-end JS dep surface tight. Routing through `invoke()` directly
// keeps the IPC boundary in one place and makes the E2E mock trivial.
//
// E2E path: when `getTestHooks()?.dialogOpenResult` is set (the spec
// installed `installDialogMock`), we resolve to that sentinel without
// hitting the IPC at all — same shim pattern as `convertFileSrc` in
// PlaybackPanel.
//
// Production path: forward to the real plugin via `invoke()`. A missing
// plugin (e.g. headless `npm run dev` outside Tauri) degrades to `null`
// so the UI does not crash; the click is a silent no-op which matches
// the dialog-dismiss branch.

import { invoke } from "@tauri-apps/api/core";
import { getTestHooks } from "@/lib/test-hooks";

/** Filter forwarded to `plugin:dialog|open`. Mirrors the plugin's
 *  `DialogFilter` shape. */
export interface AudioOpenFilter {
  readonly name: string;
  readonly extensions: readonly string[];
}

/** Open a single-file picker constrained to the supplied filters. Returns
 *  the selected path on success, or `null` if the user dismissed the
 *  dialog. Errors degrade to `null` — a non-functional plugin (test page
 *  outside Tauri) should not crash the UI. */
export async function openAudioFileDialog(
  filters: readonly AudioOpenFilter[],
): Promise<string | null> {
  const hook = getTestHooks();
  if (hook !== undefined && "dialogOpenResult" in hook) {
    // The harness branch — `null` is a legitimate value (user dismissed).
    return hook.dialogOpenResult ?? null;
  }
  try {
    const raw = await invoke<unknown>("plugin:dialog|open", {
      options: {
        multiple: false,
        directory: false,
        filters: filters.map((f) => ({ name: f.name, extensions: [...f.extensions] })),
      },
    });
    if (typeof raw === "string") return raw;
    // The plugin can return `{ path }` on some platforms; accept both
    // shapes defensively.
    if (raw !== null && typeof raw === "object" && "path" in raw) {
      const p = (raw as { path?: unknown }).path;
      if (typeof p === "string") return p;
    }
    return null;
  } catch {
    return null;
  }
}
