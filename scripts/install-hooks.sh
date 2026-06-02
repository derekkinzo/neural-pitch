#!/usr/bin/env bash
set -euo pipefail

# Installs pre-commit and commit-msg hooks for the neural-pitch repo.

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

pre-commit install --hook-type pre-commit --hook-type commit-msg

echo "✅ pre-commit hooks installed."
