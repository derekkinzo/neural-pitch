#!/usr/bin/env bash
# Runs the same gate CI runs. Use before pushing.
set -euo pipefail

run() {
  local label="$1"
  shift
  echo
  echo "==> ${label}"
  "$@"
}

run "cargo fmt --check"      cargo fmt --all -- --check
run "cargo clippy"           cargo clippy --workspace --all-targets --all-features -- -D warnings
run "cargo test"             cargo test --workspace --all-features
run "npm run typecheck"      npm run typecheck
run "tsc -p tests/e2e"       npx tsc -p tests/e2e/tsconfig.json
run "npm run format --check" npm run format -- --check
run "npm run lint"           npm run lint
# Tier-5 E2E (Playwright); requires `npx playwright install` once per machine.
# Skips if browsers are not installed by exiting non-zero, surfacing the
# missing-browser case to the operator rather than silently passing.
run "npm run e2e"            npm run e2e

echo
echo "all checks passed."
