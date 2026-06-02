#!/usr/bin/env bash
set -euo pipefail

# Validates a commit message against neural-pitch project policy:
#   - Subject <= 72 chars
#   - Subject matches: ^(core|ui|tauri|audio|dsp|ml|ci|docs|build|test|chore|fix|feat|refactor|perf): [a-z].+$
#   - At least one Signed-off-by: trailer (DCO)
#   - If GIT_AUTHOR_EMAIL is set, the Signed-off-by email must match it
#
# Usage: check-commit-msg.sh <path-to-COMMIT_EDITMSG>

if [[ $# -lt 1 ]]; then
  echo "error: missing path to commit message file" >&2
  echo "usage: $0 <path-to-COMMIT_EDITMSG>" >&2
  exit 2
fi

msg_file="$1"

if [[ ! -f "${msg_file}" ]]; then
  echo "error: commit message file not found: ${msg_file}" >&2
  exit 2
fi

# Strip comment lines (lines starting with '#') as git would.
mapfile -t lines < <(grep -v '^#' "${msg_file}" || true)

# Drop trailing blank lines.
while [[ ${#lines[@]} -gt 0 && -z "${lines[-1]}" ]]; do
  unset 'lines[-1]'
done

if [[ ${#lines[@]} -eq 0 ]]; then
  echo "error: commit message is empty" >&2
  exit 1
fi

subject="${lines[0]}"
errors=0

# 1. Subject length
if [[ ${#subject} -gt 72 ]]; then
  echo "error: subject line exceeds 72 chars (${#subject})" >&2
  echo "       subject: ${subject}" >&2
  errors=1
fi

# 2. Subject prefix + lowercase imperative-mood approximation
subject_re='^(core|ui|tauri|audio|dsp|ml|ci|docs|build|test|chore|fix|feat|refactor|perf): [a-z].+$'
if [[ ! "${subject}" =~ ${subject_re} ]]; then
  echo "error: subject does not match required format" >&2
  echo "       expected: <subsystem>: <lowercase imperative summary>" >&2
  echo "       subsystems: core|ui|tauri|audio|dsp|ml|ci|docs|build|test|chore|fix|feat|refactor|perf" >&2
  echo "       subject:   ${subject}" >&2
  errors=1
fi

# 3. Signed-off-by trailer (DCO)
signoff_lines=()
for line in "${lines[@]}"; do
  if [[ "${line}" =~ ^Signed-off-by:\ .+\ \<.+@.+\>$ ]]; then
    signoff_lines+=("${line}")
  fi
done

if [[ ${#signoff_lines[@]} -eq 0 ]]; then
  echo "error: missing Signed-off-by trailer (DCO)" >&2
  echo "       add one with: git commit -s" >&2
  errors=1
fi

# 4. Signed-off-by email matches GIT_AUTHOR_EMAIL when available
if [[ ${#signoff_lines[@]} -gt 0 && -n "${GIT_AUTHOR_EMAIL:-}" ]]; then
  matched=0
  for line in "${signoff_lines[@]}"; do
    # Extract email between < and >
    email="${line##*<}"
    email="${email%>*}"
    if [[ "${email}" == "${GIT_AUTHOR_EMAIL}" ]]; then
      matched=1
      break
    fi
  done
  if [[ "${matched}" -ne 1 ]]; then
    echo "error: no Signed-off-by email matches GIT_AUTHOR_EMAIL=${GIT_AUTHOR_EMAIL}" >&2
    errors=1
  fi
fi

if [[ "${errors}" -ne 0 ]]; then
  exit 1
fi

echo "OK"
