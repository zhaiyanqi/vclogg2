#!/usr/bin/env bash

set -euo pipefail

tag_name=""
if (($# > 0)); then
  if (($# != 2)) || [[ "$1" != "--tag" ]]; then
    echo "Usage: $0 [--tag v<semver>]" >&2
    exit 2
  fi
  tag_name="$2"
fi

version=""
if [[ -n "$tag_name" ]]; then
  version="$tag_name"
else
  version="${VCLOGG2_BUILD_VERSION:-}"
fi
if [[ -z "$version" ]]; then
  version="$(git describe --tags --exact-match --match 'v[0-9]*' HEAD 2>/dev/null || true)"
fi
if [[ -z "$version" ]]; then
  commit="$(git rev-parse --short=12 HEAD 2>/dev/null || true)"
  if [[ -n "$commit" ]]; then
    version="0.0.0-dev+g$commit"
  else
    version="0.0.0"
  fi
fi

version="${version#v}"
number='(0|[1-9][0-9]*)'
prerelease_identifier='(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)'
build_identifier='[0-9A-Za-z-]+'
semver_pattern="^${number}\\.${number}\\.${number}(-${prerelease_identifier}(\\.${prerelease_identifier})*)?(\\+${build_identifier}(\\.${build_identifier})*)?$"
if [[ ! "$version" =~ $semver_pattern ]]; then
  echo "Build version must be a semantic version or v-prefixed semantic version: ${version}" >&2
  exit 1
fi

printf '%s\n' "$version"
