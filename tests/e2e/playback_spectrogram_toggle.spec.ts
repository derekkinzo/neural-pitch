// Spectrogram toggle spec.
//
// Asserts the spectrogram host starts hidden and empty, and that
// clicking the toggle button:
//   1. Flips `aria-pressed` to "true".
//   2. Lazy-mounts a `<canvas>` inside `#spectrogram-host` (the dynamic
//      `wavesurfer.js/dist/plugins/spectrogram.esm.js` import lands).
//
// Locator polling — never `Promise.race` against `wavesurfer.on(...)` —
// so the spec is robust against the channel-receiver-closes-early
// invariant.

import { expect, test } from "./fixtures";
import {
  installPlaybackMock,
  installPlaybackRoutes,
  installRecordingsMock,
  type MockRecording,
} from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const SEED: MockRecording[] = [
  {
    id: "rec-spec-001",
    filename: "voice-spec-001.flac",
    createdAt: NOW - 3 * 60 * 1000,
    durationMs: 1_000,
    sampleRateHz: 8000,
    channels: 1,
    bitDepth: 16,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

test.describe("playback panel — spectrogram toggle", () => {
  test("spectrogram host is initially hidden, populates on toggle", async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installPlaybackMock(),
    });
    await installPlaybackRoutes(page);

    await page.goto("/");
    await page.getByTestId("library-trigger").click();
    await expect(page.getByTestId("recordings-list")).toBeVisible();
    await page.getByTestId("recording-row").first().click();

    await expect(page.getByTestId("playback-panel")).toBeVisible();

    const host = page.locator("#spectrogram-host");
    await expect(host).toHaveAttribute("hidden", "");
    await expect(host.locator("canvas")).toHaveCount(0);

    const toggle = page.getByTestId("spectrogram-toggle");
    await expect(toggle).toBeVisible();
    await expect(toggle).toHaveAttribute("aria-pressed", "false");

    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-pressed", "true");

    // Lazy plugin import lands a <canvas> child inside the host.
    await expect(host.locator("canvas").first()).toBeVisible({ timeout: 4000 });
  });
});
