#!/usr/bin/env python3
"""Generate categorized bilingual release notes from commit messages."""

import argparse
import json
import re
import subprocess
from pathlib import Path


CONVENTIONAL = re.compile(
    r"^(?P<type>[a-z]+)(?:\((?P<scope>[^)]+)\))?(?P<breaking>!)?:\s+(?P<summary>.+)$",
    re.IGNORECASE,
)
LANGUAGE_MARKER = re.compile(r"(English|中文|EN|ZH-CN):\s*(.*)", re.IGNORECASE)
FOOTER = re.compile(
    r"^(?:BREAKING[ -]CHANGE|Refs|Closes|Fixes|Resolves|[A-Za-z-]+-by)(?:: | #)"
)
CATEGORIES = (
    ("breaking", "⚠️ Breaking Changes", "⚠️ 不兼容变更"),
    ("feat", "✨ Features", "✨ 新增功能"),
    ("fix", "🐛 Bug Fixes", "🐛 问题修复"),
    ("perf", "⚡ Performance", "⚡ 性能优化"),
    ("refactor", "♻️ Refactoring", "♻️ 代码重构"),
    ("docs", "📝 Documentation", "📝 文档更新"),
    ("maintenance", "🔧 Maintenance", "🔧 工程维护"),
    ("other", "📦 Other Changes", "📦 其他变更"),
)


def git(*args):
    return subprocess.check_output(
        ["git", *args], encoding="utf-8", errors="replace"
    )


def translations(message):
    sections = {"EN": [], "ZH-CN": []}
    language = None
    for line in message.splitlines()[1:]:
        marker = LANGUAGE_MARKER.fullmatch(line)
        if marker:
            language = "EN" if marker[1].upper() in ("EN", "ENGLISH") else "ZH-CN"
            if marker[2]:
                sections[language].append(marker[2])
        elif FOOTER.match(line):
            language = None
        elif language:
            sections[language].append(line)
    return {key: "\n".join(lines).strip() for key, lines in sections.items()}


def category_for(subject, message):
    conventional = CONVENTIONAL.fullmatch(subject)
    if (conventional and conventional["breaking"]) or re.search(
        r"^BREAKING[ -]CHANGE: ", message, re.MULTILINE
    ):
        return "breaking"
    kind = conventional["type"].lower() if conventional else "other"
    if kind in ("build", "ci", "chore", "style", "test"):
        return "maintenance"
    if kind == "revert":
        return "fix"
    return kind if kind in {category[0] for category in CATEGORIES} else "other"


def render_entry(content, subject, link):
    lines = content.splitlines()
    first = re.sub(r"^[-*] ", "", lines[0])
    summary = CONVENTIONAL.fullmatch(first)
    if summary:
        first = summary["summary"]
    conventional = CONVENTIONAL.fullmatch(subject)
    scope = f"**{conventional['scope']}:** " if conventional and conventional["scope"] else ""
    entry = f"- {scope}{first} ({link})"
    if len(lines) > 1:
        entry += "\n" + "\n".join(f"  {line}" for line in lines[1:])
    return entry


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tag")
    parser.add_argument("repository", help="GitHub owner/repository")
    parser.add_argument("output", type=Path)
    parser.add_argument(
        "--translations", type=Path,
        default=Path(__file__).with_name("commit-translations.json"),
        help="Reviewed translations keyed by full commit SHA",
    )
    parser.add_argument(
        "--previous-tag",
        help="Explicit previous release tag; an empty value includes all history",
    )
    parser.add_argument(
        "--require-translations", action="store_true",
        help="Fail instead of emitting untranslated entries",
    )
    args = parser.parse_args()
    catalog = json.loads(args.translations.read_text(encoding="utf-8"))

    # Resolve the tag first so an invalid reference cannot silently produce notes.
    revision = git("rev-parse", "--verify", f"refs/tags/{args.tag}^{{commit}}").strip()
    previous_tag = args.previous_tag
    if previous_tag is None:
        previous = subprocess.run(
            ["git", "describe", "--tags", "--match", "v*", "--match", "[0-9]*",
             "--abbrev=0", f"{revision}^"],
            capture_output=True, text=True, check=False,
        )
        previous_tag = previous.stdout.strip() if previous.returncode == 0 else ""
    commit_range = revision
    if previous_tag:
        previous_revision = git(
            "rev-parse", "--verify", f"refs/tags/{previous_tag}^{{commit}}"
        ).strip()
        subprocess.run(
            ["git", "merge-base", "--is-ancestor", previous_revision, revision], check=True
        )
        commit_range = f"{previous_revision}..{revision}"
    fields = git("log", "-z", "--format=%H%x00%h%x00%B", commit_range, "--").split("\0")
    if fields[-1] == "":
        fields.pop()
    if len(fields) % 3:
        raise ValueError("Unexpected git log record format")

    entries = {language: {key: [] for key, _, _ in CATEGORIES} for language in ("EN", "ZH-CN")}
    for index in range(0, len(fields), 3):
        commit, short_commit, message = fields[index:index + 3]
        sections = translations(message)
        for language in sections:
            sections[language] = sections[language] or catalog.get(commit, {}).get(language, "")
        subject = message.strip().splitlines()[0] if message.strip() else short_commit
        category = category_for(subject, message)
        link = f"[{short_commit}](https://github.com/{args.repository}/commit/{commit})"
        for language, missing in (
            ("EN", "English translation unavailable; original message"),
            ("ZH-CN", "缺少中文翻译，保留原文"),
        ):
            content = sections[language]
            if not content:
                if args.require_translations:
                    raise ValueError(f"Missing {language} translation for {commit}")
                original = sections["EN"] or sections["ZH-CN"] or subject
                content = f"{missing}: {original}"
            entries[language][category].append(render_entry(content, subject, link))

    output = ["[English](#english) | [简体中文](#简体中文)"]
    for language, heading, empty, heading_index in (
        ("EN", "English", "No commits in this release.", 1),
        ("ZH-CN", "简体中文", "本次发布没有新增提交。", 2),
    ):
        sections = [f"## {heading}"]
        for category in CATEGORIES:
            items = entries[language][category[0]]
            if items:
                sections.append(f"### {category[heading_index]}\n\n" + "\n".join(items))
        if len(sections) == 1:
            sections.append(empty)
        if language == "ZH-CN":
            output.append("---")
        output.append("\n\n".join(sections))
    if previous_tag:
        output.append(
            f"---\n\n[Full Changelog / 完整变更记录]"
            f"(https://github.com/{args.repository}/compare/{previous_tag}...{args.tag})"
        )
    args.output.write_text("\n\n".join(output) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
