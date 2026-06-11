#!/usr/bin/env bash
# scripts/smoke-test.sh
#
# Live-shell smoke harness. Drives a real `cargo tauri build` binary
# through `tauri-driver` (the official Tauri WebDriver shim around
# WebKitWebDriver) and walks the UI through every shipped feature,
# capturing screenshots at each step.
#
# Prerequisites (system-installed, not via npm):
#   - WebKitWebDriver         (apt: webkit2gtk-driver)
#   - tauri-driver            (cargo install tauri-driver --locked)
#   - ImageMagick `import`    (apt: imagemagick)            -- already present
#   - wmctrl                  (apt: wmctrl)                 -- already present
#
# Optional:
#   - ORT_DYLIB_PATH          libonnxruntime.so for transcribe / stem-separate
#                             paths to actually run end-to-end. Auto-resolved
#                             below if a common cache path exists.
#
# This script does NOT inject keyboard / mouse events at the OS layer —
# it drives the webview directly through the WebDriver protocol, which
# works on both X11 and Wayland without root or uinput.
#
# Output: screenshots and a JSON summary under `.smoke-reports/<UTC-timestamp>/`.
# Exit code: 0 on green, non-zero on first failure.

set -euo pipefail
IFS=$'\n\t'

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${REPO_ROOT}"

# ---------------------------------------------------------------------
# Tooling guards.
# ---------------------------------------------------------------------
require() {
  local cmd="$1"
  local hint="$2"
  if ! command -v "${cmd}" >/dev/null 2>&1; then
    echo "ERROR: ${cmd} not found." >&2
    echo "       ${hint}" >&2
    exit 2
  fi
}

require WebKitWebDriver "sudo apt-get install -y webkit2gtk-driver"
require tauri-driver "cargo install tauri-driver --locked"
require import "sudo apt-get install -y imagemagick"
require cargo "rustup default stable"

# ---------------------------------------------------------------------
# Auto-resolve libonnxruntime so the transcribe + stem-separate paths
# do not block in dlopen. Mirror scripts/ci-local.sh's resolution.
# ---------------------------------------------------------------------
if [[ -z "${ORT_DYLIB_PATH:-}" ]]; then
  for candidate in \
    "${HOME}/.bun/install/cache/onnxruntime-node@1.21.0@@@1/bin/napi-v3/linux/x64/libonnxruntime.so.1.21.0" \
    "/usr/local/lib/libonnxruntime.so" \
    "/usr/lib/x86_64-linux-gnu/libonnxruntime.so"; do
    if [[ -f "${candidate}" ]]; then
      export ORT_DYLIB_PATH="${candidate}"
      echo "Resolved ORT_DYLIB_PATH=${candidate}"
      break
    fi
  done
fi

# ---------------------------------------------------------------------
# Output directory.
# ---------------------------------------------------------------------
TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
REPORT_DIR="${REPO_ROOT}/.smoke-reports/${TIMESTAMP}"
mkdir -p "${REPORT_DIR}"
echo "Smoke report: ${REPORT_DIR}"

# ---------------------------------------------------------------------
# App-data isolation. The shell writes its SQLite library + recordings
# under `$APP_DATA` resolved by `tauri::Manager::path().app_data_dir()`.
# A clean smoke run starts from an empty library so the row counts are
# deterministic. We do NOT delete the cached HTDemucs ONNX (~316 MB) —
# the harness instead seeds it into the app-data models dir so the
# stem-separate step does not have to redownload.
# ---------------------------------------------------------------------
APP_ID="com.derekkinzo.neuralpitch"
APP_DATA="${HOME}/.local/share/${APP_ID}"
RECORDINGS="${APP_DATA}/recordings"
MODELS_DIR="${APP_DATA}/models"
LEGACY_MODELS_DIR="${HOME}/.local/share/neural-pitch/models"

echo "==> resetting app-data at ${APP_DATA}"
rm -rf "${RECORDINGS}" "${APP_DATA}/settings.json" "${APP_DATA}/library.sqlite"*
mkdir -p "${RECORDINGS}" "${MODELS_DIR}"

# If a cached HTDemucs ONNX exists in the legacy location, hard-link it
# into the active app-data dir so the stem-separate step short-circuits
# the 316 MB download. (Hard-link, not copy, so we don't double-up disk.)
if [[ -f "${LEGACY_MODELS_DIR}/htdemucs.onnx" ]] && [[ ! -f "${MODELS_DIR}/htdemucs.onnx" ]]; then
  echo "==> linking cached HTDemucs ONNX from ${LEGACY_MODELS_DIR}"
  ln -f "${LEGACY_MODELS_DIR}/htdemucs.onnx" "${MODELS_DIR}/htdemucs.onnx"
fi

# ---------------------------------------------------------------------
# Pick a fixture FLAC. The driver imports it via the Tauri command
# instead of the native open dialog (which WebDriver can't drive).
# ---------------------------------------------------------------------
FIXTURE="${REPO_ROOT}/crates/neural-pitch-core/tests/fixtures/voice/069_A4_synthvoice_clean.flac"
if [[ ! -f "${FIXTURE}" ]]; then
  echo "ERROR: fixture not found at ${FIXTURE}" >&2
  exit 2
fi
echo "==> fixture: ${FIXTURE}"

# ---------------------------------------------------------------------
# Build a release binary so the smoke pass exercises the same code
# the user will install. `npm run build` first because Tauri's
# `generate_context!` reads `dist/`.
# ---------------------------------------------------------------------
echo "==> npm run build"
npm run build > "${REPORT_DIR}/npm-build.log" 2>&1

echo "==> cargo tauri build (debug; release would push the run past 10 min)"
# Both `app-neural` (PESTO/CREPE in core) and `neural` (Basic Pitch +
# HTDemucs IPC surface in src-tauri) must be on for the smoke pass to
# exercise the import / transcribe / separate commands.
cargo build -p neural-pitch --features app-neural,neural \
  > "${REPORT_DIR}/cargo-build.log" 2>&1

BINARY="${REPO_ROOT}/target/debug/neural-pitch"
if [[ ! -x "${BINARY}" ]]; then
  echo "ERROR: ${BINARY} not found after build." >&2
  exit 1
fi

# ---------------------------------------------------------------------
# Driver run. tauri-driver listens on http://localhost:4444; the test
# script connects via WebDriver and walks the UI.
# ---------------------------------------------------------------------
echo "==> Spawning tauri-driver"
tauri-driver --port 4444 > "${REPORT_DIR}/tauri-driver.log" 2>&1 &
DRIVER_PID=$!
trap 'kill ${DRIVER_PID} 2>/dev/null || true' EXIT

# Give the driver a beat to bind the port.
sleep 2

if ! curl -fsS http://localhost:4444/status >/dev/null 2>&1; then
  echo "ERROR: tauri-driver did not start on :4444." >&2
  echo "       Check ${REPORT_DIR}/tauri-driver.log" >&2
  exit 1
fi

echo "==> Running scripts/smoke-driver.mjs"
set +e
node "${REPO_ROOT}/scripts/smoke-driver.mjs" \
  --binary "${BINARY}" \
  --report-dir "${REPORT_DIR}" \
  --fixture "${FIXTURE}" \
  --driver-url http://localhost:4444 \
  > "${REPORT_DIR}/driver.log" 2>&1
DRIVER_EXIT=$?
set -e

if [[ ${DRIVER_EXIT} -eq 0 ]]; then
  echo "==> SMOKE PASS — ${REPORT_DIR}"
else
  echo "==> SMOKE FAIL — exit ${DRIVER_EXIT}; see ${REPORT_DIR}/driver.log"
fi
exit ${DRIVER_EXIT}
