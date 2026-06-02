#!/usr/bin/env bash
set -euo pipefail

# Runs the same checks that pre-commit and CI run.
# Useful for local "validate before pushing."

run() {
  local label="$1"
  shift
  echo
  echo "==> ${label}"
  "$@"
}

run "cargo fmt --check"      cargo fmt --all -- --check
run "cargo clippy"           cargo clippy --workspace --all-targets --all-features -- -D warnings
run "cargo test"             cargo test --workspace
run "npm run typecheck"      npm run typecheck
run "npm run format"         npm run format

echo
echo "✅ all checks passed."
