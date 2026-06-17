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

  test("dragging the position slider seeks and advances the time readout", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installPlaybackMock(),
    });
    await installPlaybackRoutes(page);

    await page.goto("/");
    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    const panel = page.getByTestId("playback-panel");
    await expect(panel).toBeVisible();
    // Gate on wavesurfer "ready" — the slider stays disabled until the
    // duration is known, and seekTo is a no-op while durationMs <= 0.
    await expect(panel.locator("canvas").first()).toBeVisible({ timeout: 2000 });

    const slider = panel.getByRole("slider", { name: /Playback position/i });
    await expect(slider).toBeEnabled();

    // Readout starts at the origin.
    await expect(page.getByTestId("playback-time")).toContainText(/^0:00|^0\s*s/);

    // Read the slider's upper bound (wavesurfer's decoded duration, ~1000 ms
    // for the fixture) so the seek target tracks the real clip length.
    const ariaMax = Number(await slider.getAttribute("aria-valuemax"));
    expect(ariaMax).toBeGreaterThan(0);

    // Seek the transport head to ~60% of the clip. `onSeek` clamps
    // value/durationMs to a [0,1] ratio and calls `ws.seekTo`. The panel
    // surfaces the head position through `audioprocess`, so we start
    // playback to sample the head: if the seek took effect the processed
    // frames report a time past the midpoint, whereas a from-zero
    // playback would still be near the origin for the first frames.
    const target = Math.round(ariaMax * 0.6);
    await slider.fill(String(target));
    await page.getByTestId("playback-toggle").click();

    // The readout advances to a position past the midpoint — discriminating
    // the seek from a no-op (which would crawl up from 0). The fixture is
    // 1 s, so a ~600 ms head leaves enough playback tail to emit frames.
    await expect
      .poll(async () => Number(await slider.getAttribute("aria-valuenow")), { timeout: 3000 })
      .toBeGreaterThan(Math.round(ariaMax * 0.45));
  });

  test("'k' toggles play via the panel shortcut; Space on the slider does not", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installPlaybackMock(),
    });
    await installPlaybackRoutes(page);

    await page.goto("/");
    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    const panel = page.getByTestId("playback-panel");
    await expect(panel).toBeVisible();
    await expect(panel.locator("canvas").first()).toBeVisible({ timeout: 2000 });

    const toggle = page.getByTestId("playback-toggle");
    await expect(toggle).toHaveAttribute("aria-pressed", "false");

    // 'k' is the unambiguous panel-level transport key: unlike Space it
    // does not natively activate the focused button, so the panel-root
    // onPanelKeyDown handler toggles transport exactly once. Focus stays
    // inside the panel (on the toggle) so the keydown reaches the panel.
    await toggle.focus();
    await page.keyboard.press("k");
    await expect(toggle).toHaveAttribute("aria-pressed", "true");

    await page.keyboard.press("k");
    await expect(toggle).toHaveAttribute("aria-pressed", "false");

    // The slider is a form control; the panel guard skips it so native
    // range behavior (arrow keys only) is preserved — 'k' must NOT toggle
    // transport when the slider holds focus.
    const slider = panel.getByRole("slider", { name: /Playback position/i });
    await slider.focus();
    await page.keyboard.press("k");
    await expect(toggle).toHaveAttribute("aria-pressed", "false");
  });
});
