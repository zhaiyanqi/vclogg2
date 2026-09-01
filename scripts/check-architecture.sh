#!/usr/bin/env bash
set -euo pipefail

workspace_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$workspace_root"

fail() {
  echo "architecture check failed: $1" >&2
  exit 1
}

require_file() {
  [[ -f "$1" ]] || fail "missing required boundary file: $1"
}

forbid_manifest_dependency() {
  local manifest="$1"
  local dependency="$2"
  if rg --quiet "^[[:space:]]*${dependency}[[:space:]]*=" "$manifest"; then
    fail "$manifest must not depend on $dependency"
  fi
}

require_file crates/vclogg-core/Cargo.toml
require_file crates/vclogg-core/src/lib.rs
require_file crates/vclogg-data/Cargo.toml
require_file crates/vclogg-data/src/lib.rs
require_file crates/vclogg-app/Cargo.toml
require_file crates/vclogg-app/src/main.rs

for dependency in gpui gpui-base gpui-component rusqlite vclogg-data vclogg-app; do
  forbid_manifest_dependency crates/vclogg-core/Cargo.toml "$dependency"
done

for dependency in gpui gpui-base gpui-component vclogg-app; do
  forbid_manifest_dependency crates/vclogg-data/Cargo.toml "$dependency"
done

if rg --quiet '(^|::)gpui(_base|_component)?\b' crates/vclogg-core/src crates/vclogg-data/src; then
  fail "core and data source must not import GPUI"
fi

if rg --quiet 'vclogg_(app|data)' crates/vclogg-core/src crates/vclogg-core/Cargo.toml; then
  fail "core must not depend on app or data"
fi

if rg --quiet 'vclogg_app' crates/vclogg-data/src crates/vclogg-data/Cargo.toml; then
  fail "data must not depend on app"
fi

echo "architecture boundaries are valid"
