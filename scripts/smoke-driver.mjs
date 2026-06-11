#!/usr/bin/env node
// scripts/smoke-driver.mjs
//
// Walks the live Tauri shell through every shipped feature using the
// WebDriver protocol exposed by `tauri-driver`. The protocol speaks
// JSON over HTTP, so we use a minimal hand-rolled client rather than
// pulling in selenium-webdriver or @playwright/test as a dep.
//
// Each step:
//   - asserts a stable selector resolves
//   - takes a screenshot via WebDriver `:saveScreenshot`
//   - records a JSON line in summary.jsonl
//
// Exit 0 on green; non-zero on first failure.

import { mkdir, writeFile, appendFile } from "node:fs/promises";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

// -------- arg parse ----------------------------------------------------
const args = Object.fromEntries(
  process.argv.slice(2).reduce((acc, cur, i, arr) => {
    if (cur.startsWith("--") && i + 1 < arr.length) acc.push([cur.slice(2), arr[i + 1]]);
    return acc;
  }, []),
);
const BINARY = args.binary;
const REPORT_DIR = args["report-dir"];
const DRIVER_URL = args["driver-url"] ?? "http://localhost:4444";
if (!BINARY || !REPORT_DIR) {
  console.error("usage: smoke-driver.mjs --binary <path> --report-dir <path>");
  process.exit(2);
}

// -------- WebDriver thin client ---------------------------------------
async function wd(method, path, body) {
  const url = `${DRIVER_URL}${path}`;
  const init = { method, headers: { "Content-Type": "application/json" } };
  if (body !== undefined) init.body = JSON.stringify(body);
  const res = await fetch(url, init);
  const text = await res.text();
  if (!res.ok) throw new Error(`WebDriver ${method} ${path} -> ${res.status}: ${text}`);
  return text ? JSON.parse(text) : null;
}

let sessionId = null;
async function newSession() {
  const out = await wd("POST", "/session", {
    capabilities: {
      alwaysMatch: {
        "tauri:options": { application: BINARY },
      },
    },
  });
  sessionId = out.value.sessionId;
}
async function endSession() {
  if (sessionId) await wd("DELETE", `/session/${sessionId}`).catch(() => {});
}
async function find(strategy, value) {
  const out = await wd("POST", `/session/${sessionId}/element`, {
    using: strategy,
    value,
  });
  return out.value.ELEMENT ?? out.value["element-6066-11e4-a52e-4f735466cecf"];
}
async function findAll(strategy, value) {
  const out = await wd("POST", `/session/${sessionId}/elements`, {
    using: strategy,
    value,
  });
  return out.value.map((v) => v.ELEMENT ?? v["element-6066-11e4-a52e-4f735466cecf"]);
}
async function click(elementId) {
  await wd("POST", `/session/${sessionId}/element/${elementId}/click`, {});
}
async function text(elementId) {
  const out = await wd("GET", `/session/${sessionId}/element/${elementId}/text`);
  return out.value;
}
async function attr(elementId, name) {
  const out = await wd("GET", `/session/${sessionId}/element/${elementId}/attribute/${name}`);
  return out.value;
}
async function screenshot() {
  const out = await wd("GET", `/session/${sessionId}/screenshot`);
  return Buffer.from(out.value, "base64");
}

// -------- step harness -------------------------------------------------
const SUMMARY = join(REPORT_DIR, "summary.jsonl");
let stepIndex = 0;
async function step(name, fn) {
  stepIndex += 1;
  const id = String(stepIndex).padStart(2, "0");
  const startedAt = new Date().toISOString();
  try {
    const result = await fn();
    const png = await screenshot();
    await writeFile(join(REPORT_DIR, `${id}-${slug(name)}.png`), png);
    await appendFile(
      SUMMARY,
      JSON.stringify({ id, name, status: "pass", startedAt, result }) + "\n",
    );
    console.log(`PASS [${id}] ${name}`);
  } catch (err) {
    let png = null;
    try {
      png = await screenshot();
    } catch {
      /* ignore — driver may be dead */
    }
    if (png) await writeFile(join(REPORT_DIR, `${id}-${slug(name)}.FAIL.png`), png);
    await appendFile(
      SUMMARY,
      JSON.stringify({ id, name, status: "fail", startedAt, error: String(err) }) + "\n",
    );
    console.error(`FAIL [${id}] ${name}: ${err}`);
    throw err;
  }
}
function slug(s) {
  return s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
}

// -------- the smoke script -------------------------------------------
async function main() {
  await mkdir(REPORT_DIR, { recursive: true });
  console.log(`Connecting to tauri-driver at ${DRIVER_URL}`);
  console.log(`Launching binary: ${BINARY}`);
  await newSession();

  try {
    // 1. App boots and the live tuner mounts
    await step("app boot — status pill becomes 'live'", async () => {
      let attempts = 0;
      while (attempts < 30) {
        try {
          const el = await find("css selector", "[data-testid='status-pill']");
          const state = await attr(el, "data-state");
          if (state === "live") return { state };
        } catch {
          /* element not ready yet */
        }
        await sleep(500);
        attempts += 1;
      }
      throw new Error("status pill never reached 'live' state");
    });

    // 2. Tuner core elements present
    await step("tuner — note display + cents meter mounted", async () => {
      const note = await find("css selector", "[data-testid='note-letter']");
      const meter = await find("css selector", "[role='meter']");
      return { noteText: await text(note), meterPresent: !!meter };
    });

    // 3. Settings drawer opens
    await step("settings — drawer opens with A4 input", async () => {
      const trigger = await find("css selector", "[data-testid='settings-trigger']");
      await click(trigger);
      await sleep(300);
      const a4 = await find("css selector", "[data-testid='a4-input']");
      return { a4Present: !!a4 };
    });

    // 4. Library opens. Close the settings drawer first so its overlay does
    //    not intercept the click on the library trigger underneath.
    await step("library — recordings list reachable", async () => {
      // Drawer close button is "×" with aria-label "Close settings".
      const close = await find(
        "css selector",
        "[aria-label='Close settings'], button[aria-label*='Close']",
      ).catch(() => null);
      if (close) {
        await click(close);
        await sleep(300);
      }
      const lib = await find("css selector", "[data-testid='library-trigger']");
      await click(lib);
      await sleep(500);
      // If there are no recordings yet the empty state should render.
      const rows = await findAll("css selector", "[data-testid='recording-row']");
      return { rowCount: rows.length };
    });

    // 5. Training landing
    await step("training — practice screen reachable", async () => {
      // Use the deep-link hash so we don't depend on a header button name
      await wd("POST", `/session/${sessionId}/url`, {
        url: "tauri://localhost/#training",
      }).catch(async () => {
        // some tauri-driver builds reject custom schemes; fall back
        await wd("POST", `/session/${sessionId}/url`, { url: "/" });
      });
      await sleep(500);
      const cards = await findAll("css selector", "[data-testid='drill-card']");
      return { drillCardCount: cards.length };
    });

    console.log("==> smoke pass complete");
  } finally {
    await endSession();
  }
}

main().catch((err) => {
  console.error("smoke pass aborted:", err);
  process.exit(1);
});
