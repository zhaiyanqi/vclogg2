# VCLogg2

使用 Rust、GPUI 与 GPUI Component 构建的跨平台桌面日志查看器。

VCLogg2 面向大文件浏览、持续追加日志、多范围检索和分析型工作流：后台建立行索引，只解码当前可见窗口，并把搜索、标记、颜色标签、视口和会话恢复组织在同一个原生工作区中。

[项目主页](https://zhaiyanqi.github.io/vclogg2/) · [下载最新版本](https://github.com/zhaiyanqi/vclogg2/releases) · [问题反馈](https://github.com/zhaiyanqi/vclogg2/issues) · [项目文档](doc/README.md)

![VCLogg2：海量日志，信号清晰](docs/assets/readme-hero.jpg)

## 特性

- **大文件优先**：后台建立行起点索引，按需解码可见行，并通过虚拟化表格限制内存和渲染开销。
- **快速可用**：打开文件时先显示有界安全预览，再原子换入完整索引；支持持久索引缓存和纯追加日志的增量尾部索引。
- **灵活检索**：支持多关键词、大小写匹配、字节正则、页内查找，以及当前标签、多个已打开标签和目录三种范围。
- **分析工作流**：提供压缩多选、文字选择、行标记、颜色标签、日志级别着色、结果分组、流式导出和按时间戳合并。
- **原生工作区**：支持多标签、多窗口、文件拖放、跨窗口标签移动或复制，以及三平台的单进程启动请求接管。
- **可靠恢复**：使用 SQLite/WAL 保存最近文件、收藏、查询、标记、视口、标签顺序和呈现偏好。
- **编码一致性**：自动检测 UTF-8、UTF-16、常见传统编码和二进制文件；显示、搜索、高亮与导出共享同一解码快照。
- **跨平台交付**：Windows 10/11 x64、macOS 15 ARM64 与 Ubuntu 22.04 x86_64 共享同一套产品能力，平台差异仅限安装格式、快捷键修饰键和系统运行库。

功能实现边界以[功能实现状态](doc/migration-status.md)为准。

## 工作方式

| 能力 | 行为 |
| --- | --- |
| 多关键词 | 使用 `|` 分隔普通关键词，例如 `error|timeout|retry` |
| 正则与大小写 | 可在搜索栏切换；无效正则不会替换上一份有效结果 |
| 搜索范围 | 当前标签、选定的已打开标签或目录；跨文件搜索并发执行 |
| 结果模式 | 标记与匹配、仅匹配、仅标记；空查询可用于只查看标记 |
| 导航与恢复 | 结果重组后按稳定文件与源行恢复选择和视口，失效锚点回退到最近结果 |
| 结果导出 | 流式导出当前或跨文件结果，支持分组输出和按日志时间戳稳定合并 |

长时间搜索会显示扫描行数、匹配数和进度，并可取消。新的搜索、重新加载或关闭文档会使旧扫描失效，迟到结果不会覆盖当前视图。

仅在窗口处于前台时监测当前查看的日志，通过系统通知刷新正文、当前文件搜索及已有全局搜索中该文件的结果；切到后台停止监听和轮询，回到前台或切换标签立即检查一次。连续通知按 400 ms 合并，每 30 秒只复核当前文件。“末尾跟随”仅控制正文是否滚到最新一行。持续追加、半行续写、截断和轮转的处理方式及刷新延迟边界见[动态日志文件](doc/dynamic-log-files.md)。

## 快速开始

### Windows

下载已签名的 `vclogg2-<version>-windows-x86_64.zip`，或文件名带 `unsigned` 的未签名版本，解压后直接运行：

```powershell
.\vclogg2.exe
```

Windows 包是纯便携包，不创建开始菜单快捷方式或文件关联。应用不会自动下载或安装更新。

### macOS

下载 `vclogg2-<version>-macos-aarch64.dmg`，打开镜像后将 `VCLogg2.app` 拖入 `Applications`。当前 Actions 产物使用临时签名，未使用 Apple Developer ID 签名或公证，首次打开时系统可能要求确认。

### Linux

下载 `vclogg2-<version>-linux-x86_64.tar.gz` 后执行：

```bash
tar -xzf vclogg2-<version>-linux-x86_64.tar.gz
cd vclogg2-<version>-linux-x86_64
./Install-VCLogg2-linux.sh --launch
```

安装器默认写入 `~/.local/lib/vclogg2`，创建 `~/.local/bin/vclogg2` 入口，并注册桌面应用和支持的 MIME 类型。

### 从命令行打开日志

```text
vclogg2 <service.log> <worker.trace>
```

如果 VCLogg2 已在运行，路径会按参数顺序交给现有进程，并在最近激活的窗口中打开。

## 架构

VCLogg2 使用 `core / data / app` 三层 Rust workspace。应用层可以组合领域核心与数据层；领域核心和数据层不依赖 GPUI，也不反向依赖应用层。

| Crate | 职责 |
| --- | --- |
| `vclogg-core` | 文件快照、行索引、解码、搜索、压缩结果集合与取消 |
| `vclogg-data` | SQLite 持久化、路径编码、索引缓存生命周期与恢复记录 |
| `vclogg-app` / `vclogg2` | GPUI 应用外壳、窗口与标签、虚拟日志视图、交互和后台任务编排 |

> 文件与搜索逻辑属于 core，持久化与缓存生命周期属于 data，界面呈现与交互属于 app。

完整的依赖方向、状态所有权和可执行架构守卫见[架构说明](doc/architecture.md)。

## 从源码构建

### 环境要求

- 所有平台：Rust stable、Git；首次解析 GPUI 与 gpui-component 依赖时需要访问 GitHub。
- Windows：MSVC Rust 工具链，以及包含“使用 C++ 的桌面开发”和 Windows SDK 的 Visual Studio 2022 Build Tools。
- macOS：Xcode 与 Xcode Command Line Tools。
- Linux：Clang、CMake、Fontconfig、Vulkan、Wayland、X11/XCB 与 xkbcommon 开发库；GitHub Actions 使用 Ubuntu 22.04。

检查通用工具链：

```text
rustc --version
cargo --version
git --version
```

Ubuntu/Debian 可安装与 Actions 一致的原生依赖：

```bash
sudo apt-get update
sudo apt-get install --yes --no-install-recommends \
  build-essential clang cmake libfontconfig-dev libglib2.0-dev libssl-dev \
  libvulkan1 libwayland-dev libx11-dev libx11-xcb-dev libxcb1-dev \
  libxkbcommon-x11-dev pkg-config
```

### 开发命令

| 任务 | Windows | macOS / Linux |
| --- | --- | --- |
| 启动 Debug | `powershell -ExecutionPolicy Bypass -File scripts/run-debug.ps1` | `./scripts/build-debug.sh` 后运行 `./target/debug/vclogg2` |
| 构建 Debug | `powershell -ExecutionPolicy Bypass -File scripts/build-debug.ps1` | `./scripts/build-debug.sh` |
| 构建 Release | `powershell -ExecutionPolicy Bypass -File scripts/build-release.ps1` | `./scripts/build-release.sh` |
| 静态检查 | `powershell -ExecutionPolicy Bypass -File scripts/check.ps1` | `./scripts/check.sh` |

Release 可执行文件位于 Windows 的 `target\release\vclogg2.exe` 或 macOS/Linux 的 `target/release/vclogg2`。三平台脚本使用同一份锁定的 `Cargo.lock`。

## 性能诊断

使用 Python 3 生成 50 MiB、100 MiB 和 500 MiB 测试日志：

```bash
python3 scripts/generate-test-data.py
```

使用 Python 3 持续追加随机日志，观察文件增长时的行为：

```bash
python3 scripts/generate-live-log.py --output target/test-data/live.log --min-lines 5 --max-lines 30 --interval 1
```

默认每秒追加 1～20 行到 `target/test-data/live.log`，每批写入后刷新，按 Ctrl+C 停止。文件不存在时自动创建，已存在时保留内容并追加。可加 `--total-lines 1000` 在本次追加 1000 行后退出；`--seed 42` 固定随机内容和批次行数，时间戳仍为当前 UTC 时间。日志中的 `line` 从本次运行的 1 开始计数。使用 `--help` 查看全部参数。

Debug 构建默认记录超过 16 ms 的已标记 UI 渲染作用域。需要隔离应用数据、缓存和构建产物时，在 Windows 使用：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/run-performance-debug.ps1 `
  -WarnAfterMilliseconds 8 `
  -RepeatAfterMilliseconds 1000 `
  -Paths D:\logs\service.log,D:\logs\worker.log
```

诊断器能力、受控 A/B 数据和验证边界见[性能交付说明](doc/PERFORMANCE_DELIVERY_2026-08-31.md)。

## 打包与发布

平台打包入口如下；产物分别写入 `dist/windows-x86_64/`、`dist/macos-aarch64/` 和 `dist/linux-x86_64/`：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-release.ps1
```

```bash
./scripts/package-release-macos.sh
./scripts/package-release-linux.sh
```

正式版本由指向当前 `main` 的 `v<SemVer>` 标签触发三平台 GitHub Actions。Windows 未配置签名时生成带 `unsigned` 标识的包；macOS 正式公开分发仍需 Developer ID 签名和公证。签名后端、产物内容、发布脚本与信任边界见[交付说明](doc/delivery.md)。

## 本地数据与隐私

- 源日志始终作为只读输入；只有用户显式导出结果时才写入所选位置。
- Windows 便携版把状态库、索引缓存、崩溃报告和应用临时结果保存在可执行文件同级的 `VCLogg2` 目录；macOS/Linux 使用各自的系统应用数据、缓存和临时目录。
- 会话状态使用 SQLite/WAL；索引缓存与会话身份分离，失效时会安全重建。
- 云端连接的公开配置写入 SQLite，Cookie 与 CSRF 只保存在系统凭据库。
- 应用日志使用有界内存缓冲，不会自行创建长期日志文件。

## 文档

完整索引与维护约定见 [`doc/README.md`](doc/README.md)。

| 分类 | 入口 |
| --- | --- |
| 产品范围与状态 | [功能实现状态](doc/migration-status.md) · [功能与验收记录](doc/feature-parity.md) |
| 架构与界面 | [架构说明](doc/architecture.md) · [UI 布局层级](doc/ui-layout.md) |
| 构建、交付与性能 | [交付说明](doc/delivery.md) · [性能交付说明](doc/PERFORMANCE_DELIVERY_2026-08-31.md) |
| 视觉与手工验收 | [界面优化验收](doc/ui-polish-acceptance.md) · [过滤器 UUID 分支验收](doc/filter-uuid-branch-manual-checklist.md) |

## 参与贡献

欢迎提交问题、改进建议与代码贡献。开始修改前请阅读 [`RULES.md`](RULES.md) 和对应分类文档，并遵守三层职责边界。修改功能范围、UI 层级、构建命令或交付流程时，请同步更新对应文档。

提交前在 Windows 运行 `scripts/check.ps1`，在 macOS/Linux 运行 `scripts/check.sh`，并在变更说明中明确复现路径、预期行为、实际行为和验证范围。

## 鸣谢

感谢 [klogg](https://github.com/variar/klogg) 在高性能日志浏览与检索方面的探索。VCLogg2 在 Rust 与 GPUI 技术栈上继续实践大文件日志浏览、搜索和持续跟随。

同时感谢 Rust、[GPUI](https://github.com/zed-industries/zed) 与 [GPUI Component](https://github.com/longbridge/gpui-component) 社区提供的基础设施和开源成果。

## 许可证

本项目采用 [Apache License 2.0](LICENSE) 开源许可证。
