# VCLogg2

VCLogg2 (VCLogg) is a free, open-source desktop log viewer for large files on Windows, macOS and Linux, built with Rust and GPUI.

Open log files produced by your application's logger, search across files with keywords or regex, follow live updates and export matching lines.

[![Latest release](https://img.shields.io/github/v/release/zhaiyanqi/vclogg2?label=version)](https://github.com/zhaiyanqi/vclogg2/releases/latest)
[![Release build](https://github.com/zhaiyanqi/vclogg2/actions/workflows/release-build.yml/badge.svg)](https://github.com/zhaiyanqi/vclogg2/actions/workflows/release-build.yml)
[![CI (manual)](https://img.shields.io/github/actions/workflow/status/zhaiyanqi/vclogg2/ci.yml?branch=main&label=CI%20%28manual%29)](https://github.com/zhaiyanqi/vclogg2/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/github/license/zhaiyanqi/vclogg2)](LICENSE)

[Website](https://zhaiyanqi.github.io/vclogg2/) · [Download](https://github.com/zhaiyanqi/vclogg2/releases/latest) · [Wiki](wiki/en/Home.md) · [简体中文](README.zh-CN.md)

![VCLogg2: Clear signals in large logs](doc/assets/readme-hero.jpg)

## Features

- **Large files** — Background indexing, on-demand decoding, and virtualized scrolling.
- **Flexible search** — Keywords, regex, and quick find across the current file, open tabs, or a directory.
- **Live logs** — Refresh growing logs and follow the latest lines.
- **Log analysis** — Bookmarks, color labels, grouped results, and streaming export.
- **Native workspace** — Multiple tabs and windows, automatic encoding detection, and session recovery.

## Download

Get the [latest release](https://github.com/zhaiyanqi/vclogg2/releases/latest) for your platform:

| Platform | Package | Install |
| --- | --- | --- |
| Windows 10/11 · x64 | Portable ZIP / Setup EXE | Extract and run `vclogg2.exe`, or run the installer |
| macOS 15 · Apple Silicon | DMG | Drag `VCLogg2.app` into Applications |
| Linux · x86_64 (Ubuntu 22.04) | tar.gz | Extract and run `./Install-VCLogg2-linux.sh --launch` |

See [installation](wiki/en/Installation.md) for data locations and [build and release notes](wiki/en/Build-and-Release.md) for signing details. Current macOS packages use ad-hoc signing without notarization.

## Build from source

See the [source build guide](wiki/en/Building.md) for Rust and platform prerequisites, environment setup, Debug/Release commands, and validation. Build and packaging scripts are maintained in `scripts/` and use the committed `Cargo.lock`.

The optional cloud filter server lives in [`server/`](server/README.md). Its Windows/Linux packages are included in tagged releases; local packaging uses `scripts/package-server-release.sh` or `scripts/package-server-release.ps1`.

## Documentation & contributing

Detailed documentation is maintained in the main repository’s [wiki/](wiki/README.md), in English and Simplified Chinese, and published with the website using mdBook.

- [Quick start](wiki/en/Getting-Started.md) · [Interface](wiki/en/Interface.md) · [Keyboard shortcuts](wiki/en/Keyboard-Shortcuts.md)
- [Search & filters](wiki/en/Search-and-Filters.md) · [Live logs](wiki/en/Live-Logs.md) · [Settings](wiki/en/Settings.md)
- [Project overview](wiki/en/Project-Overview.md) · [Technology stack](wiki/en/Technology-Stack.md) · [Architecture](wiki/en/Architecture.md)
- [Contributing](wiki/en/Contributing.md) · [Troubleshooting](wiki/en/Troubleshooting.md) · [All documents](wiki/en/Home.md)

Issues and pull requests are welcome.

Commits use an English Conventional Commit title and matching `English:` / `中文:` body sections. Enable the [commit template](.gitmessage) with `git config --local commit.template .gitmessage`. Release notes group changes by type within separate English and Simplified Chinese sections.

## Acknowledgments

Thanks to [klogg](https://github.com/variar/klogg) for inspiring work on high-performance log browsing and search, and to the Rust and [GPUI Kit](https://gpui-kit.com) communities.

## License

[Apache License 2.0](LICENSE).
