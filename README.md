# VCLogg2

A fast, native desktop log viewer for large files, built with Rust and GPUI.

[![Latest release](https://img.shields.io/github/v/release/zhaiyanqi/vclogg2?label=version)](https://github.com/zhaiyanqi/vclogg2/releases/latest)
[![Release build](https://github.com/zhaiyanqi/vclogg2/actions/workflows/release-build.yml/badge.svg)](https://github.com/zhaiyanqi/vclogg2/actions/workflows/release-build.yml)
[![CI (manual)](https://img.shields.io/github/actions/workflow/status/zhaiyanqi/vclogg2/ci.yml?branch=main&label=CI%20%28manual%29)](https://github.com/zhaiyanqi/vclogg2/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/github/license/zhaiyanqi/vclogg2)](LICENSE)

[Website](https://zhaiyanqi.github.io/vclogg2/) · [Download](https://github.com/zhaiyanqi/vclogg2/releases/latest) · [Documentation](doc/README.md) · [简体中文](doc/README.zh-CN.md)

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

See [installation and usage](doc/usage.md#快速开始) for data locations and [delivery notes](doc/delivery.md) for signing details. Current macOS packages use ad-hoc signing without notarization.

## Build from source

<details>
<summary>Prerequisites and build commands</summary>

- All platforms: Rust stable and Git. Initial resolution of GPUI and gpui-component dependencies requires access to GitHub.
- Windows: The MSVC Rust toolchain and Visual Studio 2022 Build Tools with Desktop development with C++ and the Windows SDK.
- macOS: Xcode and Xcode Command Line Tools.
- Linux: Development libraries for Clang, CMake, Fontconfig, Vulkan, Wayland, X11/XCB, and xkbcommon. GitHub Actions uses Ubuntu 22.04.

On Ubuntu/Debian, install the native dependencies used by Actions:

```bash
sudo apt-get update
sudo apt-get install --yes --no-install-recommends \
  build-essential clang cmake libfontconfig-dev libglib2.0-dev libssl-dev \
  libvulkan1 libwayland-dev libx11-dev libx11-xcb-dev libxcb1-dev \
  libxkbcommon-x11-dev pkg-config
```

Run these commands from the repository root, including when following the Chinese README in `doc/`.

| Task | Windows | macOS / Linux |
| --- | --- | --- |
| Run Debug | `powershell -ExecutionPolicy Bypass -File scripts/run-debug.ps1` | Run `./scripts/build-debug.sh`, then `./target/debug/vclogg2` |
| Build Debug | `powershell -ExecutionPolicy Bypass -File scripts/build-debug.ps1` | `./scripts/build-debug.sh` |
| Build Release | `powershell -ExecutionPolicy Bypass -File scripts/build-release.ps1` | `./scripts/build-release.sh` |
| Static checks | `powershell -ExecutionPolicy Bypass -File scripts/check.ps1` | `./scripts/check.sh` |

Release executables are written to `target\release\vclogg2.exe` on Windows and `target/release/vclogg2` on macOS/Linux. Scripts on all platforms use the same locked `Cargo.lock`.

</details>

## Documentation & contributing

[User guide](doc/usage.md) · [Implementation status](doc/migration-status.md) · [Architecture](doc/architecture.md) · [Build & release](doc/delivery.md) · [All documents](doc/README.md)

Detailed documentation is currently in Chinese. Issues and pull requests are welcome.

## Acknowledgments

Thanks to [klogg](https://github.com/variar/klogg) for inspiring work on high-performance log browsing and search, and to the Rust, [GPUI](https://github.com/zed-industries/zed), and [GPUI Component](https://github.com/longbridge/gpui-component) communities.

## License

[Apache License 2.0](LICENSE).
