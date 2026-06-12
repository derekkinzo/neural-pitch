#!/usr/bin/env bash
# scripts/ci-local.sh
#
# Tiered local-CI gate. Each tier is a curated subset of the jobs
# defined in .github/workflows/ci.yml — chosen so a green local run is
# a near-certain green remote run. The tiers are layered:
#
#   quick   (default) host-only gate, target ~3min warm cache.
#           Pre-push hook contract — if quick passes, the push must
#           not break CI.
#   visual  quick + Playwright verify (Chromium + WebKit) inside the
#           pinned Microsoft Playwright Docker image (deterministic
#           font rendering). Target ~90s warm cache.
#   full    visual + `act` replay of every Linux-runnable job in
#           ci.yml. macOS / Windows matrix legs cannot run under act
#           and are listed as skipped. Target ~10min warm cache.
#
# Hard rule: zero warnings, zero errors. Both --all-features AND
# --no-default-features must build clean and pass tests. No internal
# references, no personal-machine paths.
#
# CI environment parity. The CI workflow declares these env vars at
# workflow scope (.github/workflows/ci.yml `env:` block) — every cargo
# step there inherits them. The harness exports the same values so a
# warning that fails CI also fails the harness:
#
#   CARGO_TERM_COLOR=always
#   RUSTFLAGS=-D warnings
#   RUST_BACKTRACE=1
#
# CI-job coverage map (the harness's "mirrors CI" claim, made precise):
#
#   quick:
#     fmt                          (cargo fmt --all -- --check)
#     clippy                       (cargo clippy --all-features)
#     clippy-no-default-features   (build + clippy, core crate)
#     test                         (cargo test --all-features, host
#                                   default toolchain only — beta
#                                   exercised when `rustup toolchain
#                                   list` shows it)
#     deny                         (cargo deny check)
#     typecheck                    (npx tsc, npx tsc -p tests/e2e)
#     lint                         (npm run lint)
#     build                        (cargo build --release --workspace,
#                                   gated by `npm run build` since
#                                   Tauri's generate_context! reads
#                                   dist/)
#     no-leak grep                 (project hard-rule; also a CI job —
#                                   ci.yml `no-leak` is the
#                                   authoritative copy)
#     voice acceptance             (§13.2 floor; also a CI job —
#                                   ci.yml `voice-acceptance` is the
#                                   authoritative copy)
#
#   visual: quick + e2e-mock (Chromium + WebKit in Docker)
#   full:   visual + `act` replay of all Linux jobs in ci.yml
#
# Skipped from local replay (remote CI is authoritative):
#   - test (macos-latest / stable, beta)
#   - test (windows-latest / stable, beta)
#   - commit-lint (pull_request only — runs on the PR base..head range)
#
# Usage:
#   scripts/ci-local.sh            # quick (default)
#   scripts/ci-local.sh quick
#   scripts/ci-local.sh visual
#   scripts/ci-local.sh full
#   scripts/ci-local.sh --help
#
# Exit codes:
#   0  all steps passed
#   1  a gate step failed
#   2  unknown tier, or a required tool (e.g. act) is missing
set -euo pipefail
IFS=$'\n\t'

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export REPO_ROOT
cd "${REPO_ROOT}"

# ---------------------------------------------------------------------
# CI environment parity — mirror the workflow `env:` block so any
# rustc-level warning (not just clippy lints) that would fail CI also
# fails the harness. Without this, e.g. `unused_imports` or
# `dead_code` in newly-edited code passes the harness and breaks CI.
# ---------------------------------------------------------------------
export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"
export RUSTFLAGS="${RUSTFLAGS:--D warnings}"
export RUST_BACKTRACE="${RUST_BACKTRACE:-1}"

# `ort = { features = ["load-dynamic"] }` looks up `libonnxruntime.so` at
# runtime. If unset and the system loader cannot find one, ONNX session
# constructors block in `dlopen` instead of erroring fast. Auto-detect a
# common cached copy so the local gate's `--include-ignored` ONNX tests
# do not silently hang. Override by exporting `ORT_DYLIB_PATH` yourself.
if [[ -z "${ORT_DYLIB_PATH:-}" ]]; then
  # Resolve the host arch sub-dir under `onnxruntime-node`'s napi layout
  # so we never hand ort an arch-mismatched dylib. Filter the candidate
  # list by `*/${HOST_NAPI}/...` instead of relying on directory order.
  case "$(uname -s)/$(uname -m)" in
    Linux/x86_64)  HOST_NAPI="linux/x64" ;;
    Linux/aarch64) HOST_NAPI="linux/arm64" ;;
    Darwin/x86_64) HOST_NAPI="darwin/x64" ;;
    Darwin/arm64)  HOST_NAPI="darwin/arm64" ;;
    *)             HOST_NAPI="" ;;
  esac
  declare -a CANDIDATES=()
  if [[ -n "${HOST_NAPI}" ]]; then
    while IFS= read -r found; do
      CANDIDATES+=("${found}")
    done < <(
      find "${REPO_ROOT}/.ort" "${REPO_ROOT}/node_modules" "${HOME}/.npm/_npx" \
        -maxdepth 6 -path "*/${HOST_NAPI}/libonnxruntime.so*" -print 2>/dev/null || true
    )
  fi
  CANDIDATES+=(
    "/usr/local/lib/libonnxruntime.so"
    "/usr/lib/x86_64-linux-gnu/libonnxruntime.so"
  )
  for candidate in "${CANDIDATES[@]}"; do
    if [[ -f "${candidate}" ]]; then
      export ORT_DYLIB_PATH="${candidate}"
      break
    fi
  done
fi

# ---------------------------------------------------------------------
# tty-aware colors via tput. Gate on BOTH stdout and stderr being a
# tty so a single-stream redirect (e.g. `... 2> err.log`) strips ANSI
# escapes from BOTH streams — otherwise raw escape bytes leak into the
# redirected file. Honor NO_COLOR (https://no-color.org/) too.
# ---------------------------------------------------------------------
if [ -z "${NO_COLOR:-}" ] && [ -t 1 ] && [ -t 2 ] \
  && command -v tput >/dev/null 2>&1; then
  C_CYAN="$(tput setaf 6 2>/dev/null || true)"
  C_GREEN="$(tput setaf 2 2>/dev/null || true)"
  C_RED="$(tput setaf 1 2>/dev/null || true)"
  C_YELLOW="$(tput setaf 3 2>/dev/null || true)"
  C_BOLD="$(tput bold 2>/dev/null || true)"
  C_RESET="$(tput sgr0 2>/dev/null || true)"
else
  C_CYAN=""
  C_GREEN=""
  C_RED=""
  C_YELLOW=""
  C_BOLD=""
  C_RESET=""
fi

usage() {
  cat <<'EOF'
Usage: scripts/ci-local.sh [tier]

Tiers:
  quick    (default) host-only gate, target ~3min warm cache (cold
           release-cache rebuilds may push to a few minutes).
           Pre-push hook contract.
  visual   quick prelude + Playwright verify (Chromium + WebKit)
           inside the pinned Microsoft Playwright Docker image.
           Target ~90s warm cache.
  full     visual + `act` replay of every Linux-runnable CI job.
           macOS / Windows matrix legs cannot run under act and are
           reported as skipped. Target ~10min warm cache.

Options:
  -h, --help   show this message and exit.

Environment:
  NO_COLOR     when set (any value), disable ANSI color output.

Exit codes:
  0  all steps passed
  1  a gate step failed
  2  unknown tier, or a required tool is missing
EOF
}

# ---------------------------------------------------------------------
# Step harness — captures stdout+stderr per step, prints only on
# failure (with last 20 lines + a "fix with: ..." hint), and records
# elapsed wall-clock for the summary banner. Quiet on green to keep
# the gate fast and scannable.
#
# Failure-log lifecycle: per-step logs live under .ci-local-logs/ at
# the repo root (gitignored). On any failed step the offending log is
# ALSO copied to .ci-local-last-failure.log so the user has one
# stable file to inspect after the trap fires. On a fully green run
# the per-step logs are removed by the EXIT trap; the last-failure
# pointer is preserved so it remains the canonical "what did I break
# yesterday" file.
# ---------------------------------------------------------------------
LOGS_DIR="${REPO_ROOT}/.ci-local-logs"
LAST_FAILURE_LOG="${REPO_ROOT}/.ci-local-last-failure.log"
mkdir -p "${LOGS_DIR}"

# Mark whether any run_step failed; consulted by the EXIT trap. We
# preserve logs on failure so the user can `cat .ci-local-logs/...`
# after the shell prompt returns.
RUN_FAILED=0

cleanup_logs_on_green() {
  if [ "${RUN_FAILED}" -eq 0 ]; then
    rm -rf "${LOGS_DIR}" 2>/dev/null || true
  fi
}
trap cleanup_logs_on_green EXIT

# Parallel indexed arrays of step names, shell commands, and per-step
# fix hints. Walked by integer index so we stay compatible with macOS
# bash 3.2 (no associative arrays).
STEP_NAMES=()
STEP_CMDS=()
STEP_HINTS=()
STEP_TIMES=()

run_step() {
  local idx="$1"
  local total="$2"
  local name="$3"
  local cmd="$4"
  local hint="${5:-}"
  local logfile="${LOGS_DIR}/step-${idx}.log"
  local start
  start="$(date +%s)"
  printf '%s==> step %d/%d: %s%s\n' "${C_CYAN}" "${idx}" "${total}" "${name}" "${C_RESET}"
  if bash -c "${cmd}" >"${logfile}" 2>&1; then
    local end elapsed
    end="$(date +%s)"
    elapsed=$((end - start))
    STEP_TIMES+=("${elapsed}")
    printf '    %sok (%ds)%s\n' "${C_GREEN}" "${elapsed}" "${C_RESET}"
    return 0
  else
    local rc=$?
    local end elapsed
    end="$(date +%s)"
    elapsed=$((end - start))
    RUN_FAILED=1
    cp "${logfile}" "${LAST_FAILURE_LOG}" 2>/dev/null || true
    printf '%s%sFAILED: %s (%ds)%s\n' "${C_BOLD}" "${C_RED}" "${name}" "${elapsed}" "${C_RESET}" >&2
    echo "--- last 20 lines of output ---" >&2
    tail -n 20 "${logfile}" >&2 || true
    echo "--- full log: ${LAST_FAILURE_LOG} ---" >&2
    if [ -n "${hint}" ]; then
      echo "To fix: ${hint}" >&2
    fi
    return "${rc}"
  fi
}

# ---------------------------------------------------------------------
# No-leak grep — fail if any tracked file contains an internal-only
# reference or a personal-machine path.
#
# Self-exclusion mechanism: the grep below has to *contain* the very
# patterns it forbids, so we tag each such line with the literal
# sentinel `# no-leak: regex-source` and then strip those tagged
# lines from the input that the grep sees. The sentinel comment must
# stay on the SAME line as the offending substring. New tagged lines
# are allowed only when they are part of the regex source — any other
# leak in this script will still fail the gate.
# ---------------------------------------------------------------------
no_leak_grep() {
  local hits
  # The pattern below mentions the forbidden substrings; tag this
  # exact line with the sentinel so the grep below skips ONLY it.
  local pattern='amazon|amzn|asbx|midway|brazil|claude|/home/ANT' # no-leak: regex-source
  # Singer-positioning preflight. The repo positions itself as a
  # general-purpose pitch-detection app for musicians. The forbidden
  # phrases appearing in copy or marketing surfaces would re-introduce
  # the singer-specific framing that was explicitly dropped.
  # no-leak: regex-source. 'sight-singing' is the ear-training drill ID — allowlisted under training/ sub-trees.
  local singer_pattern='for singers|sight-singing|app for singers' # no-leak: regex-source
  # Path allowlist for ear-training drill code. These trees
  # legitimately reference the drill ID; the singer_pattern grep is
  # precise about which paths it permits — we do NOT blanket-exclude
  # src/components/, only training/ subtrees.
  local training_allowlist='^crates/neural-pitch-core/src/training/|^src-tauri/src/commands_drill\.rs$|^src-tauri/tests/(drill_history_persists|match_channel_emits)\.rs$|^src/components/training/|^src/hooks/useDrillMatchStream\.ts$|^src/lib/drill-synth\.ts$|^src/stores/trainingStore\.ts$|^src/types/training\.ts$|^tests/e2e/[a-z0-9_]*(training|karaoke|sight_singing|interval_drill|chord_drill|scale_drill|solfege)[a-z0-9_]*\.spec\.ts$|^tests/e2e/helpers/tauri-mock\.ts$'
  hits="$(git ls-files \
    | xargs grep -InE "${pattern}" 2>/dev/null \
    | grep -vE '# no-leak: regex-source' \
    | cut -d: -f1 \
    | sort -u || true)"
  local singer_hits
  singer_hits="$(git ls-files \
    | xargs grep -InE "${singer_pattern}" 2>/dev/null \
    | grep -vE '# no-leak: regex-source' \
    | cut -d: -f1 \
    | grep -vE "${training_allowlist}" \
    | sort -u || true)"
  if [ -n "${hits}" ] || [ -n "${singer_hits}" ]; then
    echo "no-leak grep FAILED — offending files:" >&2
    [ -n "${hits}" ] && echo "${hits}" >&2
    [ -n "${singer_hits}" ] && echo "${singer_hits}" >&2
    return 1
  fi
}
export -f no_leak_grep

# ---------------------------------------------------------------------
# dist/ guard — Tauri's generate_context! macro reads dist/ at compile
# time; clippy and cargo build will fail without it. The harness
# requires `npm run build` has produced dist/ on the host before
# running cargo steps. We don't run npm build here ourselves because
# it doubles cold-cache wall-clock; instead we fail fast with a clear
# pointer.
# ---------------------------------------------------------------------
dist_guard() {
  if [ ! -f "${REPO_ROOT}/dist/index.html" ]; then
    echo "dist/index.html is missing." >&2
    echo "Tauri's generate_context! macro reads dist/ at compile time;" >&2
    echo "cargo clippy and cargo build will fail without it." >&2
    echo "Run: npm install && npm run build" >&2
    return 1
  fi
}
export -f dist_guard

# ---------------------------------------------------------------------
# beta toolchain — only exercised when the developer has it
# installed. Otherwise emit a soft-skip reminder so the gap is
# visible without blocking pushes that don't touch Rust.
# ---------------------------------------------------------------------
beta_test_step() {
  if rustup toolchain list 2>/dev/null | grep -q '^beta'; then
    cargo +beta test --workspace --all-features
  else
    cat <<'EOF' >&2
note: rust beta toolchain not installed — skipping `cargo +beta test`.
      CI runs the test matrix on stable AND beta. Install with:
          rustup toolchain install beta
      so beta-only regressions surface locally.
EOF
  fi
}
export -f beta_test_step

# ---------------------------------------------------------------------
# Quick tier — host-only gate ordered cheapest → most expensive so a
# 1-line prettier violation surfaces in <2s, not after the 30s test
# step. Steps are documented above in "CI-job coverage map".
# ---------------------------------------------------------------------
quick_tier() {
  STEP_NAMES=(
    "no-leak grep"
    "prettier"
    "eslint"
    "cargo fmt"
    "tsc app"
    "tsc e2e"
    "dist guard"
    "clippy (all-features)"
    "cargo build (no-default)"
    "clippy (no-default)"
    "cargo deny"
    "cargo test (all)"
    "cargo test (no-default)"
    "cargo +beta test"
    "voice acceptance"
    "cargo build --release"
    "summary"
  )
  STEP_CMDS=(
    "no_leak_grep"
    "npx prettier --check ."
    "npm run lint"
    "cargo fmt --all -- --check"
    "npx tsc --noEmit"
    "npx tsc -p tests/e2e/tsconfig.json"
    "dist_guard"
    "cargo clippy --workspace --all-targets --all-features -- -D warnings"
    "cargo build -p neural-pitch-core --no-default-features"
    "cargo clippy -p neural-pitch-core --all-targets --no-default-features -- -D warnings"
    "cargo deny check"
    "cargo test --workspace --all-features"
    "cargo test -p neural-pitch-core --no-default-features"
    "beta_test_step"
    "bash scripts/run-acceptance.sh"
    "cargo build --release --workspace"
    ":"
  )
  STEP_HINTS=(
    "edit the listed file(s) to remove the term, or tag the offending line with '# no-leak: regex-source' if it is a documented regex/pattern definition"
    "npx prettier --write ."
    "npm run lint -- --fix"
    "cargo fmt --all"
    "open the failing file path printed above and fix the type error"
    "open the failing file path printed above and fix the type error"
    "npm install && npm run build"
    "cargo clippy --workspace --all-targets --all-features --fix --allow-dirty -- -D warnings"
    "investigate the build error; check feature gates on conditional imports (#[cfg(feature = ...)] )"
    "cargo clippy --workspace --all-targets --no-default-features --fix --allow-dirty -- -D warnings"
    "review .deny.toml and update advisories/licenses/bans, or 'cargo update' the offending crate"
    "cargo test --workspace --all-features -- --nocapture"
    "cargo test --workspace --no-default-features -- --nocapture"
    "rustup toolchain install beta && cargo +beta test --workspace --all-features"
    "rerun: bash scripts/run-acceptance.sh — fixtures dipping below 0.95 floor indicate a regression in the pitch estimator"
    "cargo build --release --workspace -v"
    ""
  )

  local total="${#STEP_NAMES[@]}"
  local quick_start
  quick_start="$(date +%s)"

  local i
  for i in $(seq 0 $((total - 1))); do
    local n cmd hint idx
    n="${STEP_NAMES[$i]}"
    cmd="${STEP_CMDS[$i]}"
    hint="${STEP_HINTS[$i]}"
    idx=$((i + 1))
    if [ "${idx}" -eq "${total}" ]; then
      # Final step is the summary banner — handled outside run_step
      # so the printed totals reflect the whole pipeline.
      local quick_end total_elapsed
      quick_end="$(date +%s)"
      total_elapsed=$((quick_end - quick_start))
      printf '%s==> step %d/%d: %s%s\n' "${C_CYAN}" "${idx}" "${total}" "${n}" "${C_RESET}"
      printf '\n'
      printf '%s%s==========================================%s\n' "${C_BOLD}" "${C_GREEN}" "${C_RESET}"
      printf '%s%s  ci-local.sh quick — ALL %d STEPS PASS%s\n' "${C_BOLD}" "${C_GREEN}" "${total}" "${C_RESET}"
      printf '%s%s  total: %ds   (target ~3min warm cache)%s\n' "${C_BOLD}" "${C_GREEN}" "${total_elapsed}" "${C_RESET}"
      printf '%s%s==========================================%s\n' "${C_BOLD}" "${C_GREEN}" "${C_RESET}"
      return 0
    fi
    # Don't rely on `set -e` propagating through function boundaries
    # (POSIX errexit-in-functions semantics are implementation-defined
    # and pre-4.4 bash on stock macOS treats them inconsistently).
    # Make the failure path explicit.
    run_step "${idx}" "${total}" "${n}" "${cmd}" "${hint}" || return $?
  done
}

# ---------------------------------------------------------------------
# Visual tier — quick prelude, then Playwright verify (Chromium AND
# WebKit, matching ci.yml e2e-mock) inside the pinned Microsoft
# Playwright Docker image. NO --update-snapshots — this is the
# determinism gate, not the regen path. To regenerate baselines, run
# scripts/update-visual-baselines.sh directly.
# ---------------------------------------------------------------------
visual_tier() {
  # Quick prelude — never burn ~60s of Docker on a tree that
  # fails cheap checks.
  quick_tier

  printf '\n'
  printf '%s==> visual tier: Docker Playwright verify (chromium + webkit)%s\n' "${C_CYAN}" "${C_RESET}"

  # shellcheck source=scripts/lib/playwright.sh
  . "${REPO_ROOT}/scripts/lib/playwright.sh"
  playwright_resolve_image "${REPO_ROOT}"
  if ! playwright_ensure_image; then
    return 1
  fi

  printf '%s==> using %s%s\n' "${C_CYAN}" "${IMAGE}" "${C_RESET}"
  printf '%s==> verifying committed baselines under tests/e2e/visual.spec.ts-snapshots/%s\n' "${C_CYAN}" "${C_RESET}"

  local visual_log="${LOGS_DIR}/visual-verify.log"
  local visual_start visual_end visual_elapsed
  visual_start="$(date +%s)"
  if playwright_docker_run "${REPO_ROOT}" \
    "cd /work && npx playwright test --project=chromium --project=webkit --reporter=line" \
    >"${visual_log}" 2>&1; then
    visual_end="$(date +%s)"
    visual_elapsed=$((visual_end - visual_start))
    printf '\n'
    printf '%s%s==========================================%s\n' "${C_BOLD}" "${C_GREEN}" "${C_RESET}"
    printf '%s%s  ci-local.sh visual — VERIFY PASSED%s\n' "${C_BOLD}" "${C_GREEN}" "${C_RESET}"
    printf '%s%s  docker phase: %ds  (target ~90s warm cache)%s\n' "${C_BOLD}" "${C_GREEN}" "${visual_elapsed}" "${C_RESET}"
    printf '%s%s==========================================%s\n' "${C_BOLD}" "${C_GREEN}" "${C_RESET}"
    return 0
  else
    visual_end="$(date +%s)"
    visual_elapsed=$((visual_end - visual_start))
    RUN_FAILED=1
    cp "${visual_log}" "${LAST_FAILURE_LOG}" 2>/dev/null || true
    printf '%s%sVisual baselines drifted (%ds).%s\n' "${C_BOLD}" "${C_RED}" "${visual_elapsed}" "${C_RESET}" >&2
    echo "--- last 40 lines of Playwright output ---" >&2
    tail -n 40 "${visual_log}" >&2 || true
    echo "--- full log: ${LAST_FAILURE_LOG} ---" >&2
    echo >&2
    echo "To regenerate inside the official image:" >&2
    echo "    bash scripts/update-visual-baselines.sh" >&2
    echo "Then review tests/e2e/visual.spec.ts-snapshots/ and commit." >&2
    return 1
  fi
}

# ---------------------------------------------------------------------
# Full tier — visual + `act -j <job>` for each Linux-runnable CI job.
# macOS / Windows matrix legs are listed as skipped; act has no
# mechanism to honor `runs-on: macos-latest` / `windows-latest`
# selectively, so we don't try to drive those legs through act and
# defer to remote CI for them.
# ---------------------------------------------------------------------
full_tier() {
  visual_tier

  printf '\n'
  printf '%s==> full tier: act replay of ci.yml Linux jobs%s\n' "${C_CYAN}" "${C_RESET}"

  if ! command -v act >/dev/null 2>&1; then
    echo "act is not installed." >&2
    echo "Install (Linux):" >&2
    echo "  curl -fsSL https://raw.githubusercontent.com/nektos/act/master/install.sh \\" >&2
    echo "    | sudo bash -s -- -b /usr/local/bin" >&2
    echo "Or:  brew install act   (macOS)" >&2
    return 2
  fi
  if ! act --version >/dev/null 2>&1; then
    echo "act installed but not runnable" >&2
    return 2
  fi

  # Apple-Silicon x86 emulation warning. .actrc pins
  # --container-architecture linux/amd64 to match remote CI; on arm64
  # hosts that forces Rosetta emulation, which is meaningfully slower
  # for compile-heavy jobs and can fail outright on Colima/OrbStack
  # setups without Rosetta.
  if [ "$(uname -m 2>/dev/null || echo)" = "arm64" ] \
    || [ "$(uname -m 2>/dev/null || echo)" = "aarch64" ]; then
    if grep -q 'linux/amd64' "${REPO_ROOT}/.actrc" 2>/dev/null; then
      printf '%snote: full tier runs under amd64 emulation on arm64 hosts;%s\n' "${C_YELLOW}" "${C_RESET}" >&2
      printf '%s      expect 3-10x slowdown. Ensure Rosetta is enabled in%s\n' "${C_YELLOW}" "${C_RESET}" >&2
      printf '%s      Docker Desktop > Settings > General.%s\n' "${C_YELLOW}" "${C_RESET}" >&2
    fi
  fi

  # Linux-runnable jobs from .github/workflows/ci.yml. macOS / Windows
  # matrix legs and the pull-request-only commit-lint job are skipped
  # — remote CI is authoritative for those.
  local linux_jobs=(
    fmt
    clippy
    clippy-no-default-features
    typecheck
    lint
    deny
    build
    voice-acceptance
    e2e-mock
    no-leak
  )
  local skipped=(
    "test (matrix: 6 legs — Linux exercised by quick tier; macOS/Windows on remote CI)"
    "commit-lint  (pull_request only)"
  )

  # Loop one act invocation per job. `act` historically only honors
  # the LAST `-j` flag when multiple are passed in a single
  # invocation; iterating per-job avoids that footgun and gives us
  # per-job pass/fail surfaces.
  local j fail=0
  for j in "${linux_jobs[@]}"; do
    printf '%s==> act -j %s%s\n' "${C_CYAN}" "${j}" "${C_RESET}"
    if ! act push --workflows .github/workflows/ci.yml -j "${j}"; then
      printf '%s%sFAILED: act -j %s%s\n' "${C_BOLD}" "${C_RED}" "${j}" "${C_RESET}" >&2
      fail=1
    fi
  done
  if [ "${fail}" -ne 0 ]; then
    return 1
  fi

  printf '\n'
  printf '%s%s==========================================%s\n' "${C_BOLD}" "${C_GREEN}" "${C_RESET}"
  printf '%s%s  ci-local.sh full — act replay PASSED%s\n' "${C_BOLD}" "${C_GREEN}" "${C_RESET}"
  printf '%s%s==========================================%s\n' "${C_BOLD}" "${C_GREEN}" "${C_RESET}"
  printf 'Ran in act:    %s\n' "${linux_jobs[*]}"
  printf 'Skipped:%s\n' ""
  for j in "${skipped[@]}"; do
    printf '  %s%s%s\n' "${C_YELLOW}" "${j}" "${C_RESET}"
  done
  printf '%s^ verify on remote CI before merging.%s\n' "${C_YELLOW}" "${C_RESET}"
  return 0
}

# ---------------------------------------------------------------------
# Tier dispatch
# ---------------------------------------------------------------------
TIER="${1:-quick}"
case "${TIER}" in
  -h|--help|help)
    usage
    exit 0
    ;;
  quick)
    quick_tier
    ;;
  visual)
    visual_tier
    ;;
  full)
    full_tier
    ;;
  *)
    echo "Unknown tier: ${TIER}" >&2
    echo >&2
    usage >&2
    exit 2
    ;;
esac
