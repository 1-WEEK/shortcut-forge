#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

SERVER_URL="${SERVER_URL:-http://127.0.0.1:8787}"
SERVER_AUTH_TOKEN="${SERVER_AUTH_TOKEN:-}"

if [ -z "${SERVER_AUTH_TOKEN}" ]; then
  echo "SERVER_AUTH_TOKEN is required for POST /api/builds" >&2
  exit 1
fi

echo "Posting minimal build to ${SERVER_URL}"
response="$(
  curl -sS \
    -X POST "${SERVER_URL}/api/builds" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer ${SERVER_AUTH_TOKEN}" \
    --data-binary @"${REPO_ROOT}/docs/examples/minimal-request.json"
)"

echo "${response}"

download_url="$(
  printf '%s' "${response}" \
    | sed -n 's/.*"download_url"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p'
)"

if [ -z "${download_url}" ]; then
  echo "No download_url found in response" >&2
  exit 1
fi

echo
echo "Fetching signed shortcut from ${download_url}"
curl -fsS -o /tmp/minimal.signed.shortcut "${download_url}"
ls -lh /tmp/minimal.signed.shortcut

echo
echo "Smoke build OK: /tmp/minimal.signed.shortcut"
