#!/usr/bin/env bash

set -euo pipefail

script_directory="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)"
repository_root="$(CDPATH= cd -- "$script_directory/.." && pwd -P)"
requested_output_directory="${1:-$repository_root/dist}"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "package-release-linux.sh must be run on Linux." >&2
  exit 1
fi

case "$(uname -m)" in
  x86_64)
    architecture="x86_64"
    ;;
  aarch64 | arm64)
    architecture="aarch64"
    ;;
  *)
    echo "Unsupported Linux architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

cd "$repository_root"
version="$({ cargo metadata --no-deps --format-version 1 --locked; } | python3 -c '
import json
import sys

metadata = json.load(sys.stdin)
package = next(item for item in metadata["packages"] if item["name"] == "vclogg2")
print(package["version"])
')"

"$script_directory/build-release.sh"

mkdir -p "$requested_output_directory"
output_root="$(CDPATH= cd -- "$requested_output_directory" && pwd -P)"
mkdir -p "$output_root/linux-$architecture"
output_directory="$(CDPATH= cd -- "$output_root/linux-$architecture" && pwd -P)"
stage_name="vclogg2-$version-linux-$architecture"
stage_directory="$output_directory/$stage_name"
archive_path="$output_directory/$stage_name.tar.gz"

case "$stage_directory" in
  "$output_directory"/vclogg2-*-linux-*) ;;
  *)
    echo "Refusing to replace unexpected stage directory: $stage_directory" >&2
    exit 1
    ;;
esac

rm -rf -- "$stage_directory"
rm -f -- "$archive_path"
mkdir -p "$stage_directory"

install -m 755 "$repository_root/target/release/vclogg2" "$stage_directory/vclogg2"
install -m 644 "$repository_root/README.md" "$stage_directory/README.md"
install -m 644 "$repository_root/LICENSE" "$stage_directory/LICENSE"
install -m 644 \
  "$repository_root/crates/vclogg-app/resources/windows/vclogg2.png" \
  "$stage_directory/vclogg2.png"
install -m 755 \
  "$script_directory/Install-VCLogg2-linux.sh" \
  "$stage_directory/Install-VCLogg2-linux.sh"
install -m 755 \
  "$script_directory/Apply-VCLogg2Update.sh" \
  "$stage_directory/Apply-VCLogg2Update.sh"

tar -C "$output_directory" -czf "$archive_path" "$stage_name"
"$script_directory/write-update-metadata.py" \
  --archive "$archive_path" \
  --platform linux \
  --architecture "$architecture" \
  --version "$version" \
  --blockmap-name "$stage_name.blockmap.json"

echo "Linux portable directory: $stage_directory"
echo "Linux release package: $archive_path"
echo "Linux update feed: $output_directory/latest.json"
