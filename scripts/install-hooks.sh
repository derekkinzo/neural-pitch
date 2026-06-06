#!/usr/bin/env bash
set -euo pipefail

# Installs pre-commit, commit-msg, AND pre-push hooks for the
# neural-pitch repo. The pre-push hook wires `scripts/ci-local.sh
# quick` as the canonical pre-push gate (see ADR-0022 and
# CONTRIBUTING.md). Without --hook-type pre-push the gate is silently
# inactive — devs would push red into CI without warning.

if ! command -v pre-commit >/dev/null 2>&1; then
  cat >&2 <<'EOF'
error: pre-commit is not installed.

Install it via one of:
  pip install --user pre-commit
  pipx install pre-commit
  brew install pre-commit

Then re-run: scripts/install-hooks.sh
EOF
  exit 1
fi

pre-commit install \
  --hook-type pre-commit \
  --hook-type commit-msg \
  --hook-type pre-push

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "ok: pre-commit hooks installed."
for h in pre-commit commit-msg pre-push; do
  if [ -f "${REPO_ROOT}/.git/hooks/${h}" ]; then
    echo "  installed: ${h}"
  else
    echo "  MISSING:   ${h}" >&2
  fi
done
echo
echo "verify with: scripts/ci-local.sh quick"
