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
run "npm run format --check" npm run format -- --check
run "npm run lint"           npm run lint

echo
echo "all checks passed."
