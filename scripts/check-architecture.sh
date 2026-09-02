#!/usr/bin/env bash
set -euo pipefail

workspace_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$workspace_root"

fail() {
  echo "architecture check failed: $1" >&2
  exit 1
}

search_quiet() {
  local pattern="$1"
  shift

  if command -v rg >/dev/null 2>&1; then
    rg --quiet -- "$pattern" "$@"
  else
    grep -ERq -- "$pattern" "$@"
  fi
}

require_file() {
  [[ -f "$1" ]] || fail "missing required boundary file: $1"
}

forbid_manifest_dependency() {
  local manifest="$1"
  local dependency="$2"
  if search_quiet "^[[:space:]]*${dependency}[[:space:]]*=" "$manifest"; then
    fail "$manifest must not depend on $dependency"
  fi
}

require_file crates/vclogg-core/Cargo.toml
require_file crates/vclogg-core/src/lib.rs
require_file crates/vclogg-data/Cargo.toml
require_file crates/vclogg-data/src/lib.rs
require_file crates/vclogg-app/Cargo.toml
require_file crates/vclogg-app/src/main.rs
require_file .agents/skills/vclogg-core/SKILL.md
require_file .agents/skills/vclogg-data/SKILL.md
require_file .agents/skills/vclogg-app/SKILL.md

workspace_capabilities=(
  document_commands
  document_lifecycle
  document_opening
  document_tasks
  log_presentation
  preferences
  quick_find
  render_shell
  result_export_flow
  search_orchestration
  tab_lifecycle
  viewport_orchestration
  window_registry
)
for capability in "${workspace_capabilities[@]}"; do
  require_file "crates/vclogg-app/src/workspace/${capability}.rs"
done

workspace_line_limit=4000
workspace_line_count="$(wc -l < crates/vclogg-app/src/workspace.rs)"
if (( workspace_line_count > workspace_line_limit )); then
  fail "workspace.rs has ${workspace_line_count} lines; move capabilities into workspace/* modules before exceeding ${workspace_line_limit}"
fi

for layer in core data app; do
  skill=".agents/skills/vclogg-${layer}/SKILL.md"
  search_quiet "^name: vclogg-${layer}$" "$skill" \
    || fail "$skill has an invalid or missing skill name"
  if search_quiet '\[TODO:' "$skill"; then
    fail "$skill contains an unfinished TODO"
  fi
done

for dependency in gpui gpui-base gpui-component rusqlite vclogg-data vclogg-app; do
  forbid_manifest_dependency crates/vclogg-core/Cargo.toml "$dependency"
done

for dependency in gpui gpui-base gpui-component vclogg-app; do
  forbid_manifest_dependency crates/vclogg-data/Cargo.toml "$dependency"
done

forbid_manifest_dependency crates/vclogg-app/Cargo.toml rusqlite

if search_quiet '(^|::)gpui(_base|_component)?\b' crates/vclogg-core/src crates/vclogg-data/src; then
  fail "core and data source must not import GPUI"
fi

if search_quiet 'vclogg_(app|data)' crates/vclogg-core/src crates/vclogg-core/Cargo.toml; then
  fail "core must not depend on app or data"
fi

if search_quiet 'vclogg_app' crates/vclogg-data/src crates/vclogg-data/Cargo.toml; then
  fail "data must not depend on app"
fi

if search_quiet '(^|::)rusqlite\b' crates/vclogg-app/src; then
  fail "app must access SQLite through vclogg-data"
fi

if search_quiet 'confirm_close_and_delete_file|start_move_file_to_trash|关闭并删除文件|Close and delete file' crates/vclogg-app/src; then
  fail "the log viewer must not expose source-file deletion"
fi

echo "architecture boundaries are valid"
