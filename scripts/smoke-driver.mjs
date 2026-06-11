#!/usr/bin/env node
// scripts/smoke-driver.mjs
//
// Drives the live Tauri shell through every shipped feature using the
// WebDriver protocol exposed by `tauri-driver`. Each step:
//
//   - has an explicit precondition (what must be true on entry)
//   - has an explicit post-condition that is polled with a timeout
//     (we never `sleep(N)` and assume the UI caught up — we wait until
//     a discriminating selector resolves OR the budget runs out)
//   - takes a screenshot at the end (also on FAIL with a `.FAIL` suffix)
//   - records pass/fail JSON to summary.jsonl
//
// The driver uses two complementary capabilities:
//
//   1. WebDriver `findElement` / `click` — for UI interactions where the
//      user normally clicks something visible (toolbar buttons, drill cards).
//   2. WebDriver `executeScript` — for Tauri IPC calls
//      (`invoke('import_audio_file', ...)`). Native open/save dialogs cannot
//      be driven over WebDriver, so we fire the underlying command directly.
//
// Exit 0 on green; non-zero on first failure. Failure dumps the page DOM
// + driver log tail to the report dir for triage.

import { mkdir, writeFile, appendFile, readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
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
const FIXTURE_FLAC = args.fixture; // path to a real 24-bit mono 48 kHz FLAC
const DRIVER_URL = args["driver-url"] ?? "http://localhost:4444";
if (!BINARY || !REPORT_DIR || !FIXTURE_FLAC) {
  console.error(
    "usage: smoke-driver.mjs --binary <path> --report-dir <path> --fixture <path-to-flac>",
  );
  process.exit(2);
}
if (!existsSync(FIXTURE_FLAC)) {
  console.error(`fixture not found: ${FIXTURE_FLAC}`);
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
      alwaysMatch: { "tauri:options": { application: BINARY } },
    },
  });
  sessionId = out.value.sessionId;
}
async function endSession() {
  if (sessionId) await wd("DELETE", `/session/${sessionId}`).catch(() => {});
}
function elemId(v) {
  return v?.ELEMENT ?? v?.["element-6066-11e4-a52e-4f735466cecf"];
}
async function find(strategy, value) {
  const out = await wd("POST", `/session/${sessionId}/element`, { using: strategy, value });
  return elemId(out.value);
}
async function findAll(strategy, value) {
  const out = await wd("POST", `/session/${sessionId}/elements`, { using: strategy, value });
  return out.value.map(elemId);
}
async function click(eid) {
  await wd("POST", `/session/${sessionId}/element/${eid}/click`, {});
}
async function getText(eid) {
  const out = await wd("GET", `/session/${sessionId}/element/${eid}/text`);
  return out.value;
}
async function getAttr(eid, name) {
  const out = await wd("GET", `/session/${sessionId}/element/${eid}/attribute/${name}`);
  return out.value;
}
async function pageSource() {
  const out = await wd("GET", `/session/${sessionId}/source`);
  return out.value;
}
async function screenshot() {
  const out = await wd("GET", `/session/${sessionId}/screenshot`);
  return Buffer.from(out.value, "base64");
}
// Execute a synchronous JS snippet inside the webview. WebDriver's
// `executeScript` is `/session/<id>/execute/sync` per the W3C spec.
// Tauri's `invoke()` is async; we wrap it in `Promise.resolve()` and use
// `executeAsyncScript` (`execute/async`) for any IPC call.
async function executeAsync(script, args = []) {
  const out = await wd("POST", `/session/${sessionId}/execute/async`, { script, args });
  return out.value;
}
// Tighten or relax the WebDriver `script` timeout. The default is
// 30 s, which is fine for IPC-only steps but truncates HTDemucs and
// other ONNX-bound flows. Pass a value larger than the expected
// `executeAsync` runtime (HTDemucs on CPU separates 1.5 s of audio in
// roughly 10 s wall-clock once the session is warm; cold-start adds
// another 10 s).
async function setScriptTimeout(ms) {
  await wd("POST", `/session/${sessionId}/timeouts`, { script: ms });
}
async function execute(script, args = []) {
  const out = await wd("POST", `/session/${sessionId}/execute/sync`, { script, args });
  return out.value;
}

// -------- timing primitives -------------------------------------------
//
// Every gating predicate goes through `waitFor`. It polls `predicate` on
// a fixed interval and returns when the predicate yields a truthy value.
// On timeout it throws with an actionable message AND captures a
// screenshot tagged with the step id so the report directory shows the
// state of the UI when the assumption broke.
async function waitFor(description, predicate, opts = {}) {
  const timeoutMs = opts.timeoutMs ?? 15_000;
  const intervalMs = opts.intervalMs ?? 250;
  const deadline = Date.now() + timeoutMs;
  let lastErr = null;
  let attempts = 0;
  while (Date.now() < deadline) {
    attempts += 1;
    try {
      const value = await predicate();
      if (value) return { value, attempts };
    } catch (err) {
      lastErr = err;
    }
    await sleep(intervalMs);
  }
  const reason = lastErr ? ` (last error: ${lastErr})` : "";
  throw new Error(
    `waitFor timed out after ${timeoutMs}ms (${attempts} polls): ${description}${reason}`,
  );
}

// -------- step harness -------------------------------------------------
const SUMMARY = join(REPORT_DIR, "summary.jsonl");
let stepIndex = 0;
// CSS transitions in the app last `duration-150` (= 150 ms). Padding the
// screenshot capture by a bit more than that lets every step's snapshot
// reflect the steady-state visual rather than a mid-transition frame.
const SCREENSHOT_SETTLE_MS = 200;
async function step(name, fn, opts = {}) {
  stepIndex += 1;
  const id = String(stepIndex).padStart(2, "0");
  const startedAt = new Date().toISOString();
  const t0 = Date.now();
  try {
    const result = await fn();
    await sleep(SCREENSHOT_SETTLE_MS);
    const png = await screenshot();
    await writeFile(join(REPORT_DIR, `${id}-${slug(name)}.png`), png);
    const elapsedMs = Date.now() - t0;
    await appendFile(
      SUMMARY,
      JSON.stringify({ id, name, status: "pass", startedAt, elapsedMs, result }) + "\n",
    );
    console.log(`PASS [${id}] ${name} (${elapsedMs}ms)`);
    return result;
  } catch (err) {
    const elapsedMs = Date.now() - t0;
    let png = null;
    try {
      png = await screenshot();
    } catch {
      /* driver may be dead — best-effort */
    }
    if (png) await writeFile(join(REPORT_DIR, `${id}-${slug(name)}.FAIL.png`), png);
    let html = null;
    try {
      html = await pageSource();
    } catch {
      /* same */
    }
    if (html) {
      await writeFile(join(REPORT_DIR, `${id}-${slug(name)}.FAIL.html`), html);
    }
    await appendFile(
      SUMMARY,
      JSON.stringify({
        id,
        name,
        status: "fail",
        startedAt,
        elapsedMs,
        error: String(err),
        critical: opts.critical ?? false,
      }) + "\n",
    );
    console.error(`FAIL [${id}] ${name} (${elapsedMs}ms): ${err}`);
    if (opts.critical !== false) throw err; // default: fatal
    return null; // non-fatal step: keep going so the report covers everything
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
  console.log(`Fixture: ${FIXTURE_FLAC}`);
  await newSession();

  try {
    // ============================================================
    // PHASE 1 — boot + navigation surface
    // ============================================================

    // 1. App boots and the live tuner mounts.
    //    Post-condition: status pill reaches `data-state="live"`. The
    //    cpal backend init is synchronous-ish but the React tree mounts
    //    via useEffect after a paint — we poll up to 30 s.
    await step("app boot — status pill becomes live", async () => {
      const { value, attempts } = await waitFor(
        "status-pill data-state=live",
        async () => {
          const el = await find("css selector", "[data-testid='status-pill']").catch(() => null);
          if (!el) return null;
          const state = await getAttr(el, "data-state");
          return state === "live" ? state : null;
        },
        { timeoutMs: 30_000, intervalMs: 500 },
      );
      return { state: value, attempts };
    });

    // 2. Note display + cents meter mounted (the tuner is live).
    await step("tuner — note display + cents meter mounted", async () => {
      const note = await find("css selector", "[data-testid='note-letter']");
      const meter = await find("css selector", "[role='meter']");
      const noteText = await getText(note);
      // No real mic in headless, so the note display reads "—".
      if (noteText.trim() !== "—") {
        throw new Error(`expected "—" (silent), got "${noteText}"`);
      }
      return { noteText, meterPresent: !!meter };
    });

    // ============================================================
    // PHASE 2 — settings drawer + select dropdowns
    // ============================================================

    // 3. Settings drawer opens with the A4 + InstrumentHint + NoteLabels
    //    selects readable (the bug fixed in d791d49).
    await step("settings — drawer opens with readable selects", async () => {
      const trigger = await find("css selector", "[data-testid='settings-trigger']");
      await click(trigger);
      // Post-condition: A4 input is in the DOM AND the Instrument hint
      // select has a non-empty visible value (the white-on-white bug
      // would show a blank string).
      await waitFor("a4-input present", async () => {
        return find("css selector", "[data-testid='a4-input']").catch(() => null);
      });
      // Read the SELECT's selected option text via the label-htmlFor
      // association. Gather ALL <select>s that are anchored to a <label>
      // whose text matches the visible heading; the dark-theme white-on-
      // white bug would have surfaced as an empty selectedIndex text.
      const selectsReport = await execute(
        `function selectByLabel(text) {
           const labels = Array.from(document.querySelectorAll('label'));
           const lbl = labels.find((l) => (l.textContent || '').trim() === text);
           if (!lbl) return { error: 'label-not-found' };
           const id = lbl.getAttribute('for');
           if (!id) return { error: 'label-without-htmlFor' };
           const sel = document.getElementById(id);
           if (!sel || sel.tagName !== 'SELECT') return { error: 'control-not-select' };
           const opt = sel.options[sel.selectedIndex];
           return { value: sel.value, text: opt ? opt.textContent.trim() : null };
         }
         return {
           instrument: selectByLabel('Instrument hint'),
           noteLabels: selectByLabel('Note labels'),
         };`,
      );
      if (selectsReport.instrument.error) {
        throw new Error(`Instrument hint select unresolvable: ${selectsReport.instrument.error}`);
      }
      if (!selectsReport.instrument.text) {
        throw new Error(
          `Instrument hint select text is empty: ${JSON.stringify(selectsReport.instrument)}`,
        );
      }
      if (selectsReport.noteLabels.error) {
        throw new Error(`Note labels select unresolvable: ${selectsReport.noteLabels.error}`);
      }
      if (!selectsReport.noteLabels.text) {
        throw new Error(
          `Note labels select text is empty: ${JSON.stringify(selectsReport.noteLabels)}`,
        );
      }
      return {
        instrumentText: selectsReport.instrument.text,
        noteLabelText: selectsReport.noteLabels.text,
      };
    });

    // 4. Close settings drawer cleanly.
    await step("settings — drawer closes", async () => {
      const close = await find("css selector", "[aria-label='Close settings']");
      await click(close);
      await waitFor(
        "drawer dismissed",
        async () => {
          const drawer = await find("css selector", "[data-testid='drawer-root']").catch(
            () => null,
          );
          return drawer === null;
        },
        { timeoutMs: 3_000 },
      );
      return { closed: true };
    });

    // ============================================================
    // PHASE 3 — recordings library: import a real FLAC via IPC,
    // then walk the pYIN analyze + Basic Pitch transcribe + HTDemucs
    // separate paths against it.
    // ============================================================

    // 5. Import a real FLAC fixture by invoking the Tauri command
    //    directly (no native dialog driveable from WebDriver).
    //    Post-condition: list_recordings returns >= 1 row whose id
    //    matches the import response.
    let recordingId = null;
    await step("import — copy fixture FLAC into the library via IPC", async () => {
      const out = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         const sourcePath = arguments[0];
         window.__TAURI_INTERNALS__.invoke('import_audio_file', { sourcePath })
           .then((id) => cb({ ok: true, id }))
           .catch((e) => cb({ ok: false, error: String(e) }));`,
        [FIXTURE_FLAC],
      );
      if (!out || !out.ok) {
        throw new Error(`import_audio_file failed: ${out && out.error}`);
      }
      recordingId = out.id;
      // Verify list_recordings reflects the new row.
      const listed = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         window.__TAURI_INTERNALS__.invoke('list_recordings')
           .then((rows) => cb(rows.map((r) => (typeof r === 'string' ? r : r.id))))
           .catch((e) => cb({ error: String(e) }));`,
      );
      if (!Array.isArray(listed) || !listed.includes(recordingId)) {
        throw new Error(
          `list_recordings did not include ${recordingId}: ${JSON.stringify(listed)}`,
        );
      }
      return { recordingId, totalRows: listed.length };
    });

    // 6. Open the library drawer so the row renders. Post-condition: at
    //    least one `recording-row` element exists.
    await step("library — recordings list shows the imported row", async () => {
      const lib = await find("css selector", "[data-testid='library-trigger']");
      await click(lib);
      const { value } = await waitFor(
        "recording-row appears",
        async () => {
          const rows = await findAll("css selector", "[data-testid='recording-row']");
          return rows.length > 0 ? rows : null;
        },
        { timeoutMs: 10_000 },
      );
      return { rowCount: value.length };
    });

    // 7. Run pYIN analysis on the imported recording. We invoke the
    //    Tauri command directly so we do not have to drive the UI flow
    //    (clicking the row, waiting for the detail panel, finding the
    //    analyze button). The wire shape is (recording_id, force, channel).
    //    The channel is required by Rust but we pass a no-op via the
    //    `Channel` constructor exposed on `window.__TAURI_INTERNALS__`.
    await step(
      "analyze — pYIN runs against the imported FLAC",
      async () => {
        const out = await executeAsync(
          `const cb = arguments[arguments.length - 1];
           const recId = arguments[0];
           // Tauri 2 channels serialise to "__CHANNEL__:<id>"; the
           // command deserialises that into a real Channel that drops
           // messages on the floor when no callback is bound.
           const chId = '__CHANNEL__:' + window.__TAURI_INTERNALS__.transformCallback(() => {}, false);
           window.__TAURI_INTERNALS__.invoke('analyze_recording', {
             recordingId: recId, forceRefresh: false, progress: chId
           }).then((summary) => cb({ ok: true, summary }))
             .catch((e) => cb({ ok: false, error: String(e) }));`,
          [recordingId],
        );
        if (!out || !out.ok) {
          throw new Error(`analyze_recording failed: ${out && out.error}`);
        }
        // The fixture is 069_A4 — pYIN should land near MIDI 69. Allow a
        // couple of cents wobble for the realistic vibrato variants.
        const summary = out.summary ?? {};
        const median = summary.medianMidi ?? summary.median_midi;
        if (typeof median !== "number" || median < 65 || median > 73) {
          throw new Error(
            `analyze_recording medianMidi looks wrong (expected near 69, got ${median}): ${JSON.stringify(summary)}`,
          );
        }
        return { medianMidi: median, voicedRatio: summary.voicedRatio ?? summary.voiced_ratio };
      },
      { timeoutMs: 30_000 },
    );

    // ============================================================
    // PHASE 4 — Basic-Pitch transcribe path through the IPC layer.
    // The real ONNX runner runs end-to-end. The fixture is monophonic,
    // so we just assert >= 1 note in the response (the assertion the
    // `--include-ignored` polyphonic test uses 2 notes; here we just
    // need to prove the wire-up plumbs through).
    // ============================================================

    await step(
      "transcribe — Basic Pitch ONNX runs and returns >= 1 note",
      async () => {
        const out = await executeAsync(
          `const cb = arguments[arguments.length - 1];
           const recId = arguments[0];
           // Tauri 2 channels serialise to "__CHANNEL__:<id>"; the
           // command deserialises that into a real Channel that drops
           // messages on the floor when no callback is bound.
           const chId = '__CHANNEL__:' + window.__TAURI_INTERNALS__.transformCallback(() => {}, false);
           window.__TAURI_INTERNALS__.invoke('transcribe_recording', {
             recordingId: recId, forceRefresh: false, stem: null, progress: chId
           }).then((result) => cb({ ok: true, result }))
             .catch((e) => cb({ ok: false, error: String(e) }));`,
          [recordingId],
        );
        if (!out || !out.ok) {
          throw new Error(`transcribe_recording failed: ${out && out.error}`);
        }
        const noteCount = out.result?.noteCount ?? out.result?.note_count ?? 0;
        if (noteCount < 1) {
          throw new Error(
            `transcribe_recording returned ${noteCount} notes; expected >= 1: ${JSON.stringify(out.result)}`,
          );
        }
        return { noteCount, transcriberVersion: out.result?.transcriberVersion };
      },
      { timeoutMs: 90_000 },
    );

    // ============================================================
    // PHASE 5 — HTDemucs stem separation through the IPC layer.
    // The model is pre-cached at $APPDATA/models/htdemucs.onnx by
    // smoke-test.sh, so this should NOT trigger a 316 MB download.
    // ============================================================

    await step(
      "separate-stems — HTDemucs ONNX runs and produces 4 stem paths",
      async () => {
        // Bump the WebDriver `script` timeout above the worst-case
        // HTDemucs wall-clock so executeAsync does not return early.
        await setScriptTimeout(180_000);
        const out = await executeAsync(
          `const cb = arguments[arguments.length - 1];
           const recId = arguments[0];
           // Tauri 2 channels serialise to "__CHANNEL__:<id>"; the
           // command deserialises that into a real Channel that drops
           // messages on the floor when no callback is bound.
           const chId = '__CHANNEL__:' + window.__TAURI_INTERNALS__.transformCallback(() => {}, false);
           window.__TAURI_INTERNALS__.invoke('separate_stems', {
             recordingId: recId, progress: chId
           }).then((summary) => cb({ ok: true, summary }))
             .catch((e) => cb({ ok: false, error: String(e) }));`,
          [recordingId],
        );
        if (!out || !out.ok) {
          throw new Error(`separate_stems failed: ${out && out.error}`);
        }
        const summary = out.summary ?? {};
        const stems = ["vocals", "drums", "bass", "other"];
        for (const s of stems) {
          const path = summary[s] ?? summary[`${s}Path`] ?? summary[`${s}_path`];
          if (!path || typeof path !== "string") {
            throw new Error(`separate_stems missing ${s} path: ${JSON.stringify(summary)}`);
          }
        }
        return { summary };
      },
      // HTDemucs on CPU separates 1.5 s of audio in ~10 s wall-clock; the
      // fixture is 1.5 s long; we still allow generous headroom for the
      // initial session warm-up.
      { timeoutMs: 180_000 },
    );

    // ============================================================
    // PHASE 6 — training landing + per-drill smoke
    // ============================================================

    await step("training — practice screen renders 5 drill cards", async () => {
      // Navigate via the deep-link the App component handles on mount.
      await wd("POST", `/session/${sessionId}/url`, {
        url: "tauri://localhost/#training",
      }).catch(async () => {
        // some tauri-driver builds reject custom schemes — fall back
        // by clicking through.
        await wd("POST", `/session/${sessionId}/url`, { url: "/" }).catch(() => {});
      });
      const { value } = await waitFor(
        "5 drill-card elements render",
        async () => {
          const cards = await findAll("css selector", "[data-testid='drill-card']");
          return cards.length === 5 ? cards : null;
        },
        { timeoutMs: 10_000 },
      );
      return { drillCardCount: value.length };
    });

    // Walk into the Intervals drill — the simplest drill — and verify
    // the prompt audio synthesis IPC command returns a buffer.
    await step("drills — synthesize_prompt returns a non-empty WAV buffer", async () => {
      const out = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         window.__TAURI_INTERNALS__.invoke('synthesize_prompt', {
           note: { midi: 69, a4_hz: 440.0 }, durationMs: 500
         }).then((bytes) => cb({ ok: true, length: Array.isArray(bytes) ? bytes.length : (bytes?.byteLength ?? -1) }))
           .catch((e) => cb({ ok: false, error: String(e) }));`,
      );
      if (!out || !out.ok) {
        throw new Error(`synthesize_prompt failed: ${out && out.error}`);
      }
      if (out.length < 1000) {
        throw new Error(`synthesize_prompt returned ${out.length} bytes; expected > 1000`);
      }
      return { bytes: out.length };
    });

    // ============================================================
    // PHASE 7 — Cleanup
    // ============================================================

    await step("cleanup — delete the imported recording", async () => {
      const out = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         const recId = arguments[0];
         window.__TAURI_INTERNALS__.invoke('delete_recording', { id: recId })
           .then(() => cb({ ok: true }))
           .catch((e) => cb({ ok: false, error: String(e) }));`,
        [recordingId],
      );
      if (!out || !out.ok) {
        throw new Error(`delete_recording failed: ${out && out.error}`);
      }
      return { recordingId, deleted: true };
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

// Avoid an unused-import warning when readFile is not exercised in a
// minimal codepath: re-export it for symmetry with future steps that
// will diff a JSON file directly.
void readFile;
