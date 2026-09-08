# VCLogg2

使用 Rust 与 GPUI 构建的高性能原生桌面日志查看器，专为大文件设计。

[![Latest release](https://img.shields.io/github/v/release/zhaiyanqi/vclogg2?label=version)](https://github.com/zhaiyanqi/vclogg2/releases/latest)
[![Release build](https://github.com/zhaiyanqi/vclogg2/actions/workflows/release-build.yml/badge.svg)](https://github.com/zhaiyanqi/vclogg2/actions/workflows/release-build.yml)
[![CI (manual)](https://img.shields.io/github/actions/workflow/status/zhaiyanqi/vclogg2/ci.yml?branch=main&label=CI%20%28manual%29)](https://github.com/zhaiyanqi/vclogg2/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/github/license/zhaiyanqi/vclogg2)](../LICENSE)

[项目主页](https://zhaiyanqi.github.io/vclogg2/) · [下载](https://github.com/zhaiyanqi/vclogg2/releases/latest) · [文档](README.md) · [English](../README.md)

![VCLogg2：海量日志，信号清晰](assets/readme-hero-zh-CN.jpg)

## 特性

- **大文件浏览** — 后台索引、按需解码和虚拟化滚动。
- **灵活检索** — 多关键词、正则与页内查找，支持当前文件、已打开标签和目录范围。
- **实时日志** — 自动刷新持续增长的日志，支持末尾跟随。
- **日志分析** — 行标记、颜色标签、结果分组与流式导出。
- **原生工作区** — 多标签、多窗口、自动编码检测与会话恢复。

## 下载

从[最新版本](https://github.com/zhaiyanqi/vclogg2/releases/latest)下载对应平台的安装包：

| 平台 | 格式 | 安装方式 |
| --- | --- | --- |
| Windows 10/11 · x64 | 便携 ZIP / 安装 EXE | 解压后运行 `vclogg2.exe`，或运行安装向导 |
| macOS 15 · Apple Silicon | DMG | 将 `VCLogg2.app` 拖入 Applications |
| Linux · x86_64（Ubuntu 22.04） | tar.gz | 解压后运行 `./Install-VCLogg2-linux.sh --launch` |

数据目录见[安装与使用](usage.md#快速开始)，签名说明见[交付文档](delivery.md)。当前 macOS 包使用临时签名，尚未公证。

## 从源码构建

<details>
<summary>环境要求与构建命令</summary>

- 所有平台：Rust stable、Git；首次解析 GPUI 与 gpui-component 依赖时需要访问 GitHub。
- Windows：MSVC Rust 工具链，以及包含“使用 C++ 的桌面开发”和 Windows SDK 的 Visual Studio 2022 Build Tools。
- macOS：Xcode 与 Xcode Command Line Tools。
- Linux：Clang、CMake、Fontconfig、Vulkan、Wayland、X11/XCB 与 xkbcommon 开发库；GitHub Actions 使用 Ubuntu 22.04。

Ubuntu/Debian 可安装与 Actions 一致的原生依赖：

```bash
sudo apt-get update
sudo apt-get install --yes --no-install-recommends \
  build-essential clang cmake libfontconfig-dev libglib2.0-dev libssl-dev \
  libvulkan1 libwayland-dev libx11-dev libx11-xcb-dev libxcb1-dev \
  libxkbcommon-x11-dev pkg-config
```

以下命令均在仓库根目录执行，即使正在阅读 `doc/` 下的中文版 README。

| 任务 | Windows | macOS / Linux |
| --- | --- | --- |
| 启动 Debug | `powershell -ExecutionPolicy Bypass -File scripts/run-debug.ps1` | `./scripts/build-debug.sh` 后运行 `./target/debug/vclogg2` |
| 构建 Debug | `powershell -ExecutionPolicy Bypass -File scripts/build-debug.ps1` | `./scripts/build-debug.sh` |
| 构建 Release | `powershell -ExecutionPolicy Bypass -File scripts/build-release.ps1` | `./scripts/build-release.sh` |
| 静态检查 | `powershell -ExecutionPolicy Bypass -File scripts/check.ps1` | `./scripts/check.sh` |

Release 可执行文件位于 Windows 的 `target\release\vclogg2.exe` 或 macOS/Linux 的 `target/release/vclogg2`。三平台脚本使用同一份锁定的 `Cargo.lock`。

</details>

## 文档与贡献

[使用指南](usage.md) · [功能状态](migration-status.md) · [架构](architecture.md) · [构建与发布](delivery.md) · [全部文档](README.md)

欢迎提交 Issue 和 Pull Request。

## 致谢

参见[英文 README](../README.md#acknowledgments)。

## 许可证

[Apache License 2.0](../LICENSE)。
