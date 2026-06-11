// ImportButton spec.
//
// Drives the import flow through the mock IPC + dialog bridge:
//
//   1. Open the recordings drawer; the toolbar at the top of the drawer
//      body exposes the existing record button + a new ImportButton.
//   2. Click ImportButton — the dialog mock returns a fixed sentinel path
//      so the page-side `open()` resolves to a string and the component
//      forwards it to `import_audio_file`.
//   3. The store re-fetches via `list_recordings()` after the import, so a
//      new `[data-testid=recording-row]` appears in the list.
//   4. `getInvokeCalls(page, "import_audio_file")` length === 1.
//
// The button is disabled during the in-flight call and reads
// `aria-busy="true"`. We do not assert the busy attribute here — that is
// covered by the a11y spec — to keep this test focused on the wire-level
// contract.
//

import { expect, test } from "./fixtures";
import {
  getInvokeCalls,
  installDialogMock,
  installImportMock,
  installRecordingsMock,
  type MockRecording,
} from "./helpers/tauri-mock";

const SEED: MockRecording[] = [];

test.describe("import button", () => {
  test("clicking Import invokes import_audio_file and the row appears", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installImportMock(),
    });
    await installDialogMock(page, "/tmp/imported-fixture.wav");

    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    const list = page.getByTestId("recordings-list");
    await expect(list).toBeVisible();
    const initialRows = await list.locator("li").count();

    // The new ImportButton lives in the drawer-header toolbar beside the
    // existing RecordButton.
    const importButton = page.getByTestId("import-button");
    await expect(importButton).toBeVisible();
    await expect(importButton).toHaveAttribute("aria-label", /Import audio file/i);

    await importButton.click();

    // List grows by one; new row mounts at the top (descending createdAt).
    await expect(list.locator("li")).toHaveCount(initialRows + 1);
    const calls = await getInvokeCalls(page, "import_audio_file");
    expect(calls).toHaveLength(1);
    expect(calls[0]?.args).toMatchObject({ sourcePath: "/tmp/imported-fixture.wav" });
  });

  test("dismissing the file dialog is a no-op", async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installImportMock(),
    });
    // `null` means the user dismissed the dialog without selecting a file.
    await installDialogMock(page, null);

    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    const list = page.getByTestId("recordings-list");
    await expect(list).toBeVisible();

    const importButton = page.getByTestId("import-button");
    await importButton.click();

    // No row added, no IPC fired.
    await expect(list.locator("li")).toHaveCount(0);
    const calls = await getInvokeCalls(page, "import_audio_file");
    expect(calls).toHaveLength(0);
  });

  test("toolbar exposes role=toolbar with Recording controls label", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installImportMock(),
    });
    await installDialogMock(page, "/tmp/imported-fixture.wav");

    await page.goto("/");
    await page.getByTestId("library-trigger").click();

    const toolbar = page.getByRole("toolbar", { name: /Recording controls/i });
    await expect(toolbar).toBeVisible();

    // Both controls are descendants of the toolbar so keyboard arrow
    // navigation eventually picks them up. We assert presence here; the
    // arrow-key wiring is owned by the underlying primitive.
    await expect(toolbar.getByTestId("record-button")).toBeVisible();
    await expect(toolbar.getByTestId("import-button")).toBeVisible();
  });
});
