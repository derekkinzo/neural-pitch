#!/usr/bin/env bash
# Regenerate Playwright visual baselines inside the official Playwright
# Docker image so the bytes match CI's font / freetype / cairo rendering.
#
# Why: chromium-linux baselines drift between developer Linux machines and
# the GitHub Actions ubuntu-latest runner because of subpixel font hinting
# and freetype version differences. Generating them inside the same image
# CI would use eliminates the false positives.
#
# Pre-reqs: docker; the working tree must be clean of build outputs you
# care about (this script binds the repo into the container as /work).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# shellcheck source=scripts/lib/playwright.sh
. "${REPO_ROOT}/scripts/lib/playwright.sh"

playwright_resolve_image "${REPO_ROOT}"
playwright_ensure_image

echo "==> using ${IMAGE}"
echo "==> regenerating baselines under ${REPO_ROOT}/tests/e2e/visual.spec.ts-snapshots/"
playwright_docker_run "${REPO_ROOT}" \
  "cd /work && npx playwright test --project=chromium --update-snapshots --reporter=line"

echo
echo "==> verifying regenerated baselines pass cleanly"
playwright_docker_run "${REPO_ROOT}" \
  "cd /work && npx playwright test --project=chromium --reporter=line"

echo
echo "Done. Review the diff in tests/e2e/visual.spec.ts-snapshots/ before committing."
