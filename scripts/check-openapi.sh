#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OPENAPI_FILE="${REPO_ROOT}/docs/openapi.yaml"

if [ "${REDOCLY_LINT:-0}" = "1" ]; then
  if ! command -v npx >/dev/null 2>&1; then
    echo "REDOCLY_LINT=1 requires Node/npx on PATH" >&2
    exit 1
  fi
  npx --yes @redocly/cli lint "${OPENAPI_FILE}"
else
  ruby -ryaml -e 'YAML.load_file(ARGV.fetch(0)); puts "OpenAPI YAML parsed. Set REDOCLY_LINT=1 to run Redocly lint with npx."' "${OPENAPI_FILE}"
fi
