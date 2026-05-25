#!/usr/bin/env bash
set -euo pipefail

echo "== macOS =="
sw_vers

if command -v mise >/dev/null 2>&1; then
  echo
  echo "== mise =="
  mise --version
  if [ -f mise.toml ]; then
    if rustc_path="$(mise which rustc 2>/dev/null)"; then
      echo "rustc: ${rustc_path}"
      "${rustc_path}" --version
    else
      echo "Rust toolchain from mise.toml is not installed yet. Run: mise trust && mise install"
    fi
  fi
fi

echo
echo "== Cherri =="
if command -v mise >/dev/null 2>&1 && [ -f mise.toml ]; then
  if cherri_path="$(mise which cherri 2>/dev/null)"; then
    echo "cherri: ${cherri_path}"
    "${cherri_path}" --version
  else
    echo "Cherri from mise.toml is not installed yet. Run: mise trust && mise install"
    exit 1
  fi
else
  command -v cherri
  cherri --version
fi

echo
echo "== Shortcuts signing =="
command -v shortcuts
shortcuts help sign | head -n 5

echo
echo "Environment OK"
