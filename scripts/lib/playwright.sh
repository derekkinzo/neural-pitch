#!/usr/bin/env bash
# scripts/lib/playwright.sh
#
# Single source of truth for invoking Playwright inside the official
# Microsoft Playwright Docker image. Both `update-visual-baselines.sh`
# and `ci-local.sh visual` source this file so the docker run flags
# (--user, --network host, mount path, HOME=/tmp) cannot drift between
# the regen path and the verify path.
#
# Why Docker: chromium font hinting, freetype version, and fontconfig
# caches drift between developer Linux distros and the GitHub Actions
# ubuntu-latest runner. Pixel diffs from that drift are noise, not
# signal. Running inside the same image CI uses makes committed PNG
# baselines byte-identical between local and CI.
#
# Required: caller exports IMAGE before calling playwright_docker_run.
# IMAGE is derived from package.json devDependencies["@playwright/test"]
# so bumping the npm dep automatically bumps the image.

# Resolve IMAGE from package.json. Caller may override by exporting
# IMAGE before sourcing this file.
playwright_resolve_image() {
  local repo_root="$1"
  if [ -z "${IMAGE:-}" ]; then
    local pw_version
    pw_version=$(node -p "require('${repo_root}/package.json').devDependencies['@playwright/test'].replace(/^[\\^~]/, '')")
    IMAGE="mcr.microsoft.com/playwright:v${pw_version}-noble"
    export IMAGE
  fi
}

# Ensure the pinned Playwright image is present locally; pull if missing.
# Exits non-zero with a clear message if Docker is not installed.
playwright_ensure_image() {
  if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is required for the visual tier (deterministic font rendering)." >&2
    echo "Install Docker Engine or Docker Desktop and re-run." >&2
    return 1
  fi
  if ! docker image inspect "${IMAGE}" >/dev/null 2>&1; then
    echo "==> pulling ${IMAGE} (one-time)"
    docker pull "${IMAGE}"
  fi
}

# Run an arbitrary command inside the Playwright image with the repo
# bind-mounted at /work. All Playwright invocations from this repo go
# through here so flags stay consistent.
#
# Usage: playwright_docker_run "<repo_root>" "<bash -c command string>"
playwright_docker_run() {
  local repo_root="$1"; shift
  docker run --rm \
    --network host \
    -v "${repo_root}:/work" \
    -w /work \
    --user "$(id -u):$(id -g)" \
    -e HOME=/tmp \
    "${IMAGE}" \
    bash -c "$*"
}
