# Contributing

[简体中文](../zh-CN/Contributing.md) · [Home / 首页](Home.md)

Keep changes within the ownership boundaries in [architecture](Architecture.md). Update paired English/Chinese documentation when changing behavior, settings, module ownership, scripts or deployment. Keep stable UI IDs when moving or renaming controls.

Use an English Conventional Commit subject, for example `docs(wiki): migrate bilingual documentation`, followed by matching `English:` and `中文:` body sections. Enable the repository template with:

```sh
git config --local commit.template .gitmessage
```

Report checks actually run. `git diff --check` and `bash scripts/check-architecture.sh` are static checks. `scripts/check.sh` compiles analysis and runs Clippy; application builds, tests, benchmarks and real-window acceptance are separate activities. See [building](Building.md) for commands.

Do not commit runtime databases, cloud session files, private keys or local collaboration notes. Existing compatibility patches are documented in [technology](Technology-Stack.md); use upstream public APIs for ordinary application changes.
