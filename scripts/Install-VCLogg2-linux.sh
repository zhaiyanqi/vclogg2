#!/bin/sh

set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
source_executable="$script_directory/vclogg2"
source_icon="$script_directory/vclogg2.png"
install_directory="${HOME}/.local/lib/vclogg2"
launch=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --install-directory)
      install_directory=${2-}
      shift 2
      ;;
    --launch)
      launch=1
      shift
      ;;
    *)
      echo "Unknown installer argument: $1" >&2
      exit 2
      ;;
  esac
done

if [ ! -f "$source_executable" ] || [ -z "$install_directory" ]; then
  echo "The package executable and install directory are required." >&2
  exit 2
fi

mkdir -p "$install_directory"
installed_executable="$install_directory/vclogg2"
temporary_executable="$install_directory/.vclogg2.new-$$"
install -m 755 "$source_executable" "$temporary_executable"
mv -f "$temporary_executable" "$installed_executable"

for document in README.md LICENSE; do
  if [ -f "$script_directory/$document" ]; then
    install -m 644 "$script_directory/$document" "$install_directory/$document"
  fi
done

binary_directory="${HOME}/.local/bin"
data_directory="${XDG_DATA_HOME:-${HOME}/.local/share}"
application_directory="$data_directory/applications"
icon_directory="$data_directory/icons/hicolor/512x512/apps"
mime_package_directory="$data_directory/mime/packages"
mkdir -p \
  "$binary_directory" \
  "$application_directory" \
  "$icon_directory" \
  "$mime_package_directory"
ln -sfn "$installed_executable" "$binary_directory/vclogg2"
if [ -f "$source_icon" ]; then
  install -m 644 "$source_icon" "$icon_directory/com.vclogg2.desktop.png"
fi

desktop_file="$application_directory/com.vclogg2.desktop.desktop"
{
  echo '[Desktop Entry]'
  echo 'Type=Application'
  echo 'Name=VCLogg2'
  echo 'Comment=Large log file viewer'
  printf 'Exec="%s" %%F\n' "$installed_executable"
  echo 'Icon=com.vclogg2.desktop'
  echo 'Terminal=false'
  echo 'Categories=Utility;Development;'
  echo 'MimeType=application/x-vclogg2-log;text/plain;application/json;text/csv;'
  echo 'StartupNotify=true'
} >"$desktop_file"
chmod 644 "$desktop_file"

mime_package="$mime_package_directory/com.vclogg2.desktop.xml"
{
  echo '<?xml version="1.0" encoding="UTF-8"?>'
  echo '<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">'
  echo '  <mime-type type="application/x-vclogg2-log">'
  echo '    <comment>Log or trace document</comment>'
  echo '    <glob pattern="*.log"/>'
  echo '    <glob pattern="*.out"/>'
  echo '    <glob pattern="*.trace"/>'
  echo '  </mime-type>'
  echo '</mime-info>'
} >"$mime_package"
chmod 644 "$mime_package"
if command -v update-mime-database >/dev/null 2>&1; then
  update-mime-database "$data_directory/mime" >/dev/null 2>&1 || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$application_directory" >/dev/null 2>&1 || true
fi

echo "VCLogg2 installed at: $installed_executable"
if [ "$launch" -eq 1 ]; then
  "$installed_executable" >/dev/null 2>&1 &
fi
