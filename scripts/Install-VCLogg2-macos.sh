#!/bin/sh

set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
source_application="$script_directory/VCLogg2.app"
install_application="${HOME}/Applications/VCLogg2.app"
launch=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --install-directory)
      install_application=${2-}
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

if [ ! -d "$source_application" ] || [ -z "$install_application" ]; then
  echo "The package application and install destination are required." >&2
  exit 2
fi
case "$install_application" in
  *.app) ;;
  *)
    echo "The macOS install destination must end in .app." >&2
    exit 2
    ;;
esac

install_parent=$(dirname -- "$install_application")
install_name=$(basename -- "$install_application")
temporary_application="$install_parent/.${install_name}.new-$$"
backup_application="$install_parent/.${install_name}.old-$$"
mkdir -p "$install_parent"
rm -rf -- "$temporary_application" "$backup_application"
ditto "$source_application" "$temporary_application"

had_previous=0
if [ -e "$install_application" ]; then
  mv "$install_application" "$backup_application"
  had_previous=1
fi
if ! mv "$temporary_application" "$install_application"; then
  if [ "$had_previous" -eq 1 ]; then
    mv "$backup_application" "$install_application"
  fi
  exit 1
fi
rm -rf -- "$backup_application"

launch_services=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
if [ -x "$launch_services" ]; then
  "$launch_services" -f "$install_application" >/dev/null 2>&1 || true
fi

echo "VCLogg2 installed at: $install_application"
if [ "$launch" -eq 1 ]; then
  open "$install_application"
fi
