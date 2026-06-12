#!/usr/bin/env bash
# Alias for `scripts/ci-local.sh quick`. ci-local.sh is the canonical
# entry point; this shim exists so older invocations keep working.
exec "$(dirname "$0")/ci-local.sh" quick "$@"
