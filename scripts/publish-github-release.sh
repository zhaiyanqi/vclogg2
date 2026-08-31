#!/usr/bin/env bash

set -euo pipefail

script_directory="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)"
repository_root="$(CDPATH= cd -- "$script_directory/.." && pwd -P)"
remote_name="origin"
assume_yes=false
tag_name=""

print_usage() {
  echo "Usage: ./scripts/publish-github-release.sh <TAG> [--remote NAME] [--yes]"
  echo
  echo "Validate and push an existing v<Cargo version> tag; GitHub Actions publishes the Release."
}

while (($# > 0)); do
  case "$1" in
    --remote)
      if (($# < 2)); then
        echo "--remote requires a remote name." >&2
        exit 2
      fi
      remote_name="$2"
      shift 2
      ;;
    --yes)
      assume_yes=true
      shift
      ;;
    --help | -h)
      print_usage
      exit 0
      ;;
    -*)
      echo "Unknown argument: $1" >&2
      print_usage >&2
      exit 2
      ;;
    *)
      if [[ -n "$tag_name" ]]; then
        echo "Only one release tag may be specified." >&2
        print_usage >&2
        exit 2
      fi
      tag_name="$1"
      shift
      ;;
  esac
done

if [[ -z "$tag_name" ]]; then
  echo "A release tag is required." >&2
  print_usage >&2
  exit 2
fi

for command_name in cargo git python3; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "Required command not found: $command_name" >&2
    exit 1
  fi
done

cd "$repository_root"

if ! git remote get-url "$remote_name" >/dev/null 2>&1; then
  echo "Git remote does not exist: $remote_name" >&2
  exit 1
fi
remote_url="$(git remote get-url "$remote_name")"
case "$remote_url" in
  https://github.com/zhaiyanqi/vclogg2 | \
    https://github.com/zhaiyanqi/vclogg2.git | \
    https://github.com/zhaiyanqi/vclogg2/ | \
    git@github.com:zhaiyanqi/vclogg2 | \
    git@github.com:zhaiyanqi/vclogg2.git | \
    ssh://git@github.com/zhaiyanqi/vclogg2 | \
    ssh://git@github.com/zhaiyanqi/vclogg2.git) ;;
  *)
    echo "Release remote must point to github.com/zhaiyanqi/vclogg2: $remote_url" >&2
    exit 1
    ;;
esac

branch_name="$(git branch --show-current)"
if [[ "$branch_name" != "main" ]]; then
  echo "Release tags must be created from main; current branch: ${branch_name:-detached HEAD}" >&2
  exit 1
fi

if [[ -n "$(git status --porcelain --untracked-files=normal)" ]]; then
  echo "The working tree must be clean before publishing a release." >&2
  git status --short >&2
  exit 1
fi

version="$({ cargo metadata --no-deps --format-version 1 --locked; } | python3 -c '
import json
import sys

metadata = json.load(sys.stdin)
package = next(item for item in metadata["packages"] if item["name"] == "vclogg2")
print(package["version"])
')"
expected_tag="v$version"
if [[ "$tag_name" != "$expected_tag" ]]; then
  echo "Release tag must match the Cargo version: $expected_tag" >&2
  echo "Received: $tag_name" >&2
  exit 1
fi

if ! git check-ref-format "refs/tags/$tag_name" >/dev/null 2>&1; then
  echo "Invalid release tag: $tag_name" >&2
  exit 1
fi
if ! git show-ref --verify --quiet "refs/tags/$tag_name"; then
  echo "Release tag does not exist locally: $tag_name" >&2
  echo "Create it first, then rerun this script." >&2
  exit 1
fi

echo "Fetching $remote_name/main..."
git fetch "$remote_name" main

local_commit="$(git rev-parse HEAD)"
remote_commit="$(git rev-parse "refs/remotes/$remote_name/main")"
if [[ "$local_commit" != "$remote_commit" ]]; then
  echo "Local main must exactly match $remote_name/main before publishing." >&2
  echo "Local:  $local_commit" >&2
  echo "Remote: $remote_commit" >&2
  exit 1
fi

tag_commit="$(git rev-list -n 1 "$tag_name")"
if [[ "$tag_commit" != "$local_commit" ]]; then
  echo "Release tag must point to the current main commit." >&2
  echo "Tag:  $tag_commit" >&2
  echo "HEAD: $local_commit" >&2
  exit 1
fi
if git ls-remote --exit-code --tags "$remote_name" "refs/tags/$tag_name" >/dev/null 2>&1; then
  echo "Tag already exists on $remote_name: $tag_name" >&2
  exit 1
else
  ls_remote_status=$?
  if ((ls_remote_status != 2)); then
    echo "Could not verify whether $tag_name exists on $remote_name." >&2
    exit "$ls_remote_status"
  fi
fi

echo "Release version: $version"
echo "Release commit:  $local_commit"
echo "Existing tag:    $tag_name"
echo "Push target:     $remote_url"

if [[ "$assume_yes" != true ]]; then
  read -r -p "Run static checks and push existing tag $tag_name? [y/N] " confirmation
  case "$confirmation" in
    y | Y | yes | YES) ;;
    *)
      echo "Release cancelled."
      exit 0
      ;;
  esac
fi

"$script_directory/check.sh"
if ! git push "$remote_name" "refs/tags/$tag_name"; then
  echo "The push failed. No local tags were changed." >&2
  exit 1
fi

echo "Pushed $tag_name. GitHub Actions will build all platforms and publish the Release."
