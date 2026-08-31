#!/bin/sh

set -eu

archive_path=
install_directory=
wait_pid=
platform=
launch=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --archive)
      archive_path=${2-}
      shift 2
      ;;
    --install-directory)
      install_directory=${2-}
      shift 2
      ;;
    --wait-pid)
      wait_pid=${2-}
      shift 2
      ;;
    --platform)
      platform=${2-}
      shift 2
      ;;
    --launch)
      launch=1
      shift
      ;;
    *)
      echo "Unknown update-helper argument: $1" >&2
      exit 2
      ;;
  esac
done

if [ ! -f "$archive_path" ] || [ -z "$install_directory" ]; then
  echo "The update archive and install directory are required." >&2
  exit 2
fi
case "$wait_pid" in
  '' | *[!0-9]*)
    echo "The process id is invalid." >&2
    exit 2
    ;;
esac
case "$platform" in
  macos | linux) ;;
  *)
    echo "Unsupported update platform: $platform" >&2
    exit 2
    ;;
esac

while kill -0 "$wait_pid" 2>/dev/null; do
  sleep 1
done

stage_root=$(mktemp -d "${TMPDIR:-/tmp}/vclogg2-update.XXXXXX")
cleanup() {
  rm -rf -- "$stage_root"
}
trap cleanup EXIT HUP INT TERM

case "$platform" in
  macos)
    ditto -x -k "$archive_path" "$stage_root"
    installer_name=Install-VCLogg2-macos.sh
    ;;
  linux)
    tar -xzf "$archive_path" -C "$stage_root"
    installer_name=Install-VCLogg2-linux.sh
    ;;
esac

installer_path=
for candidate in "$stage_root"/vclogg2-*/"$installer_name" "$stage_root"/"$installer_name"; do
  if [ -f "$candidate" ]; then
    if [ -n "$installer_path" ]; then
      echo "The update package contains multiple installers." >&2
      exit 1
    fi
    installer_path=$candidate
  fi
done
if [ -z "$installer_path" ]; then
  echo "The update package is incomplete: $installer_name is missing." >&2
  exit 1
fi

chmod 700 "$installer_path"
if [ "$launch" -eq 1 ]; then
  "$installer_path" --install-directory "$install_directory" --launch
else
  "$installer_path" --install-directory "$install_directory"
fi
