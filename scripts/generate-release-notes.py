#!/usr/bin/env python3
"""Generate bilingual release notes from EN: and ZH-CN: commit sections."""

import argparse
import json
import re
import subprocess
from pathlib import Path


def git(*args):
    return subprocess.check_output(
        ["git", *args], encoding="utf-8", errors="replace"
    )


def translations(message):
    sections = {"EN": [], "ZH-CN": []}
    language = None
    for line in message.splitlines():
        marker = re.fullmatch(r"(EN|ZH-CN):\s*(.*)", line, re.IGNORECASE)
        if marker:
            language = marker[1].upper()
            if marker[2]:
                sections[language].append(marker[2])
        elif language:
            sections[language].append(line)
    return {key: "\n".join(lines).strip() for key, lines in sections.items()}


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

    entries = {"EN": [], "ZH-CN": []}
    for index in range(0, len(fields), 3):
        commit, short_commit, message = fields[index:index + 3]
        sections = translations(message)
        for language in sections:
            sections[language] = sections[language] or catalog.get(commit, {}).get(language, "")
        subject = message.strip().splitlines()[0] if message.strip() else short_commit
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
            lines = content.splitlines()
            entry = f"- {lines[0]} ({link})"
            if len(lines) > 1:
                entry += "\n" + "\n".join(f"  {line}" for line in lines[1:])
            entries[language].append(entry)

    output = []
    for language, heading, empty in (
        ("EN", "Changelog (English)", "No commits in this release."),
        ("ZH-CN", "更新记录（简体中文）", "本次发布没有新增提交。"),
    ):
        output.append(f"## {heading}\n\n" + ("\n".join(entries[language]) or empty))
    if previous_tag:
        output.append(
            f"**Full Changelog / 完整变更记录**: "
            f"https://github.com/{args.repository}/compare/{previous_tag}...{args.tag}"
        )
    args.output.write_text("\n\n".join(output) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
