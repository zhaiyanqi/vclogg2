#!/usr/bin/env python3
"""Generate the website update feed from a published GitHub Release JSON file."""
import argparse
from datetime import datetime
import json
from pathlib import Path
import re

RELEASES_URL = 'https://github.com/zhaiyanqi/vclogg2/releases'
STABLE_TAG = re.compile(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('release', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    release = json.loads(args.release.read_text(encoding='utf-8'))
    tag = release['tag_name']
    if release['draft'] or release['prerelease'] or not STABLE_TAG.fullmatch(tag):
        raise ValueError('The update feed requires a published stable v<semver> release')
    version = tag[1:]
    url = f'{RELEASES_URL}/tag/{tag}'
    if release['html_url'] != url:
        raise ValueError('Unexpected release URL')
    published_at = release['published_at']
    if datetime.fromisoformat(published_at.replace('Z', '+00:00')).tzinfo is None:
        raise ValueError('Release publication time must include a timezone')
    assets = {asset['name'] for asset in release['assets'] if asset.get('size', 0) > 0}
    required = {
        f'vclogg-{version}-windows-x86_64-portable.zip',
        f'vclogg-{version}-windows-x86_64-setup.exe',
        f'vclogg2-{version}-macos-aarch64.dmg',
        f'vclogg2-{version}-linux-x86_64.tar.gz',
    }
    if not required <= assets:
        raise ValueError(f'Release is missing desktop packages: {sorted(required - assets)}')
    manifest = {'schema_version': 1, 'version': version,
                'published_at': published_at, 'download_url': url}
    args.output.write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
    print(f'Generated update manifest for {tag}: {args.output}')


if __name__ == '__main__':
    main()
