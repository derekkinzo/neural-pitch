#!/usr/bin/env bash
# Back-compat shim. The real entry point is scripts/ci-local.sh.
# New docs reference ci-local.sh exclusively; this shim keeps existing
# muscle memory and any stale links working.
exec "$(dirname "$0")/ci-local.sh" quick "$@"
