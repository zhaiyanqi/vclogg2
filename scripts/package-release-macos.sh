#!/usr/bin/env bash

set -euo pipefail

script_directory="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)"
repository_root="$(CDPATH= cd -- "$script_directory/.." && pwd -P)"
requested_output_directory="${1:-$repository_root/dist}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "package-release-macos.sh must be run on macOS." >&2
  exit 1
fi

case "$(uname -m)" in
  arm64 | aarch64)
    architecture="aarch64"
    ;;
  x86_64)
    architecture="x86_64"
    ;;
  *)
    echo "Unsupported macOS architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

cd "$repository_root"
version="$("$script_directory/resolve-build-version.sh")"
export VCLOGG2_BUILD_VERSION="$version"

"$script_directory/build-release.sh"

mkdir -p "$requested_output_directory"
output_root="$(CDPATH= cd -- "$requested_output_directory" && pwd -P)"
mkdir -p "$output_root/macos-$architecture"
output_directory="$(CDPATH= cd -- "$output_root/macos-$architecture" && pwd -P)"
stage_name="vclogg2-$version-macos-$architecture"
stage_directory="$output_directory/.$stage_name-stage"
disk_image_root="$stage_directory/disk-image"
app_directory="$disk_image_root/VCLogg2.app"
contents_directory="$app_directory/Contents"
resources_directory="$contents_directory/Resources"
disk_image_path="$output_directory/$stage_name.dmg"
iconset_directory="$output_directory/VCLogg2.iconset"
source_icon="$repository_root/crates/vclogg-app/resources/windows/vclogg2.png"

case "$stage_directory" in
  "$output_directory"/.vclogg2-*-macos-*-stage) ;;
  *)
    echo "Refusing to replace unexpected stage directory: $stage_directory" >&2
    exit 1
    ;;
esac

cleanup() {
  rm -rf -- "$stage_directory" "$iconset_directory"
}
trap cleanup EXIT

cleanup
rm -f -- "$disk_image_path"
mkdir -p "$contents_directory/MacOS" "$resources_directory" "$iconset_directory"

install -m 755 "$repository_root/target/release/vclogg2" "$contents_directory/MacOS/vclogg2"
install -m 644 "$repository_root/README.md" "$resources_directory/README.md"
install -m 644 "$repository_root/LICENSE" "$resources_directory/LICENSE"

sips -z 16 16 "$source_icon" --out "$iconset_directory/icon_16x16.png" >/dev/null
sips -z 32 32 "$source_icon" --out "$iconset_directory/icon_16x16@2x.png" >/dev/null
sips -z 32 32 "$source_icon" --out "$iconset_directory/icon_32x32.png" >/dev/null
sips -z 64 64 "$source_icon" --out "$iconset_directory/icon_32x32@2x.png" >/dev/null
sips -z 128 128 "$source_icon" --out "$iconset_directory/icon_128x128.png" >/dev/null
sips -z 256 256 "$source_icon" --out "$iconset_directory/icon_128x128@2x.png" >/dev/null
sips -z 256 256 "$source_icon" --out "$iconset_directory/icon_256x256.png" >/dev/null
sips -z 512 512 "$source_icon" --out "$iconset_directory/icon_256x256@2x.png" >/dev/null
sips -z 512 512 "$source_icon" --out "$iconset_directory/icon_512x512.png" >/dev/null
sips -z 1024 1024 "$source_icon" --out "$iconset_directory/icon_512x512@2x.png" >/dev/null
python3 - "$iconset_directory" "$resources_directory/VCLogg2.icns" <<'PY'
import pathlib
import struct
import sys

iconset = pathlib.Path(sys.argv[1])
output = pathlib.Path(sys.argv[2])
entries = [
    ("icp4", "icon_16x16.png"),
    ("icp5", "icon_32x32.png"),
    ("ic07", "icon_128x128.png"),
    ("ic08", "icon_256x256.png"),
    ("ic09", "icon_512x512.png"),
    ("ic11", "icon_16x16@2x.png"),
    ("ic12", "icon_32x32@2x.png"),
    ("ic13", "icon_128x128@2x.png"),
    ("ic14", "icon_256x256@2x.png"),
    ("ic10", "icon_512x512@2x.png"),
]

payload = b""
for icon_type, file_name in entries:
    data = (iconset / file_name).read_bytes()
    payload += icon_type.encode("ascii") + struct.pack(">I", len(data) + 8) + data

output.write_bytes(b"icns" + struct.pack(">I", len(payload) + 8) + payload)
PY
rm -rf -- "$iconset_directory"

cat >"$contents_directory/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>VCLogg2</string>
  <key>CFBundleExecutable</key>
  <string>vclogg2</string>
  <key>CFBundleIconFile</key>
  <string>VCLogg2</string>
  <key>CFBundleIdentifier</key>
  <string>com.vclogg2.desktop</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>VCLogg2</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeExtensions</key>
      <array>
        <string>log</string>
        <string>txt</string>
        <string>out</string>
        <string>trace</string>
        <string>csv</string>
        <string>json</string>
      </array>
      <key>CFBundleTypeName</key>
      <string>Log or text document</string>
      <key>CFBundleTypeRole</key>
      <string>Viewer</string>
      <key>LSHandlerRank</key>
      <string>Alternate</string>
    </dict>
  </array>
  <key>CFBundleShortVersionString</key>
  <string>$version</string>
  <key>CFBundleVersion</key>
  <string>$version</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF

plutil -lint "$contents_directory/Info.plist"
codesign --force --sign - "$app_directory"
ln -s /Applications "$disk_image_root/Applications"
hdiutil create \
  -srcfolder "$disk_image_root" \
  -volname VCLogg2 \
  -fs HFS+ \
  -format UDZO \
  -imagekey zlib-level=9 \
  "$disk_image_path"
hdiutil verify "$disk_image_path"
echo "macOS release disk image: $disk_image_path"
