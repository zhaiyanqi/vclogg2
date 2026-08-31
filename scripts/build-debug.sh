#!/usr/bin/env bash

set -euo pipefail

script_directory="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)"
repository_root="$(CDPATH= cd -- "$script_directory/.." && pwd -P)"

cd "$repository_root"
cargo build --workspace --locked
