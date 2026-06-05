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

# Read the @playwright/test version from package.json so the image always
# matches the npm dep.
PW_VERSION=$(node -p "require('./package.json').devDependencies['@playwright/test'].replace(/^[\\^~]/, '')")
IMAGE="mcr.microsoft.com/playwright:v${PW_VERSION}-noble"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "==> using ${IMAGE}"
echo "==> regenerating baselines under ${REPO_ROOT}/tests/e2e/visual.spec.ts-snapshots/"

docker run --rm \
  --network host \
  -v "${REPO_ROOT}:/work" \
  -w /work \
  --user "$(id -u):$(id -g)" \
  -e HOME=/tmp \
  "${IMAGE}" \
  bash -c "cd /work && npx playwright test --project=chromium --update-snapshots --reporter=line"

echo
echo "==> verifying regenerated baselines pass cleanly"
docker run --rm \
  --network host \
  -v "${REPO_ROOT}:/work" \
  -w /work \
  --user "$(id -u):$(id -g)" \
  -e HOME=/tmp \
  "${IMAGE}" \
  bash -c "cd /work && npx playwright test --project=chromium --reporter=line"

echo
echo "Done. Review the diff in tests/e2e/visual.spec.ts-snapshots/ before committing."
