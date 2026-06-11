// PlaybackPanel mount + transport spec.
//
// Asserts the new wavesurfer-driven panel mounts when a recording is
// selected, exposes a play/pause toggle whose `aria-pressed` reflects
// transport state, and renders a `<canvas>` child inside the waveform
// host within 2 s of the row click.
//
// No spec subscribes directly to the synthetic Tauri channel — the
// receiver-closes-early invariant is preserved by exclusively reading
// DOM state via Locator polling.

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
    id: "rec-playback-001",
    filename: "voice-playback-001.flac",
    createdAt: NOW - 4 * 60 * 1000,
    durationMs: 1_000,
    sampleRateHz: 8000,
    channels: 1,
    bitDepth: 16,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

test.describe("playback panel — mount + transport", () => {
  test("opens panel, mounts wavesurfer canvas, toggles play/pause aria-pressed", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installPlaybackMock(),
    });
    await installPlaybackRoutes(page);

    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    await expect(page.getByTestId("recordings-list")).toBeVisible();

    await page.getByTestId("recording-row").first().click();

    const panel = page.getByTestId("playback-panel");
    await expect(panel).toBeVisible();

    // Wavesurfer mounts a <canvas> inside its host within 2 s of "ready".
    await expect(panel.locator("canvas").first()).toBeVisible({ timeout: 2000 });

    const toggle = page.getByTestId("playback-toggle");
    await expect(toggle).toBeVisible();
    await expect(toggle).toHaveAttribute("aria-pressed", "false");

    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-pressed", "true");

    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-pressed", "false");
  });
});
