#!/usr/bin/env python3
"""Validate bilingual sources and build the combined website/Pages artifact."""
import argparse
import hashlib
from html.parser import HTMLParser
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
WIKI = ROOT / 'wiki'
LANGUAGES = ('en', 'zh-CN')
VERSION = 'mdbook v0.4.52'
MARKDOWN_LINK = re.compile(r'\[[^\]]*\]\(([^\s)]+)(?:\s+"[^"]*")?\)')


def check_sources():
    chapters = []
    for language in LANGUAGES:
        directory = WIKI / language
        files = {p.name for p in directory.glob('*.md')} - {'SUMMARY.md'}
        summary = (directory / 'SUMMARY.md').read_text(encoding='utf-8')
        entries = MARKDOWN_LINK.findall(summary)
        if len(entries) != len(set(entries)) or set(entries) != files:
            raise ValueError(f'{language}: SUMMARY must list each chapter exactly once')
        chapters.append(entries)
        for name in files:
            page = directory / name
            content = page.read_text(encoding='utf-8')
            other = LANGUAGES[1] if language == LANGUAGES[0] else LANGUAGES[0]
            if f'../{other}/{name}' not in content:
                raise ValueError(f'{page}: missing paired language link')
            # Ignore examples; only authored prose links must resolve as sources.
            prose = re.sub(r'```.*?```', '', content, flags=re.S)
            for link in MARKDOWN_LINK.findall(prose):
                url = urlsplit(link)
                if url.scheme or url.netloc or not url.path:
                    continue
                target = (page.parent / unquote(url.path)).resolve()
                if not target.is_file():
                    raise ValueError(f'{page}: missing link target {link}')
    if chapters[0] != chapters[1]:
        raise ValueError('English and Chinese chapter order must match')
    manifest = json.loads((WIKI / 'archive/manifest.json').read_text(encoding='utf-8'))
    archive = WIKI / 'archive/github-wiki'
    actual = {str(p.relative_to(archive)) for p in archive.rglob('*') if p.is_file()}
    if actual != set(manifest['sha256']):
        raise ValueError('Archive file inventory differs from migration manifest')
    for name, digest in manifest['sha256'].items():
        if hashlib.sha256((archive / name).read_bytes()).hexdigest() != digest:
            raise ValueError(f'Archived original modified: {name}')
    if any(WIKI.rglob('.git')):
        raise ValueError('wiki/ must not contain a nested Git repository')
    print(f'Source checks passed: {len(chapters[0])} chapters per language; archive intact.')


class Links(HTMLParser):
    def __init__(self):
        super().__init__()
        self.links = []
        self.ids = set()

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if 'id' in attrs:
            self.ids.add(attrs['id'])
        for key in ('href', 'src'):
            if key in attrs:
                self.links.append(attrs[key])


def check_output(site):
    documents = {}
    for page in site.rglob('*.html'):
        parser = Links()
        parser.feed(page.read_text(encoding='utf-8'))
        documents[page.resolve()] = parser
    for page, parser in documents.items():
        for link in parser.links:
            url = urlsplit(link)
            if url.scheme or url.netloc:
                continue
            path = unquote(url.path)
            if path.startswith('/vclogg2/'):
                target = site / path[len('/vclogg2/'):]
            elif path.startswith('/'):
                target = site / path.lstrip('/')
            else:
                target = page.parent / path if path else page
            if target.is_dir():
                target = target / 'index.html'
            target = target.resolve()
            if not target.is_relative_to(site.resolve()) or not target.is_file():
                raise ValueError(f'{page.relative_to(site)}: broken output link {link}')
            if url.fragment and target in documents:
                if unquote(url.fragment) not in documents[target].ids:
                    raise ValueError(f'{page.relative_to(site)}: missing anchor {link}')
    print(f'Output checks passed: {len(documents)} HTML pages and their local links/anchors.')


def build():
    executable = os.environ.get('MDBOOK', 'mdbook')
    version = subprocess.check_output([executable, '--version'], text=True).strip()
    if version != VERSION:
        raise ValueError(f'Expected {VERSION}, found {version}')
    # Build in a disposable directory; keep the previous output on failure.
    with tempfile.TemporaryDirectory(prefix='vclogg-docs-') as temporary:
        stage = Path(temporary) / 'site'
        shutil.copytree(ROOT / 'doc', stage)
        for language in LANGUAGES:
            subprocess.run([executable, 'build', str(WIKI / language),
                            '--dest-dir', str(stage / 'wiki' / language)], check=True)
        (stage / 'wiki/index.html').write_text('''<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>VCLogg2 documentation / 文档</title>
<style>body{font:1.1rem system-ui;max-width:44rem;margin:12vh auto;padding:2rem;line-height:1.8;color:#193446;background:#f6f9fb}a{color:#006b79}nav{display:flex;gap:2rem}</style></head>
<body><main><h1>VCLogg2 documentation / 文档</h1><nav aria-label="Language / 语言">
<a href="en/" lang="en">English</a><a href="zh-CN/" lang="zh-CN">简体中文</a></nav>
<p><a href="../">Website / 官网</a></p></main></body></html>
''', encoding='utf-8')
        (stage / '.nojekyll').touch()
        check_output(stage)
        destination = ROOT / '_site'
        if destination.is_symlink():
            raise ValueError('Refusing to replace a symlink at _site')
        if destination.exists():
            shutil.rmtree(destination)
        shutil.copytree(stage, destination)
    print(f'Built website and bilingual documentation: {destination}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='validate sources without building')
    args = parser.parse_args()
    check_sources()
    if not args.check:
        build()


if __name__ == '__main__':
    main()
