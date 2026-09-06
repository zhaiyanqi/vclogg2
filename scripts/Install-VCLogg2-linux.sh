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
icon_directory="$data_directory/icons/hicolor/1024x1024/apps"
mime_package_directory="$data_directory/mime/packages"
mkdir -p \
  "$binary_directory" \
  "$application_directory" \
  "$icon_directory" \
  "$mime_package_directory"
ln -sfn "$installed_executable" "$binary_directory/vclogg2"
if [ -f "$source_icon" ]; then
  install -m 644 "$source_icon" "$icon_directory/com.vclogg2.desktop.png"
  # Replace the older installer's size entry so it cannot mask the new default.
  rm -f "$data_directory/icons/hicolor/512x512/apps/com.vclogg2.desktop.png"
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

# Wayland resolves the running window's app ID through a desktop entry. Keep
# alternate entries out of launch menus and file associations; they only supply
# the selected running icon. The normal launcher remains the default A icon.
for icon in compact illustration sticker; do
  alternate_icon="$script_directory/icons/$icon.png"
  if [ -f "$alternate_icon" ]; then
    desktop_id="com.vclogg2.desktop.$icon"
    install -m 644 "$alternate_icon" "$icon_directory/$desktop_id.png"
    {
      echo '[Desktop Entry]'
      echo 'Type=Application'
      echo 'Name=VCLogg2'
      printf 'Exec="%s" %%F\n' "$installed_executable"
      printf 'Icon=%s\n' "$desktop_id"
      echo 'NoDisplay=true'
      echo 'Terminal=false'
      echo 'StartupNotify=true'
    } >"$application_directory/$desktop_id.desktop"
    chmod 644 "$application_directory/$desktop_id.desktop"
  fi
done

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
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$data_directory/icons/hicolor" >/dev/null 2>&1 || true
fi

echo "VCLogg2 installed at: $installed_executable"
if [ "$launch" -eq 1 ]; then
  "$installed_executable" >/dev/null 2>&1 &
fi
