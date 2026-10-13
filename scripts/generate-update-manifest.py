#!/usr/bin/env python3
"""Build a bounded stable-release history from paginated GitHub Release JSON."""
import argparse
from datetime import datetime
import json
from pathlib import Path
import re

RELEASES_URL = 'https://github.com/zhaiyanqi/vclogg2/releases'
STABLE_TAG = re.compile(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?')
IMPORTANT = re.compile(r'breaking[ -]changes?|incompatib\w*|migrat\w*|不兼容|破坏性|迁移', re.I)


def important_notes(body):
    """Keep marked sections and complete paragraphs/list blocks, without summarizing."""
    selected = []
    section_level = None
    fenced = False
    for block in re.split(r'\n\s*\n', body):
        heading = re.match(r'^(#{1,6})\s+(.+)', block)
        if heading and not fenced:
            level = len(heading[1])
            if section_level is not None and level <= section_level:
                section_level = None
            if IMPORTANT.search(heading[2]):
                section_level = level
        if section_level is not None or IMPORTANT.search(block):
            selected.append(block)
        if len(re.findall(r'^\s*(?:```|~~~)', block, re.M)) % 2:
            fenced = not fenced
    return '\n\n'.join(selected)


def release_record(release):
    tag = release['tag_name']
    version = tag[1:]
    url = f'{RELEASES_URL}/tag/{tag}'
    if release['html_url'] != url:
        raise ValueError('Unexpected release URL')
    published_at = release['published_at']
    if datetime.fromisoformat(published_at.replace('Z', '+00:00')).tzinfo is None:
        raise ValueError('Release publication time must include a timezone')
    body = release.get('body') or ''
    return {'version': version, 'published_at': published_at,
            'download_url': url, 'release_notes': body,
            'important_notes': important_notes(body)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('release', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    data = json.loads(args.release.read_text(encoding='utf-8'))
    # gh api --paginate --slurp returns an array of pages. A single release
    # remains supported for local inspection, but cannot claim complete history.
    complete = isinstance(data, list)
    releases = ([r for page in data for r in (page if isinstance(page, list) else [page])]
                if complete else [data])
    stable = [r for r in releases if not r['draft'] and not r['prerelease']
              and STABLE_TAG.fullmatch(r['tag_name'])]
    stable.sort(key=lambda r: tuple(map(int, STABLE_TAG.fullmatch(r['tag_name']).groups())), reverse=True)
    if not stable:
        raise ValueError('No published stable release exists')
    versions = [tuple(map(int, STABLE_TAG.fullmatch(r['tag_name']).groups())) for r in stable]
    if len(versions) != len(set(versions)):
        raise ValueError('Duplicate stable SemVer precedence')
    latest = stable[0]
    version = latest['tag_name'][1:]
    assets = {a['name'] for a in latest['assets'] if a.get('size', 0) > 0}
    required = {f'vclogg-{version}-windows-x86_64-portable.zip',
                f'vclogg-{version}-windows-x86_64-setup.exe',
                f'vclogg2-{version}-macos-aarch64.dmg',
                f'vclogg2-{version}-linux-x86_64.tar.gz'}
    if not required <= assets:
        raise ValueError(f'Release is missing desktop packages: {sorted(required - assets)}')
    records = [release_record(r) for r in stable[:30]]
    # Keep the original top-level fields for clients that only know one release.
    manifest = {'schema_version': 1, **records[0], 'releases': records,
                'history_complete': complete and len(stable) <= 30}
    payload = json.dumps(manifest, indent=2, ensure_ascii=False) + '\n'
    if len(payload.encode('utf-8')) > 8 * 1024 * 1024:
        raise ValueError('Update manifest exceeds the client 8 MiB limit')
    args.output.write_text(payload, encoding='utf-8')
    print(f'Generated update manifest: latest {version}, {len(records)} stable releases')


if __name__ == '__main__':
    main()
