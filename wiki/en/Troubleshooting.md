# Troubleshooting

[简体中文](../zh-CN/Troubleshooting.md) · [Home / 首页](Home.md)

| Symptom | Check |
| --- | --- |
| File looks incorrectly decoded | Re-detect/select encoding and reload; unsupported encodings may prevent editing |
| New log lines are not shown | Focus the file's window/tab; monitoring is active-document only. F5 performs a full rebuild |
| Old lines remain after concurrent rewrite/append | Head/tail refresh may miss a middle rewrite; use F5 |
| Results appear incomplete | Check per-file result limit, selected global files, directory options, source-line bounds and result mode |
| `a|b` behaves differently in quick find | Main literal search treats `|` as alternatives; literal quick find does not |
| Settings changes disappeared | Uncommitted previews revert on cancel/close; save with Primary+S |
| File edit is refused | Check 100 MiB entry limit, valid/supported encoding and disk changes since editing began |
| Cloud registration fails | Check server URL/TLS and desktop/server authentication compatibility; same-repository builds do not guarantee compatibility |
| Windows startup reports a data-path error | Inspect `vclogg2-data-dir.txt` and directory permissions; there is deliberately no silent fallback |
| History cannot be deleted | Open, pinned or marked/annotated records are protected |
| “Check for updates” opens a browser | A newer version or failed check opens Releases; cached failures back off up to 60 minutes. No automatic installation |

For a bug report include version, platform, reproduction steps, file size/encoding and a sanitized minimal sample if possible. Export application logs through Settings → Advanced when relevant; review logs and crash reports before sharing. Do not attach cloud session JSON or unreviewed SQLite state.

Build problems: use the platform prerequisites in [Building](Building.md) and distinguish format checks, compilation, tests, packaging and actual graphics-runtime validation.
