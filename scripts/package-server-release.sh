#!/usr/bin/env bash
set -euo pipefail

script_directory="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)"
if ! command -v node >/dev/null 2>&1; then
  echo 'Node.js 22.12 or newer is required.' >&2
  exit 1
fi
exec node "$script_directory/package-server-release.mjs" "$@"
