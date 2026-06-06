#!/usr/bin/env bash
# Runs the Phase-1 acceptance harness and writes
# docs/reports/phase-1-acceptance.json.
#
# Wire-format contract with the Rust harness (test target
# `acceptance_voice` in crates/neural-pitch-core):
#
#   - Per-fixture line:
#       [ACCEPT-FIXTURE] {filename}: pass={true|false} \
#         estimated_midi={N} expected={N} cents_error={f}
#
#   - Aggregate line (single-line JSON, marker-prefixed for
#     grep-extraction without parsing test output):
#       === ACCEPTANCE_JSON === { ... }
#
#     Required keys on the JSON object:
#       aggregate, tier_1_count, tier_2_count,
#       latency_p50_ms, latency_p99_ms
#
# Output JSON shape (this script writes):
#   { aggregate, fixtures[], tier_1_count, tier_2_count,
#     latency_p50_ms, latency_p99_ms, timestamp, commit_sha }
#
# When invoked with `--write-closeout`, the script also substitutes
# the literal `<SHA>` placeholder in DESIGN.md §13.2 and
# PHASE-1-CLOSEOUT.md Status with the actual commit SHA, so the
# closeout text and the JSON cannot drift. The substitution is
# **off** by default — the pre-push gate is a verification step and
# must not mutate tracked files in the working tree (a hidden rewrite
# would surface as an unexpected `git status` after `git push`).
#
# Exit non-zero if aggregate < 0.95 (the §13.2 acceptance floor).

set -euo pipefail

# --- argument parsing ----------------------------------------------
WRITE_CLOSEOUT=0
for arg in "$@"; do
  case "${arg}" in
    --write-closeout)
      WRITE_CLOSEOUT=1
      ;;
    -h|--help)
      cat <<'EOF'
Usage: scripts/run-acceptance.sh [--write-closeout]

Runs the Phase-1 acceptance harness and writes
docs/reports/phase-1-acceptance.json.

Options:
  --write-closeout   substitute the literal `<SHA>` placeholder in
                     docs/design/DESIGN.md and
                     docs/design/PHASE-1-CLOSEOUT.md with the current
                     commit SHA. OFF by default — the pre-push gate
                     never mutates tracked files. Use this flag only
                     when you are intentionally finalising the
                     closeout doc.
EOF
      exit 0
      ;;
    *)
      echo "error: unknown argument: ${arg}" >&2
      echo "  run with --help for usage" >&2
      exit 2
      ;;
  esac
done

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPORT_DIR="${REPO_ROOT}/docs/reports"
REPORT_PATH="${REPORT_DIR}/phase-1-acceptance.json"

mkdir -p "${REPORT_DIR}"

LOG_FILE="$(mktemp -t accept-cargo-XXXXXX.log)"
trap 'rm -f "${LOG_FILE}"' EXIT

echo "==> running acceptance harness"
# `tee` so the operator sees the cargo output in real time and we
# still have the full log to parse afterwards. `--release` is used
# because Tier-2 fixtures are CPU-bound; debug builds pad the
# per-fixture deadline (the harness scales it on cfg(debug_assertions))
# but release is meaningfully faster on CI.
set +e
(
  cd "${REPO_ROOT}" && \
    cargo test -p neural-pitch-core --release --test acceptance_voice -- --nocapture
) 2>&1 | tee "${LOG_FILE}"
CARGO_STATUS="${PIPESTATUS[0]}"
set -e

if [ "${CARGO_STATUS}" -ne 0 ]; then
  echo "error: cargo test exited with status ${CARGO_STATUS}" >&2
  exit "${CARGO_STATUS}"
fi

# --- parse per-fixture lines into a JSON array ---------------------
# `parsed_count` is awk-side success. `seen_count` is the raw count
# of `[ACCEPT-FIXTURE]` lines emitted by the harness. If they
# diverge, the harness emitted a malformed line — fail loudly with
# the offending line so a future regression cannot silently land
# fewer fixtures than ran.
FIXTURES_JSON_AND_COUNT="$(
  awk '
    /^\[ACCEPT-FIXTURE\]/ {
      # Expected layout:
      #   [ACCEPT-FIXTURE] <file>: pass=<bool> estimated_midi=<int> \
      #     expected=<int> cents_error=<float>
      # Note: `exp` is mawk built-in (exp()), so we use `expd` here.
      file = ""; pass = ""; est = ""; expd = ""; cents = ""
      for (i = 2; i <= NF; i++) {
        tok = $i
        if (tok ~ /:$/) {
          file = substr(tok, 1, length(tok) - 1)
        } else if (tok ~ /^pass=/) {
          pass = substr(tok, 6)
        } else if (tok ~ /^estimated_midi=/) {
          est = substr(tok, 16)
        } else if (tok ~ /^expected=/) {
          expd = substr(tok, 10)
        } else if (tok ~ /^cents_error=/) {
          cents = substr(tok, 13)
        }
      }
      if (file == "" || pass == "" || est == "" || expd == "" || cents == "") {
        # Surface the offending line on stderr so the operator can
        # see exactly where the harness contract drifted.
        print "[run-acceptance] malformed [ACCEPT-FIXTURE] line: " $0 > "/dev/stderr"
        next
      }
      if (n++ > 0) printf(",")
      # Filename JSON-escape: the harness writes plain fixture
      # filenames (alnum + underscore + dot), so a backslash/quote
      # escape is sufficient. If that ever changes, switch to a
      # real JSON encoder here.
      gsub(/\\/, "\\\\", file)
      gsub(/"/,  "\\\"", file)
      printf("{\"file\":\"%s\",\"pass\":%s,\"estimated_midi\":%s,\"expected\":%s,\"cents_error\":%s}",
             file, pass, est, expd, cents)
    }
    END {
      # Append a sentinel line `__PARSED__=<n>` so the calling
      # shell can sed it off and compare against the raw
      # [ACCEPT-FIXTURE] line count.
      print ""
      print "__PARSED__=" (0 + n)
    }
  ' "${LOG_FILE}"
)"

FIXTURES_JSON="$(printf '%s' "${FIXTURES_JSON_AND_COUNT}" | head -n 1)"
PARSED_COUNT="$(printf '%s' "${FIXTURES_JSON_AND_COUNT}" \
  | grep -E '^__PARSED__=' \
  | sed -E 's/^__PARSED__=//')"
SEEN_COUNT="$(grep -c '^\[ACCEPT-FIXTURE\]' "${LOG_FILE}" || true)"

if [ "${PARSED_COUNT:-0}" != "${SEEN_COUNT:-0}" ]; then
  echo "error: parsed ${PARSED_COUNT:-0} [ACCEPT-FIXTURE] line(s) but harness emitted ${SEEN_COUNT:-0}" >&2
  echo "  malformed lines were printed above on stderr" >&2
  exit 1
fi

# --- pull the aggregate single-line JSON ---------------------------
AGG_LINE="$(grep -E '^=== ACCEPTANCE_JSON === ' "${LOG_FILE}" | tail -n 1 || true)"
if [ -z "${AGG_LINE}" ]; then
  echo "error: no '=== ACCEPTANCE_JSON ===' marker line in harness output" >&2
  exit 1
fi
AGG_JSON="${AGG_LINE#=== ACCEPTANCE_JSON === }"

# Validate required keys are present in the aggregate JSON. We do
# not fully parse JSON here — a substring check is enough to catch
# the harness regressing the contract.
for key in aggregate tier_1_count tier_2_count latency_p50_ms latency_p99_ms; do
  if ! printf '%s' "${AGG_JSON}" | grep -q "\"${key}\""; then
    echo "error: aggregate JSON missing required key: ${key}" >&2
    echo "  aggregate line was: ${AGG_LINE}" >&2
    exit 1
  fi
done

# Pull the scalar fields out of the aggregate JSON.
# Numbers are emitted by the Rust harness as bare JSON numbers, so
# a permissive regex is fine here.
extract_number() {
  # $1 = key
  printf '%s' "${AGG_JSON}" \
    | grep -oE "\"$1\"[[:space:]]*:[[:space:]]*-?[0-9]+(\.[0-9]+)?" \
    | head -n 1 \
    | sed -E "s/.*:[[:space:]]*//"
}

AGGREGATE="$(extract_number aggregate)"
TIER1="$(extract_number tier_1_count)"
TIER2="$(extract_number tier_2_count)"
P50="$(extract_number latency_p50_ms)"
P99="$(extract_number latency_p99_ms)"

if [ -z "${AGGREGATE}" ] || [ -z "${TIER1}" ] || [ -z "${TIER2}" ] \
   || [ -z "${P50}" ] || [ -z "${P99}" ]; then
  echo "error: failed to extract one or more aggregate fields" >&2
  echo "  aggregate=${AGGREGATE} tier_1=${TIER1} tier_2=${TIER2}" >&2
  echo "  p50=${P50} p99=${P99}" >&2
  exit 1
fi

# --- timestamp + commit sha ----------------------------------------
TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

if COMMIT_SHA="$(cd "${REPO_ROOT}" && git rev-parse HEAD 2>/dev/null)"; then
  :
else
  COMMIT_SHA="unknown"
fi

# --- write the report JSON -----------------------------------------
{
  printf '{\n'
  printf '  "aggregate": %s,\n' "${AGGREGATE}"
  printf '  "fixtures": [%s],\n' "${FIXTURES_JSON}"
  printf '  "tier_1_count": %s,\n' "${TIER1}"
  printf '  "tier_2_count": %s,\n' "${TIER2}"
  printf '  "latency_p50_ms": %s,\n' "${P50}"
  printf '  "latency_p99_ms": %s,\n' "${P99}"
  printf '  "timestamp": "%s",\n' "${TIMESTAMP}"
  printf '  "commit_sha": "%s"\n' "${COMMIT_SHA}"
  printf '}\n'
} > "${REPORT_PATH}"

echo
echo "==> wrote ${REPORT_PATH}"
echo "    aggregate=${AGGREGATE} tier_1=${TIER1} tier_2=${TIER2} p50=${P50}ms p99=${P99}ms"

# --- substitute `<SHA>` placeholder in closeout markdown -----------
# DESIGN.md §13.2 Status and PHASE-1-CLOSEOUT.md Status both carry a
# literal `<SHA>` placeholder. We rewrite both atomically in-place.
# This step is idempotent: a second run with the same SHA writes the
# same string. If git was unavailable above (`COMMIT_SHA == unknown`)
# we skip the rewrite — it is safer to leave the placeholder than to
# stamp a misleading "unknown" into the closeout text.
#
# We use awk -v to avoid shell-quoting hell around the backtick chars,
# and write to a tmp file then mv-replace so the substitution is
# atomic on the filesystem.
substitute_sha() {
  local md="$1"
  local sha="$2"
  if [ ! -f "${md}" ]; then
    return 0
  fi
  local tmp
  tmp="$(mktemp)"
  awk -v sha="${sha}" '
    {
      gsub(/`<SHA>`/, "`" sha "`")
      print
    }
  ' "${md}" > "${tmp}"
  mv "${tmp}" "${md}"
}
if [ "${WRITE_CLOSEOUT}" -eq 1 ] && [ "${COMMIT_SHA}" != "unknown" ]; then
  substitute_sha "${REPO_ROOT}/docs/design/DESIGN.md" "${COMMIT_SHA}"
  substitute_sha "${REPO_ROOT}/docs/design/PHASE-1-CLOSEOUT.md" "${COMMIT_SHA}"
  echo "==> substituted <SHA> placeholder with ${COMMIT_SHA} in closeout markdown"
elif [ "${WRITE_CLOSEOUT}" -eq 0 ]; then
  echo "==> closeout SHA substitution skipped (pass --write-closeout to enable)"
fi

# --- enforce the §13.2 floor ---------------------------------------
# awk handles the float compare without depending on bc(1).
if ! awk -v a="${AGGREGATE}" 'BEGIN { exit !(a + 0 >= 0.95) }'; then
  echo "error: aggregate ${AGGREGATE} is below the 0.95 acceptance floor" >&2
  exit 1
fi

echo "==> acceptance floor met (>= 0.95)"
