#!/usr/bin/env bash
# scripts/smoke-test.sh
#
# Live-shell smoke harness. Drives a real Tauri shell binary
# (`cargo build -p neural-pitch --features app-neural,neural`)
# through `tauri-driver` (the official Tauri WebDriver shim around
# WebKitWebDriver) and walks the UI through every shipped feature,
# capturing screenshots at each step.
#
# Prerequisites (system-installed, not via npm):
#   - WebKitWebDriver         (apt: webkit2gtk-driver)
#   - tauri-driver            (cargo install tauri-driver --locked)
#
# Optional:
#   - ORT_DYLIB_PATH          libonnxruntime.so for transcribe / stem-separate
#                             paths to actually run end-to-end. Auto-resolved
#                             below from a portable npm/system layout if not
#                             pre-set. Recommended local install:
#                               npm install --no-save --prefix .ort \
#                                 onnxruntime-node@1.21.0
#                             then export ORT_DYLIB_PATH to the resulting
#                             .ort/node_modules/onnxruntime-node/bin/...
#                             libonnxruntime.so file. CI uses the same path.
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
require cargo "rustup default stable"

# ---------------------------------------------------------------------
# Auto-resolve libonnxruntime so the transcribe + stem-separate paths
# do not block in dlopen. Searches portable install layouts only:
#   - ./.ort/node_modules/onnxruntime-node/...     (npm prefix the
#     CI workflow and the recommended local install both write to)
#   - npm global / npx node_modules                (./node_modules,
#     ${HOME}/.npm/_npx/* — works for `npm exec onnxruntime-node`)
#   - system-installed shared libs at standard prefixes
# Mirror scripts/ci-local.sh's resolution.
# ---------------------------------------------------------------------
if [[ -z "${ORT_DYLIB_PATH:-}" ]]; then
  # Resolve the host arch sub-dir under `onnxruntime-node`'s napi layout
  # (e.g. `linux/x64/` or `linux/arm64/`). Loading an arch-mismatched
  # dylib silently falls back to a degraded code path and turns a 5 s
  # ONNX call into a 4 min one.
  case "$(uname -s)/$(uname -m)" in
    Linux/x86_64)  HOST_NAPI="linux/x64" ;;
    Linux/aarch64) HOST_NAPI="linux/arm64" ;;
    Darwin/x86_64) HOST_NAPI="darwin/x64" ;;
    Darwin/arm64)  HOST_NAPI="darwin/arm64" ;;
    *)             HOST_NAPI="" ;;
  esac

  declare -a candidates=()
  if [[ -n "${HOST_NAPI}" ]]; then
    while IFS= read -r found; do
      candidates+=("${found}")
    done < <(
      # `-maxdepth 9` reaches `.ort/node_modules/onnxruntime-node/bin/napi-v3/linux/x64/libonnxruntime.so.1.21.0`
      # (7 components past the root). The earlier `-maxdepth 6` truncated
      # the traversal before the file, leaving ORT_DYLIB_PATH unset and
      # the smoke harness stuck loading whatever the system loader served.
      find "${REPO_ROOT}/.ort" "${REPO_ROOT}/node_modules" "${HOME}/.npm/_npx" \
        -maxdepth 9 -path "*/${HOST_NAPI}/libonnxruntime.so*" -print 2>/dev/null || true
    )
  fi
  candidates+=(
    "/usr/local/lib/libonnxruntime.so"
    "/usr/lib/x86_64-linux-gnu/libonnxruntime.so"
  )
  for candidate in "${candidates[@]}"; do
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

echo "==> resetting app-data at ${APP_DATA}"
rm -rf "${RECORDINGS}" "${APP_DATA}/settings.json" "${APP_DATA}/library.sqlite"*
mkdir -p "${RECORDINGS}" "${MODELS_DIR}"

# Local-developer-only convenience: if an older app-id's models cache
# still lives at ~/.local/share/neural-pitch/models, hard-link the
# pinned HTDemucs ONNX into the active app-data dir so the
# stem-separate step short-circuits the 316 MB download. CI starts
# from an empty $HOME and uses the actions/cache step instead, so this
# is a no-op there.
if [[ -z "${CI:-}" ]]; then
  LEGACY_MODELS_DIR="${HOME}/.local/share/neural-pitch/models"
  if [[ -f "${LEGACY_MODELS_DIR}/htdemucs.onnx" ]] && [[ ! -f "${MODELS_DIR}/htdemucs.onnx" ]]; then
    echo "==> linking cached HTDemucs ONNX from ${LEGACY_MODELS_DIR}"
    ln -f "${LEGACY_MODELS_DIR}/htdemucs.onnx" "${MODELS_DIR}/htdemucs.onnx"
  fi
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

echo "==> cargo build (debug; release lengthens the run beyond the smoke-step CI budget)"
# Both `app-neural` (PESTO/CREPE in core) and `neural` (Basic Pitch +
# HTDemucs IPC surface in src-tauri) must be on for the smoke pass to
# exercise the import / transcribe / separate commands.
#
# Retry once on failure: rust-lld occasionally dies with SIGBUS while
# linking the ~250-rlib graph on a memory-pressured CI runner. The
# relink runs against a warm cache once the first attempt's memory is
# reclaimed; a second failure is a real error and aborts the smoke run.
if ! cargo build -p neural-pitch --features app-neural,neural \
  > "${REPORT_DIR}/cargo-build.log" 2>&1; then
  echo "    cargo build failed once (likely a transient SIGBUS link); retrying" >&2
  cargo build -p neural-pitch --features app-neural,neural \
    >> "${REPORT_DIR}/cargo-build.log" 2>&1
fi

BINARY="${REPO_ROOT}/target/debug/neural-pitch"
if [[ ! -x "${BINARY}" ]]; then
  echo "ERROR: ${BINARY} not found after build." >&2
  exit 1
fi

# ---------------------------------------------------------------------
# Driver run. tauri-driver listens on a free TCP port chosen at
# startup; the test script connects via WebDriver and walks the UI.
# Picking the port dynamically avoids collisions with any other
# WebDriver / Selenium daemon that may already hold :4444 on a
# developer laptop.
# ---------------------------------------------------------------------
PICK_PORT_PY='import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
DRIVER_PORT="$(python3 -c "${PICK_PORT_PY}" 2>/dev/null || true)"
if [[ -z "${DRIVER_PORT}" ]]; then
  # Fallback for shells without python3 — node ships in CI and locally.
  DRIVER_PORT="$(node -e 'const s=require("net").createServer().listen(0,()=>{process.stdout.write(String(s.address().port));s.close();})' 2>/dev/null || true)"
fi
if [[ -z "${DRIVER_PORT}" ]]; then
  echo "ERROR: could not pick a free port for tauri-driver." >&2
  exit 1
fi
DRIVER_URL="http://localhost:${DRIVER_PORT}"
echo "==> Spawning tauri-driver on ${DRIVER_URL}"
tauri-driver --port "${DRIVER_PORT}" > "${REPORT_DIR}/tauri-driver.log" 2>&1 &
DRIVER_PID=$!
# Kill the driver AND any leaked app binary. A previous failed run can
# leave `target/debug/neural-pitch` resident, holding the audio device
# and competing for memory with the next session — which manifests as
# unrelated WebDriver hangs (e.g. /screenshot timing out under swap
# pressure). pkill -f matches the binary path; we silently ignore the
# absent-process exit code.
trap 'kill ${DRIVER_PID} 2>/dev/null || true; pkill -f "target/debug/neural-pitch" 2>/dev/null || true' EXIT

# Poll /status until the daemon binds the port; surface the actual
# tauri-driver stderr if the deadline expires.
DEADLINE=$((SECONDS + 10))
DRIVER_READY=0
while (( SECONDS < DEADLINE )); do
  if curl -fsS "${DRIVER_URL}/status" >/dev/null 2>&1; then
    DRIVER_READY=1
    break
  fi
  if ! kill -0 "${DRIVER_PID}" 2>/dev/null; then
    echo "ERROR: tauri-driver exited before binding ${DRIVER_URL}." >&2
    echo "       Check ${REPORT_DIR}/tauri-driver.log" >&2
    exit 1
  fi
  sleep 0.25
done
if [[ ${DRIVER_READY} -ne 1 ]]; then
  echo "ERROR: tauri-driver did not become ready at ${DRIVER_URL} within 10s." >&2
  echo "       Check ${REPORT_DIR}/tauri-driver.log" >&2
  exit 1
fi

echo "==> Running scripts/smoke-driver.mjs"
set +e
node "${REPO_ROOT}/scripts/smoke-driver.mjs" \
  --binary "${BINARY}" \
  --report-dir "${REPORT_DIR}" \
  --fixture "${FIXTURE}" \
  --driver-url "${DRIVER_URL}" \
  > "${REPORT_DIR}/driver.log" 2>&1
DRIVER_EXIT=$?
set -e

if [[ ${DRIVER_EXIT} -eq 0 ]]; then
  echo "==> SMOKE PASS — ${REPORT_DIR}"
else
  echo "==> SMOKE FAIL — exit ${DRIVER_EXIT}; see ${REPORT_DIR}/driver.log"
fi
exit ${DRIVER_EXIT}
