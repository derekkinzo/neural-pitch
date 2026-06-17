// RecordingsList drawer spec.
//
// Asserts the seeded list renders in descending createdAt order with
// per-row metadata (filename, duration, instrument profile). The drawer
// itself is the existing `<Drawer>` primitive; this spec only exercises
// the list contents.
//

import { expect, test } from "./fixtures";
import { installRecordingsMock, type MockRecording } from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000; // arbitrary fixed epoch — keeps the spec deterministic.

const SEED: MockRecording[] = [
  {
    id: "rec-001",
    filename: "voice-note-001.flac",
    createdAt: NOW - 60 * 60 * 1000, // 1 h ago — should sort *last*
    durationMs: 12_000,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
  {
    id: "rec-002",
    filename: "guitar-take-002.flac",
    createdAt: NOW - 30 * 60 * 1000, // 30 min ago — middle
    durationMs: 45_000,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Guitar",
  },
  {
    id: "rec-003",
    filename: "violin-improv-003.flac",
    createdAt: NOW - 5 * 60 * 1000, // 5 min ago — should sort *first*
    durationMs: 83_000,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Violin",
  },
];

test.describe("recordings list — seed render + ordering", () => {
  test("renders 3 rows descending by createdAt with filename + duration + profile", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({ ...installRecordingsMock(SEED) });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    const list = page.getByTestId("recordings-list");
    await expect(list).toBeVisible();

    const rows = list.locator("li");
    await expect(rows).toHaveCount(3);

    // First row is the most-recent recording.
    const first = rows.nth(0);
    await expect(first).toContainText("violin-improv-003.flac");
    await expect(first).toContainText("Violin");
    // 83000 ms → 1:23. The exact format is owned by `lib/duration-format.ts`
    // but it has to render *something* containing "1:23" or "83 s".
    await expect(first).toContainText(/1:23|83\s*s/);

    // Middle row.
    const middle = rows.nth(1);
    await expect(middle).toContainText("guitar-take-002.flac");
    await expect(middle).toContainText("Guitar");

    // Last row is the oldest recording.
    const last = rows.nth(2);
    await expect(last).toContainText("voice-note-001.flac");
    await expect(last).toContainText("Voice");
  });

  test("empty list shows the empty-state copy", async ({ page, mockTauri }) => {
    await mockTauri.install({ ...installRecordingsMock([]) });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();

    const list = page.getByTestId("recordings-list");
    await expect(list).toBeVisible();
    await expect(list.locator("li")).toHaveCount(0);

    // The empty-state lives in a `role="status"` region adjacent to the
    // (empty) list per the recordings-drawer contract.
    const empty = page.getByRole("status").filter({ hasText: /No recordings yet/i });
    await expect(empty).toBeVisible();
    await expect(empty).toContainText(/press the red dot/i);
  });

  test("each row carries a recording-row testid for downstream specs", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({ ...installRecordingsMock(SEED) });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();

    // The shared per-row test id is referenced by the playback / scrubber
    // specs (waveform / scrubber). It must be present so the row
    // contract is locked in early.
    const rows = page.getByTestId("recording-row");
    await expect(rows).toHaveCount(3);
  });

  test("Arrow / Home / End move selection in lock-step with the WAI-ARIA listbox", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({ ...installRecordingsMock(SEED) });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();

    const rows = page.getByTestId("recording-row");
    await expect(rows).toHaveCount(3);

    // Roving tabindex: with no prior selection the first row is the
    // keyboard-reachable one. Focus it, then drive arrow navigation.
    const first = rows.nth(0); // violin-improv-003 (most recent)
    const middle = rows.nth(1); // guitar-take-002
    const last = rows.nth(2); // voice-note-001 (oldest)

    await first.focus();
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowDown");

    // Two ArrowDowns land selection on the third (last) row; aria-selected
    // and the data attribute move together and focus follows.
    await expect(last).toHaveAttribute("data-selected", "true");
    await expect(last).toHaveAttribute("aria-selected", "true");
    await expect(last).toBeFocused();

    // Home jumps selection to the first row.
    await page.keyboard.press("Home");
    await expect(first).toHaveAttribute("aria-selected", "true");
    await expect(last).toHaveAttribute("aria-selected", "false");
    await expect(first).toBeFocused();

    // End jumps selection to the last row.
    await page.keyboard.press("End");
    await expect(last).toHaveAttribute("aria-selected", "true");
    await expect(first).toHaveAttribute("aria-selected", "false");

    // ArrowUp from the last row steps selection back to the middle row.
    await page.keyboard.press("ArrowUp");
    await expect(middle).toHaveAttribute("aria-selected", "true");
    await expect(middle).toBeFocused();
  });

  test("Enter and Space activate a focused row and mount its detail panel", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({ ...installRecordingsMock(SEED) });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();

    const rows = page.getByTestId("recording-row");
    await expect(rows).toHaveCount(3);

    // No selection yet — the detail panel is absent. Focusing a row
    // directly (without arrow navigation, which would select on move)
    // isolates the Enter activation branch: focus and selection differ
    // until the key fires onSelect.
    await expect(page.getByTestId("recording-detail")).toHaveCount(0);

    const middle = rows.nth(1); // guitar-take-002
    await middle.focus();
    await expect(middle).toHaveAttribute("aria-selected", "false");
    await page.keyboard.press("Enter");

    // Enter on the focused option mounts the detail panel for that id.
    await expect(middle).toHaveAttribute("aria-selected", "true");
    await expect(page.getByTestId("recording-detail")).toBeVisible();
    await expect(page.getByTestId("recording-detail-filename")).toContainText(
      "guitar-take-002.flac",
    );

    // Space activates a different focused row — the other half of the
    // Enter/Space activation case. After the Enter selection above the
    // last row is no longer the focusable one, so focus it explicitly.
    const last = rows.nth(2); // voice-note-001
    await last.focus();
    await page.keyboard.press(" ");
    await expect(last).toHaveAttribute("aria-selected", "true");
    await expect(page.getByTestId("recording-detail-filename")).toContainText(
      "voice-note-001.flac",
    );
  });
});
