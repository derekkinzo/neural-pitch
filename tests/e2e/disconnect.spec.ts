// DeviceDisconnectToast — auto-render on Disconnected, dismiss on Connected,
// and reconnect via configure({ device: "default" }).
//
// Cross-references:
//   docs/design/TEST-PLAN.md §6.2 (user flows / a11y)
//   docs/design/DESIGN.md §9.3 (audio backend events — recovery path)

import { test, expect } from "./fixtures";
import { getInvokeCalls, pushDeviceEvent } from "./helpers/tauri-mock";

test.describe("disconnect — toast + reconnect", () => {
  test("Disconnected event surfaces toast; Reconnect invokes configure", async ({
    page,
    mockTauri,
    axe,
  }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    // No toast initially.
    await expect(page.getByTestId("disconnect-toast")).toHaveCount(0);

    await pushDeviceEvent(page, { type: "Disconnected" });

    const toast = page.getByTestId("disconnect-toast");
    await expect(toast).toBeVisible();
    // The toast is announced immediately via role="alert" (which implies
    // aria-live="assertive") so screen-reader users hear the disconnect
    // before any in-progress speech finishes — see DESIGN.md §9.3.
    await expect(toast).toHaveAttribute("role", "alert");
    await expect(toast).toContainText(/Audio device disconnected/);

    const reconnect = page.getByRole("button", { name: "Reconnect to default microphone" });
    await reconnect.click();

    await expect
      .poll(async () => (await getInvokeCalls(page, "configure")).length, { timeout: 2000 })
      .toBeGreaterThanOrEqual(1);

    const calls = await getInvokeCalls(page, "configure");
    const last = calls[calls.length - 1];
    expect(last?.args["device"]).toBe("default");

    // Toast auto-dismisses once Connected arrives (which clears
    // deviceStatus back to "ok").
    await pushDeviceEvent(page, { type: "Connected" });
    await expect(page.getByTestId("disconnect-toast")).toHaveCount(0);

    const results = await axe.analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });
});
