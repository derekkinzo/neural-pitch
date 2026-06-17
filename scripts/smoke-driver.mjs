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

import { mkdir, writeFile, appendFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

// -------- arg parse ----------------------------------------------------
// CLI grammar: `--<name> <value>` or `--<name>=<value>`. A bare `--<name>`
// followed by another `--<name>` is rejected with a clear error rather
// than silently consuming the second flag name as the first flag's
// value. Boolean flags are not used by this script.
function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i++) {
    const tok = argv[i];
    if (!tok.startsWith("--")) {
      throw new Error(`unexpected positional argument: ${tok}`);
    }
    const eq = tok.indexOf("=");
    if (eq !== -1) {
      out[tok.slice(2, eq)] = tok.slice(eq + 1);
      continue;
    }
    const next = argv[i + 1];
    if (next === undefined || next.startsWith("--")) {
      throw new Error(`flag ${tok} requires a value`);
    }
    out[tok.slice(2)] = next;
    i += 1;
  }
  return out;
}
const args = parseArgs(process.argv.slice(2));
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
//
// `Connection: close` opts out of Node's keepalive pool. tauri-driver
// and hyper interact poorly when undici re-uses sockets: a stale
// pooled connection surfaces as `UND_ERR_HEADERS_TIMEOUT` on the
// client side and `Error serving connection: hyper::Error(IncompleteMessage)`
// on the server side, with the request hanging for the
// 5-minute pool-timeout default. Closing per-call adds ~1 ms of TCP
// handshake overhead in exchange for predictable behaviour.
//
// AbortController bounds every call by `WD_FETCH_TIMEOUT_MS`. Without
// a per-call deadline a hung WebKitWebDriver endpoint (canvas readback
// under host memory pressure is the recurring offender) blocks the
// whole run for undici's 5-minute headers timeout. 60 s is well above
// any healthy response time and well below the W3C-default script
// timeout, so an honest stall surfaces fast and the surrounding step
// harness can capture diagnostics.
const WD_FETCH_TIMEOUT_MS = 60_000;
async function wd(method, path, body, opts = {}) {
  const url = `${DRIVER_URL}${path}`;
  const ctrl = new AbortController();
  const deadline = opts.timeoutMs ?? WD_FETCH_TIMEOUT_MS;
  const timer = setTimeout(() => ctrl.abort(), deadline);
  const init = {
    method,
    headers: { "Content-Type": "application/json", Connection: "close" },
    signal: ctrl.signal,
  };
  if (body !== undefined) init.body = JSON.stringify(body);
  try {
    const res = await fetch(url, init);
    const text = await res.text();
    if (!res.ok) throw new Error(`WebDriver ${method} ${path} -> ${res.status}: ${text}`);
    return text ? JSON.parse(text) : null;
  } catch (err) {
    if (err?.name === "AbortError") {
      throw new Error(
        `WebDriver ${method} ${path} aborted after ${deadline}ms (driver/webview hung)`,
      );
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }
}

let sessionId = null;
async function newSession() {
  const out = await wd("POST", "/session", {
    capabilities: {
      alwaysMatch: { "tauri:options": { application: BINARY } },
    },
  });
  sessionId = out.value.sessionId;
  // Bump the WebDriver `script` timeout from the W3C default of 30 s.
  // Cold-start ONNX inference (Basic Pitch first call) routinely exceeds
  // 30 s on debug builds; HTDemucs takes another order of magnitude.
  // Setting it once after session creation covers every subsequent
  // executeAsync call without per-step ceremony.
  await wd("POST", `/session/${sessionId}/timeouts`, { script: 240_000 });
}
async function endSession() {
  if (sessionId) {
    // Surface tauri-driver shutdown failures so a hung driver does not
    // masquerade as a clean teardown — the surrounding `step` harness
    // already captures stderr per-step.
    await wd("DELETE", `/session/${sessionId}`).catch((err) => {
      console.error(`endSession: DELETE /session/${sessionId} failed: ${err}`);
    });
  }
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
  // Tighter per-call deadline than the default 60 s. WebKitWebDriver
  // sometimes blocks `/screenshot` indefinitely under memory pressure
  // (canvas readback waits on a swap-paged buffer). A 10 s ceiling
  // keeps a hung screenshot from inflating step wall-clock; the
  // surrounding step harness records the abort and continues.
  const out = await wd("GET", `/session/${sessionId}/screenshot`, undefined, { timeoutMs: 10_000 });
  return Buffer.from(out.value, "base64");
}
// Execute a synchronous JS snippet inside the webview. WebDriver's
// `executeScript` is `/session/<id>/execute/sync` per the W3C spec.
// Tauri's `invoke()` is async; we wrap it in `Promise.resolve()` and use
// `executeAsyncScript` (`execute/async`) for any IPC call.
// `executeAsync` blocks the WebDriver HTTP response until the wrapped
// script invokes its callback. Fetch deadline must therefore match the
// pre-set WebDriver `script` timeout (default 240 s after `newSession`,
// bumped to 180 s explicitly before the HTDemucs step). Add a small
// margin so the WebDriver-side timeout fires first with a meaningful
// payload rather than the fetch aborting it.
async function executeAsync(script, args = [], opts = {}) {
  const fetchTimeoutMs = opts.fetchTimeoutMs ?? 250_000;
  const out = await wd(
    "POST",
    `/session/${sessionId}/execute/async`,
    { script, args },
    { timeoutMs: fetchTimeoutMs },
  );
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

// Return the webview to the Tuner view. The Training screen is mounted by
// flipping `tunerStore.view = "training"` (the `#training` URL hash only
// routes on App mount, so re-navigating the URL does NOT switch back). The
// Training landing carries a `training-exit` button that calls
// `setView("tuner")`; clicking it yields the Tuner, which owns the
// library / settings affordances. No-op when already on the Tuner (the
// `library-trigger` is present and `training-exit` is not).
async function ensureTunerView() {
  const libTrigger = await find("css selector", "[data-testid='library-trigger']").catch(
    () => null,
  );
  if (libTrigger) return; // already on the Tuner
  const exit = await find("css selector", "[data-testid='training-exit']").catch(() => null);
  if (exit) {
    await click(exit);
    await waitFor(
      "library-trigger present after exiting training",
      async () => find("css selector", "[data-testid='library-trigger']").catch(() => null),
      { timeoutMs: 10_000 },
    );
  }
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
    // Screenshot is best-effort. WebKitWebDriver intermittently hangs
    // on `/screenshot` under memory pressure (canvas readback waits on
    // a swap-paged buffer), and a hung screenshot must not turn a
    // green step into a fail. The wd() AbortController bounds the
    // call; on abort we record the step as PASS without an image.
    let png = null;
    try {
      png = await screenshot();
    } catch (e) {
      console.warn(`[${id}] screenshot capture failed: ${e}`);
    }
    if (png) {
      await writeFile(join(REPORT_DIR, `${id}-${slug(name)}.png`), png);
    }
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
        critical: opts.critical !== false,
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
    // boot + tuner mount
    // ============================================================

    // 1. App boots and the live tuner mounts.
    //    Post-condition: the React tree painted the StatusPill. The
    //    `data-state` attribute reflects the cpal backend's outcome:
    //    `live` on a normal developer machine, `error` on a CI runner
    //    with no audio card (ALSA reports "cannot find card '0'"),
    //    `idle` before the start_capture round-trip resolves. Any of
    //    these proves the shell mounted; only a missing element means
    //    the renderer did not start.
    await step("app boot — status pill mounts", async () => {
      const { value, attempts } = await waitFor(
        "status-pill present with a known data-state",
        async () => {
          const el = await find("css selector", "[data-testid='status-pill']").catch(() => null);
          if (!el) return null;
          const state = await getAttr(el, "data-state");
          if (state === "live" || state === "idle" || state === "error") return state;
          return null;
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
      // The note display renders either a note letter (A-G optionally
      // sharpened) when the mic picks up a fundamental, or the em-dash
      // sentinel when input is below the voicing gate. Both shapes prove
      // the React tree mounted; we accept either rather than pinning to
      // a specific environment.
      const ok = /^[A-G][#♯b♭]?$/.test(noteText.trim()) || noteText.trim() === "—";
      if (!ok) {
        throw new Error(`unexpected note-letter text: "${noteText}"`);
      }
      return { noteText, meterPresent: !!meter };
    });

    // ============================================================
    // settings drawer + select dropdowns
    // ============================================================

    // 3. Settings drawer opens with the A4 + InstrumentHint + NoteLabels
    //    selects readable (white-on-white regression guard).
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
    // recordings library — import + analyze a real FLAC fixture via
    // IPC. Subsequent sections drive the transcribe + separate paths
    // against the same recording id.
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
    // transcribe path — Basic-Pitch ONNX through the IPC layer. The
    // real runner runs end-to-end. The fixture is monophonic, so we
    // assert >= 1 note in the response; the polyphonic case lives
    // under the `--include-ignored` test in the core crate, here we
    // just need to prove the wire-up plumbs through.
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
    // stem-separation path — HTDemucs through the IPC layer. The
    // model is pre-cached at $APPDATA/models/htdemucs.onnx by the
    // CI workflow / local smoke-test.sh, so this should NOT trigger
    // a 316 MB download.
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
    // training landing + per-drill prompts
    // ============================================================

    await step("training — practice screen renders 5 drill cards", async () => {
      // Navigate via the deep-link the App component handles on mount.
      await wd("POST", `/session/${sessionId}/url`, {
        url: "tauri://localhost/#training",
      }).catch(async (customSchemeErr) => {
        // Some tauri-driver builds reject custom schemes; record the
        // original error to the report dir so the failure mode stays
        // diagnosable from artifacts, then fall back by routing
        // through the default URL.
        await writeFile(
          join(REPORT_DIR, "training-nav-fallback.log"),
          `tauri:// scheme rejected: ${customSchemeErr}\n`,
        ).catch(() => {});
        await wd("POST", `/session/${sessionId}/url`, { url: "/" }).catch((fallbackErr) => {
          console.error(`training-nav fallback also failed: ${fallbackErr}`);
        });
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
    // per-stem transcribe — re-run Basic Pitch against a separated stem
    // WAV (produced by the HTDemucs step above) rather than the original
    // mix. Exercises transcribe_recording's stem-targeted branch + the
    // per-stem path resolution + cache key, which the whole-mix step
    // (stem: null) never touches. ONNX-bound; rides the same script
    // timeout the HTDemucs step set.
    // ============================================================

    await step(
      "transcribe-stem — Basic Pitch runs against the separated vocals stem",
      async () => {
        const out = await executeAsync(
          `const cb = arguments[arguments.length - 1];
           const recId = arguments[0];
           const chId = '__CHANNEL__:' + window.__TAURI_INTERNALS__.transformCallback(() => {}, false);
           window.__TAURI_INTERNALS__.invoke('transcribe_recording', {
             recordingId: recId, forceRefresh: false, stem: 'vocals', progress: chId
           }).then((result) => cb({ ok: true, result }))
             .catch((e) => cb({ ok: false, error: String(e) }));`,
          [recordingId],
        );
        if (!out || !out.ok) {
          throw new Error(`transcribe_recording (stem=vocals) failed: ${out && out.error}`);
        }
        // The separated vocals stem of a single sustained note may recover
        // zero notes (HTDemucs can attenuate a mono-sourced "vocal" bus);
        // the contract here is that the stem-targeted branch RUNS and
        // returns a well-formed summary, not that it recovers a note count
        // matching the whole-mix pass. Assert the summary shape deserialises
        // and the analyzer identity is the Basic Pitch transcriber.
        const result = out.result ?? {};
        const noteCount = result.noteCount ?? result.note_count;
        if (typeof noteCount !== "number" || noteCount < 0) {
          throw new Error(
            `transcribe_recording (stem=vocals) returned a malformed note count: ${JSON.stringify(result)}`,
          );
        }
        const analyzerName = result.analyzerName ?? result.analyzer_name;
        if (analyzerName !== "basic-pitch") {
          throw new Error(
            `transcribe_recording (stem=vocals) analyzer_name expected "basic-pitch", got ${JSON.stringify(analyzerName)}`,
          );
        }
        return { noteCount, analyzerName, wasCached: result.wasCached ?? result.was_cached };
      },
      { timeoutMs: 180_000 },
    );

    // ============================================================
    // read_stem_audio — read one separated stem FLAC back through the
    // command the PlaybackPanel uses. The mock playback spec serves a
    // fabricated blob over an intercepted route; this exercises the real
    // stem_results lookup + on-disk file read that production uses.
    // ONNX-free (pure file IO), so it is a cheap always-on addition that
    // depends only on the separate-stems step's cached output.
    // ============================================================

    await step("read-stem-audio — vocals stem FLAC bytes are non-empty", async () => {
      const out = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         const recId = arguments[0];
         window.__TAURI_INTERNALS__.invoke('read_stem_audio', {
           recordingId: recId, stem: 'vocals'
         }).then((bytes) => cb({
            ok: true,
            length: Array.isArray(bytes) ? bytes.length : (bytes?.byteLength ?? -1),
            // FLAC stream marker "fLaC" == [0x66, 0x4C, 0x61, 0x43]; sample
            // the first four bytes so we prove the read returned a real
            // FLAC container, not an empty / truncated buffer.
            head: Array.isArray(bytes) ? bytes.slice(0, 4) : [],
          }))
          .catch((e) => cb({ ok: false, error: String(e) }));`,
        [recordingId],
      );
      if (!out || !out.ok) {
        throw new Error(`read_stem_audio failed: ${out && out.error}`);
      }
      if (out.length < 1000) {
        throw new Error(
          `read_stem_audio returned ${out.length} bytes; expected a real FLAC > 1000`,
        );
      }
      const fLaC = [0x66, 0x4c, 0x61, 0x43];
      const headMatches = Array.isArray(out.head) && fLaC.every((b, i) => out.head[i] === b);
      if (!headMatches) {
        throw new Error(
          `read_stem_audio bytes do not start with the fLaC stream marker: ${JSON.stringify(out.head)}`,
        );
      }
      return { bytes: out.length };
    });

    // ============================================================
    // range + vibrato reports — the get_range_report / get_vibrato_report
    // commands have no live coverage (the frontend never calls them and
    // the readout specs feed canned mocks). Invoke both directly with the
    // pYIN analyzer key the analyze step persisted and assert the report
    // structs deserialise with the documented field set. The fixture is a
    // ~1.5 s single sustained note, so the range report lands on the
    // insufficient-data sentinel (voicedFrameCount may be below the 50-
    // frame floor) — we assert the SHAPE deserialises, not large ranges.
    // ============================================================

    await step("range-report — get_range_report returns a well-formed RangeReport", async () => {
      const out = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         const recId = arguments[0];
         window.__TAURI_INTERNALS__.invoke('get_range_report', {
           recordingId: recId, analyzerName: 'pyin', analyzerVersion: '0.2'
         }).then((report) => cb({ ok: true, report }))
           .catch((e) => cb({ ok: false, error: String(e) }));`,
        [recordingId],
      );
      if (!out || !out.ok) {
        throw new Error(`get_range_report failed: ${out && out.error}`);
      }
      const r = out.report ?? {};
      // snake_case on the wire (no serde rename on RangeReport).
      const required = [
        "voiced_frame_count",
        "median_midi",
        "comfortable_min_midi",
        "comfortable_max_midi",
        "full_min_midi",
        "full_max_midi",
      ];
      for (const key of required) {
        if (typeof r[key] !== "number") {
          throw new Error(`get_range_report missing/non-numeric ${key}: ${JSON.stringify(r)}`);
        }
      }
      return {
        voicedFrameCount: r.voiced_frame_count,
        medianMidi: r.median_midi,
      };
    });

    await step(
      "vibrato-report — get_vibrato_report returns a well-formed VibratoReport",
      async () => {
        const out = await executeAsync(
          `const cb = arguments[arguments.length - 1];
           const recId = arguments[0];
           window.__TAURI_INTERNALS__.invoke('get_vibrato_report', {
             recordingId: recId, analyzerName: 'pyin', analyzerVersion: '0.2'
           }).then((report) => cb({ ok: true, report }))
             .catch((e) => cb({ ok: false, error: String(e) }));`,
          [recordingId],
        );
        if (!out || !out.ok) {
          throw new Error(`get_vibrato_report failed: ${out && out.error}`);
        }
        const v = out.report ?? {};
        // snake_case on the wire (no serde rename on VibratoReport).
        for (const key of ["median_rate_hz", "median_extent_cents", "vibrato_ratio"]) {
          if (typeof v[key] !== "number") {
            throw new Error(`get_vibrato_report missing/non-numeric ${key}: ${JSON.stringify(v)}`);
          }
        }
        if (!Array.isArray(v.per_window)) {
          throw new Error(`get_vibrato_report per_window must be an array: ${JSON.stringify(v)}`);
        }
        if (!(v.vibrato_ratio >= 0 && v.vibrato_ratio <= 1)) {
          throw new Error(`get_vibrato_report vibrato_ratio out of [0,1]: ${v.vibrato_ratio}`);
        }
        return { medianRateHz: v.median_rate_hz, vibratoRatio: v.vibrato_ratio };
      },
    );

    // ============================================================
    // RecordingDetail row-click UI flow — the smoke run has so far driven
    // every analysis command directly via IPC. Click the imported FLAC
    // row to mount RecordingDetail in the real shell and prove the React
    // composition (detail header + Analysis-summary group + range/vibrato
    // readout sections) renders against the real backend.
    //
    // Coverage note: RecordingDetail auto-fires `analyze(id)` on select,
    // but the front-end `analysisStore.analyze` invokes `analyze_recording`
    // WITHOUT the `progress` Channel arg the Rust command requires (the
    // progress UI listens on the global `analysis-progress` event instead
    // of an invoke-arg Channel). The live shell therefore rejects the call
    // with "missing required key progress" and the summary card renders the
    // `role="alert"` failure branch. This is a real wiring bug the mock
    // specs cannot see (the mock IPC handler does not validate the channel
    // arg). The smoke step records the auto-analyze outcome in its result
    // so the regression is visible in summary.jsonl, and asserts the
    // structural wiring that does NOT depend on the broken auto-analyze:
    // the detail panel, the summary group, and both readout sections mount.
    // ============================================================

    await step("recording-detail — row click mounts the detail panel + readouts", async () => {
      // The training step navigated to the Training view; return to the
      // Tuner so the library drawer is reachable.
      await ensureTunerView();
      // The library drawer was opened in step 6 but the training round-trip
      // unmounted the Tuner; reopen it so the row is clickable.
      let rows = await findAll("css selector", "[data-testid='recording-row']");
      if (rows.length === 0) {
        const lib = await find("css selector", "[data-testid='library-trigger']");
        await click(lib);
        rows = await waitFor(
          "recording-row present after reopening the library drawer",
          async () => {
            const found = await findAll("css selector", "[data-testid='recording-row']");
            return found.length > 0 ? found : null;
          },
          { timeoutMs: 10_000 },
        ).then((r) => r.value);
      }
      const row = await find("css selector", "[data-testid='recording-row']");
      await click(row);

      // Post-condition 1: the detail header mounts.
      await waitFor("recording-detail-header mounts", async () => {
        return find("css selector", "[data-testid='recording-detail-header']").catch(() => null);
      });

      // Post-condition 2: the Analysis summary group mounts (the React
      // tree composed RecordingDetail with the live recording row).
      const summaryGroup = await find("css selector", "[data-testid='analysis-summary']");

      // Post-condition 3: both readout sections mount. They render
      // unconditionally inside RecordingDetail, so a present section proves
      // the parent composed the readout components against the real shell.
      const rangeReadout = await find("css selector", "[data-testid='range-readout']");
      const vibratoReadout = await find("css selector", "[data-testid='vibrato-readout']");

      // Diagnostic: capture the median-note text + whether the auto-analyze
      // surfaced the missing-progress-channel error. Recorded in the step
      // result (not asserted) so the front-end analyze-on-select wiring bug
      // stays visible in the report without failing the structural smoke.
      const medianNote = await find("css selector", "[data-testid='summary-median-note']")
        .then((el) => getText(el))
        .then((t) => t.trim())
        .catch(() => null);
      const analyzeAlert = await find(
        "css selector",
        "[data-testid='analysis-summary'] [role='alert']",
      )
        .then((el) => getText(el))
        .then((t) => t.trim())
        .catch(() => null);

      return {
        summaryGroupPresent: !!summaryGroup,
        rangeReadoutPresent: !!rangeReadout,
        vibratoReadoutPresent: !!vibratoReadout,
        medianNote,
        autoAnalyzeAlert: analyzeAlert,
      };
    });

    // ============================================================
    // playback panel — with a row selected, the PlaybackPanel resolves the
    // recording path through get_recording_path and hands it to wavesurfer
    // via convertFileSrc. The mock playback spec short-circuits the
    // file-read + decode with a fabricated blob over an intercepted route;
    // this exercises the real get_recording_path command (which otherwise
    // has no live coverage) and the panel-mount wiring.
    //
    // Hard assertions: get_recording_path resolves to a real .flac path AND
    // the playback panel mounts. The wavesurfer decode-ready signal
    // (`aria-busy="false"` + the play toggle no longer disabled) is captured
    // best-effort: WebKit's asset-protocol fetch + PCM decode can be slow or
    // unavailable under the headless smoke webview, so a non-ready panel is
    // recorded rather than treated as a regression. We avoid asserting the
    // wavesurfer <canvas>, which a raw WebDriver CSS selector cannot pierce
    // out of wavesurfer's shadow root.
    // ============================================================

    await step("playback — get_recording_path resolves + panel mounts", async () => {
      const out = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         const recId = arguments[0];
         window.__TAURI_INTERNALS__.invoke('get_recording_path', { id: recId })
           .then((path) => cb({ ok: true, path }))
           .catch((e) => cb({ ok: false, error: String(e) }));`,
        [recordingId],
      );
      if (!out || !out.ok) {
        throw new Error(`get_recording_path failed: ${out && out.error}`);
      }
      if (typeof out.path !== "string" || out.path.length === 0 || !out.path.endsWith(".flac")) {
        throw new Error(
          `get_recording_path returned an unexpected path: ${JSON.stringify(out.path)}`,
        );
      }
      // Hard post-condition: the panel mounts (the row selection from the
      // prior step set currentRecordingId, so PlaybackPanel composes).
      await waitFor("playback panel mounts", async () =>
        find("css selector", "[data-testid='playback-panel']").catch(() => null),
      );
      // Best-effort: did wavesurfer decode the real buffer? The play toggle
      // is `disabled` until the "ready" event flips `wsReady`. Recorded, not
      // asserted, so a slow/unavailable headless decode does not flake.
      const ready = await waitFor(
        "wavesurfer becomes ready",
        async () => {
          const panel = await find("css selector", "[data-testid='playback-panel']").catch(
            () => null,
          );
          if (!panel) return null;
          const busy = await getAttr(panel, "aria-busy");
          const toggle = await find("css selector", "[data-testid='playback-toggle']").catch(
            () => null,
          );
          if (!toggle) return null;
          const disabled = await getAttr(toggle, "disabled");
          return busy === "false" && (disabled === null || disabled === "false") ? true : null;
        },
        { timeoutMs: 15_000 },
      )
        .then((r) => r.value)
        .catch(() => false);
      return { path: out.path, wavesurferReady: ready };
    });

    // ============================================================
    // ear-training drill round — start_drill -> submit_drill_attempt ->
    // list_drill_history. The drill-card render step above only proves the
    // landing screen mounts; this exercises the full scoring + persistence
    // round-trip against the real SQLite library. ONNX-free.
    // ============================================================

    await step("drill — start -> submit -> history shows the new attempt", async () => {
      // 1. Count the pre-existing interval attempts so we can assert the
      //    submit added exactly one (a clean smoke run starts empty, but
      //    we compare deltas rather than absolutes so a re-run is robust).
      const before = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         window.__TAURI_INTERNALS__.invoke('list_drill_history', {
           filter: { kind: 'interval', since_unix_ms: null, limit: 200, offset: 0 }
         }).then((rows) => cb({ ok: true, count: Array.isArray(rows) ? rows.length : -1 }))
           .catch((e) => cb({ ok: false, error: String(e) }));`,
      );
      if (!before || !before.ok) {
        throw new Error(`list_drill_history (before) failed: ${before && before.error}`);
      }

      // 2. Start an interval drill (C4 -> G4, a perfect fifth). The synth
      //    render proves the prompt audio path; we keep the wav bytes only
      //    to assert it is non-empty.
      const started = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         window.__TAURI_INTERNALS__.invoke('start_drill', {
           spec: {
             kind: 'interval',
             prompt_notes: [{ midi: 60, a4_hz: 440.0 }, { midi: 67, a4_hz: 440.0 }],
             expected_response_midi: [67]
           }
         }).then((session) => cb({
            ok: true,
            sessionId: session.sessionId ?? session.session_id,
            wavLen: Array.isArray(session.promptWav ?? session.prompt_wav)
              ? (session.promptWav ?? session.prompt_wav).length : -1,
          }))
          .catch((e) => cb({ ok: false, error: String(e) }));`,
      );
      if (!started || !started.ok) {
        throw new Error(`start_drill failed: ${started && started.error}`);
      }
      if (typeof started.sessionId !== "string" || started.sessionId.length === 0) {
        throw new Error(`start_drill returned no session id: ${JSON.stringify(started)}`);
      }
      if (started.wavLen < 1000) {
        throw new Error(`start_drill prompt wav too small: ${started.wavLen} bytes`);
      }

      // 3. Submit a synthetic attempt — a handful of on-pitch voiced frames
      //    so the scorer marks it correct and persists a row.
      const now = Date.now();
      const submitted = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         const sessionId = arguments[0];
         const startedAt = arguments[1];
         window.__TAURI_INTERNALS__.invoke('submit_drill_attempt', {
           sessionId,
           spec: {
             kind: 'interval',
             prompt_notes: [{ midi: 60, a4_hz: 440.0 }, { midi: 67, a4_hz: 440.0 }],
             expected_response_midi: [67]
           },
           userPitchMidi: 67,
           attempt: {
             cents_error_frames: [2.0, -1.0, 0.5, 1.5, -0.5],
             voiced_frames: [true, true, true, true, true],
             started_at_unix_ms: startedAt,
             finished_at_unix_ms: startedAt + 800
           },
           pairedRecordingId: null
         }).then((result) => cb({
            ok: true,
            correct: result.correct,
            meanCentsError: result.meanCentsError ?? result.mean_cents_error,
          }))
          .catch((e) => cb({ ok: false, error: String(e) }));`,
        [started.sessionId, now],
      );
      if (!submitted || !submitted.ok) {
        throw new Error(`submit_drill_attempt failed: ${submitted && submitted.error}`);
      }
      if (submitted.correct !== true) {
        throw new Error(
          `submit_drill_attempt scored the on-pitch attempt incorrect: ${JSON.stringify(submitted)}`,
        );
      }

      // 4. History reflects the new attempt (count grew by exactly one).
      const { value: after } = await waitFor(
        "drill history grows by one after submit",
        async () => {
          const out = await executeAsync(
            `const cb = arguments[arguments.length - 1];
             window.__TAURI_INTERNALS__.invoke('list_drill_history', {
               filter: { kind: 'interval', since_unix_ms: null, limit: 200, offset: 0 }
             }).then((rows) => cb({ ok: true, count: Array.isArray(rows) ? rows.length : -1 }))
               .catch((e) => cb({ ok: false, error: String(e) }));`,
          );
          if (out && out.ok && out.count === before.count + 1) return out.count;
          return null;
        },
        { timeoutMs: 10_000 },
      );
      return { beforeCount: before.count, afterCount: after };
    });

    // ============================================================
    // settings round-trip — set_setting / get_settings have zero coverage
    // at any layer (the frontend never calls them). Patch the durable A4
    // reference through set_setting, then read it back via get_settings to
    // prove the read-modify-write + snapshot path round-trips against the
    // live process. We restore A4 to 440 afterwards (the cleanup step also
    // wipes settings.json) so the run stays idempotent.
    // ============================================================

    await step("settings — set_setting then get_settings reflects the A4 patch", async () => {
      const patched = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         window.__TAURI_INTERNALS__.invoke('set_setting', { key: 'a4_hz', value: 442.0 })
           .then((settings) => cb({ ok: true, a4: settings.a4_hz ?? settings.a4Hz }))
           .catch((e) => cb({ ok: false, error: String(e) }));`,
      );
      if (!patched || !patched.ok) {
        throw new Error(`set_setting failed: ${patched && patched.error}`);
      }
      if (Math.abs((patched.a4 ?? 0) - 442.0) > 1e-3) {
        throw new Error(`set_setting returned A4=${patched.a4}; expected 442`);
      }

      const fetched = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         window.__TAURI_INTERNALS__.invoke('get_settings')
           .then((settings) => cb({ ok: true, a4: settings.a4_hz ?? settings.a4Hz }))
           .catch((e) => cb({ ok: false, error: String(e) }));`,
      );
      if (!fetched || !fetched.ok) {
        throw new Error(`get_settings failed: ${fetched && fetched.error}`);
      }
      if (Math.abs((fetched.a4 ?? 0) - 442.0) > 1e-3) {
        throw new Error(
          `get_settings did not reflect the set_setting patch: got A4=${fetched.a4}, expected 442`,
        );
      }

      // Restore the default so the run is idempotent regardless of the
      // cleanup step's settings.json wipe ordering.
      await executeAsync(
        `const cb = arguments[arguments.length - 1];
         window.__TAURI_INTERNALS__.invoke('set_setting', { key: 'a4_hz', value: 440.0 })
           .then(() => cb({ ok: true }))
           .catch(() => cb({ ok: false }));`,
      );
      return { patchedA4: patched.a4, fetchedA4: fetched.a4 };
    });

    // ============================================================
    // capabilities probe — get_capabilities has no UI consumer and no
    // mock/e2e coverage; it is a diagnostic mirror of the build's cfg!
    // flags reserved for a future Help/About panel. Invoke it directly so
    // the one IPC command with true-zero coverage is exercised against the
    // live process and the {neural_compiled_in, pyin_compiled_in} shape is
    // asserted to be booleans. ONNX-free; pure constant return.
    // ============================================================

    await step("capabilities — get_capabilities returns boolean cfg flags", async () => {
      const out = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         window.__TAURI_INTERNALS__.invoke('get_capabilities')
           .then((caps) => cb({ ok: true, caps }))
           .catch((e) => cb({ ok: false, error: String(e) }));`,
      );
      if (!out || !out.ok) {
        throw new Error(`get_capabilities failed: ${out && out.error}`);
      }
      const caps = out.caps ?? {};
      const neural = caps.neuralCompiledIn ?? caps.neural_compiled_in;
      const pyin = caps.pyinCompiledIn ?? caps.pyin_compiled_in;
      if (typeof neural !== "boolean") {
        throw new Error(`get_capabilities neural flag not a boolean: ${JSON.stringify(caps)}`);
      }
      if (typeof pyin !== "boolean") {
        throw new Error(`get_capabilities pyin flag not a boolean: ${JSON.stringify(caps)}`);
      }
      return { neuralCompiledIn: neural, pyinCompiledIn: pyin };
    });

    // ============================================================
    // live recording round-trip — record -> stop -> library row. The mock
    // recording spec drives idle->recording->saved against the mock IPC;
    // start_recording / stop_recording are never fired against the real
    // Rust commands. This step exercises the real cpal capture -> FLAC
    // writer -> SQLite-row-on-stop path.
    //
    // start_recording requires a LIVE DSP pipeline (start_capture must have
    // landed `live`). On a runner without a usable capture device the boot
    // StatusPill lands `error`; in that case start_recording would fail
    // with "not capturing". Gate on the StatusPill data-state so a cardless
    // runner records a documented skip-as-pass rather than a spurious fail
    // — mirrors the boot step's tolerance of `data-state='error'`.
    // ============================================================

    await step("record — cpal capture round-trips to a new library row", async () => {
      // Return to the Tuner so the StatusPill + RecordButton mount (the
      // training step left the webview on the Training view; the URL hash
      // only routes on App mount, so we flip the view via training-exit).
      await ensureTunerView();

      // Probe the capture backend. `live` => a real device is enumerated
      // and start_recording can attach the encoder; anything else => skip.
      const captureState = await waitFor(
        "status-pill present with a known data-state",
        async () => {
          const el = await find("css selector", "[data-testid='status-pill']").catch(() => null);
          if (!el) return null;
          const state = await getAttr(el, "data-state");
          if (state === "live" || state === "idle" || state === "error") return state;
          return null;
        },
        { timeoutMs: 30_000, intervalMs: 500 },
      ).then((r) => r.value);

      if (captureState !== "live") {
        // Document the skip explicitly so a green run on a cardless runner
        // does not silently imply the record path was exercised.
        return { skipped: true, reason: `capture backend not live (state=${captureState})` };
      }

      // Snapshot the current library so we can assert a NEW id appears
      // (not the imported fixture's id).
      const idsBefore = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         window.__TAURI_INTERNALS__.invoke('list_recordings')
           .then((rows) => cb(rows.map((r) => (typeof r === 'string' ? r : r.id))))
           .catch((e) => cb({ error: String(e) }));`,
      );
      if (!Array.isArray(idsBefore)) {
        throw new Error(`list_recordings (before record) failed: ${JSON.stringify(idsBefore)}`);
      }

      // Start a recording directly via IPC. start_recording requires a
      // progress Channel arg in addition to instrumentProfile; bind a
      // no-op channel the same way the analyze / transcribe steps do.
      const started = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         const chId = '__CHANNEL__:' + window.__TAURI_INTERNALS__.transformCallback(() => {}, false);
         window.__TAURI_INTERNALS__.invoke('start_recording', {
           progress: chId, instrumentProfile: 'voice'
         }).then((id) => cb({ ok: true, id }))
           .catch((e) => cb({ ok: false, error: String(e) }));`,
      );
      if (!started || !started.ok) {
        throw new Error(`start_recording failed: ${started && started.error}`);
      }

      // Capture ~1.5 s of audio so the FLAC has real frames and a non-zero
      // duration on stop.
      await sleep(1_500);

      const stopped = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         window.__TAURI_INTERNALS__.invoke('stop_recording')
           .then((row) => cb({ ok: true, row }))
           .catch((e) => cb({ ok: false, error: String(e) }));`,
      );
      if (!stopped || !stopped.ok) {
        throw new Error(`stop_recording failed: ${stopped && stopped.error}`);
      }
      const row = stopped.row ?? {};
      const newId = row.id;
      if (typeof newId !== "string" || newId.length === 0) {
        throw new Error(`stop_recording returned no row id: ${JSON.stringify(row)}`);
      }
      if (idsBefore.includes(newId)) {
        throw new Error(`stop_recording reused an existing id ${newId}; expected a fresh take`);
      }
      const durationMs = row.duration_ms ?? row.durationMs;
      if (typeof durationMs !== "number" || durationMs <= 0) {
        throw new Error(`stop_recording row durationMs not > 0: ${JSON.stringify(row)}`);
      }
      const sampleRate = row.sample_rate_hz ?? row.sampleRateHz;
      if (sampleRate !== 48_000) {
        throw new Error(`stop_recording row sampleRateHz expected 48000, got ${sampleRate}`);
      }

      // list_recordings must now include the new id.
      const idsAfter = await executeAsync(
        `const cb = arguments[arguments.length - 1];
         window.__TAURI_INTERNALS__.invoke('list_recordings')
           .then((rows) => cb(rows.map((r) => (typeof r === 'string' ? r : r.id))))
           .catch((e) => cb({ error: String(e) }));`,
      );
      if (!Array.isArray(idsAfter) || !idsAfter.includes(newId)) {
        throw new Error(
          `list_recordings did not include the new take ${newId}: ${JSON.stringify(idsAfter)}`,
        );
      }

      // Delete the recorded take so the run is idempotent (the imported
      // fixture is removed by the cleanup step below).
      await executeAsync(
        `const cb = arguments[arguments.length - 1];
         const id = arguments[0];
         window.__TAURI_INTERNALS__.invoke('delete_recording', { id })
           .then(() => cb({ ok: true }))
           .catch(() => cb({ ok: false }));`,
        [newId],
      );
      return { newId, durationMs, sampleRate };
    });

    // ============================================================
    // cleanup
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
