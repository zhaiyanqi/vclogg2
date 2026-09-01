# VCLogg2

使用 Rust 与 GPUI Component 构建的桌面日志查看器，专注于大文件浏览、实时跟随、多范围检索、筛选与结果导出。

> Windows 10/11 x64、macOS 15 ARM64 与 Ubuntu 22.04 x86_64 是同等支持的桌面平台。三者共享日志浏览、单实例多窗口、系统废纸篓、文件打开集成和应用内更新能力；平台差异仅限原生安装格式、快捷键修饰键与系统运行库。功能实现状态以 [`doc/migration-status.md`](doc/migration-status.md) 为准。

[功能亮点](#功能亮点) · [快速开始](#快速开始) · [搜索与筛选](#搜索与筛选) · [快捷键](#常用快捷键) · [从源码构建](#从源码构建) · [项目文档](#项目文档) · [参与贡献](#参与贡献) · [鸣谢](#鸣谢)

## 功能亮点

- **面向大文件**：后台建立行起点索引，完整文档通过带分块摘要校验的位置读取按需解码可见行，并使用虚拟化表格控制内存占用。
- **先预览、后完整加载**：打开文件时先提供有界安全预览，再原子换入完整索引；支持持久索引缓存、文件重新加载和纯追加场景的增量尾部索引。
- **灵活检索**：支持多关键词、大小写匹配、字节正则、页内查找，以及当前标签、多标签和目录三种搜索范围。
- **原生工作区**：支持多标签、多窗口、外部文件拖放、跨窗口标签移动或复制，以及三平台单进程启动请求接管。
- **可恢复会话**：使用 SQLite/WAL 保存最近文件、收藏、查询、标记、视口、标签顺序和呈现偏好，退出时事务化写入状态。
- **面向分析的交互**：支持行标记、颜色标签、日志级别着色、多选与文字选择、结果分组、流式导出和按时间戳合并结果。
- **编码与二进制查看**：自动检测 UTF-8、UTF-16、常见传统编码和二进制文件；显示、搜索、高亮与导出共享同一解码快照。
- **可配置体验**：提供浅色、深色和跟随系统主题，可调整字体、字号、行距、行号栏、滚动策略、自动换行和常用快捷键。

## 快速开始

### Windows

获取 `vclogg2-<version>-windows-x86_64.zip` 后解压，可直接运行：

```powershell
.\vclogg2.exe
```

Windows 分发包是纯便携包，只包含可执行程序、README 和许可证，不附带 PowerShell 安装或更新脚本，也不会创建开始菜单快捷方式或注册文件关联。应用内更新由可执行程序的原生助手模式完成，不内嵌、生成或调用 PowerShell 脚本。

从仍依赖包内安装脚本的旧版本首次迁移到纯便携包时，需要手动解压并替换旧程序；完成这次迁移后，后续纯便携版本可以继续使用应用内更新。

### macOS

获取 `vclogg2-<version>-macos-aarch64.zip` 后解压，在终端执行安装脚本，或直接打开包内的 `VCLogg2.app`：

```bash
./Install-VCLogg2-macos.sh --launch
```

默认安装到 `~/Applications/VCLogg2.app` 并注册支持的文档类型。Actions 产物使用临时签名，未使用 Apple Developer ID 签名或公证，因此首次打开时 macOS 可能要求在系统安全设置中确认。

### Linux

Linux x86_64 包名为 `vclogg2-<version>-linux-x86_64.tar.gz`：

```bash
tar -xzf vclogg2-<version>-linux-x86_64.tar.gz
cd vclogg2-<version>-linux-x86_64
./Install-VCLogg2-linux.sh --launch
```

默认安装到 `~/.local/lib/vclogg2`，创建 `~/.local/bin/vclogg2` 入口并注册桌面应用与受支持 MIME 类型。Linux 包面向 Ubuntu 22.04 或具有兼容 glibc、Fontconfig、Vulkan、Wayland/X11 运行库的发行版。

### 从命令行打开日志

可向可执行文件传入一个或多个绝对或相对路径：

```text
vclogg2 <service.log> <worker.trace>
```

如果 VCLogg2 已在运行，路径会按参数顺序交给现有进程，并在最近激活的窗口中打开。

### 基本使用流程

1. 使用 Windows/Linux 的 `Ctrl+O`、macOS 的 `Command+O`、命令行参数或文件拖放打开一个或多个日志。
2. 在搜索框输入关键词并按 `Enter`，结果会显示在独立结果区域中。
3. 通过搜索范围菜单切换当前标签、多标签或目录搜索。
4. 使用结果模式组合匹配行与标记行，并按需复制、标记或导出当前结果。

## 搜索与筛选

| 能力 | 说明 |
| --- | --- |
| 多关键词 | 使用 `|` 分隔多个普通关键词，例如 `error|timeout|retry` |
| 区分大小写 | 可在搜索栏、菜单或通过 `Alt+C` 切换 |
| 正则搜索 | 启用正则模式后执行字节正则匹配，无效表达式不会替换上一份有效结果 |
| 搜索范围 | 当前标签、多标签、目录；多标签与目录搜索按文件并发扫描 |
| 结果模式 | 标记与匹配、仅匹配、仅标记；允许空查询用于只查看标记 |
| 页内查找 | `Ctrl+Shift+F` 在正文、当前结果或全局结果内定位可见关键词 |
| 搜索历史与补全 | 从历史查询和预定义过滤器生成候选，只替换当前输入片段 |
| 结果导出 | 流式导出当前或全局结果，支持按文件分组和按日志时间戳稳定合并 |

长时间搜索会显示扫描行数、匹配数和进度，并可协作取消。新的搜索、重新加载或关闭文档会使旧扫描失效，迟到结果不会覆盖当前视图。

## 工作区与会话

- 每个窗口拥有独立的标签、搜索、弹层和通知状态；`Ctrl+Shift+N` 可创建新窗口。
- 标签可在窗口内排序，也可带着查询、标记、视口和呈现状态跨窗口移动或复制。
- 活动文件可启用末尾跟随。文件纯增长时只扫描新增字节；截断、替换或中段改写会安全回退到全量重建。
- 每个标签独立保存选中行、首个可见行、横向滚动、自动换行、结果模式、查询、标记和显示设置。
- 最近文件按收藏优先和打开时间排序；历史清理只删除恢复元数据，不会删除原始日志文件。
- 关闭最后一个窗口时，应用完成会话写入后结束进程，不创建托盘图标或无窗口后台实例。

## 常用快捷键

| 快捷键 | 操作 |
| --- | --- |
| `Ctrl+O` / `Command+O` | 打开一个或多个日志文件 |
| `Ctrl+Shift+N` / `Command+Shift+N` | 创建新窗口 |
| `Ctrl+W` / `Command+W` | 关闭当前标签 |
| `Ctrl+F` / `Command+F` | 聚焦主搜索框 |
| `Ctrl+Shift+F` / `Command+Shift+F` | 打开当前区域的页内查找 |
| `Ctrl+G` / `Command+G` | 转到指定源行 |
| `Ctrl+Home` / `Ctrl+End`、`Command+Home` / `Command+End` | 跳到文件开头 / 末尾 |
| `F5` | 重新加载当前文件 |
| `Alt+C` | 切换主搜索的大小写匹配 |
| `Ctrl+D` / `Command+D` | 为当前文字或所选行轮换颜色标签 |
| `Ctrl+A` / `Command+A` | 选择当前日志区域的全部行 |
| `Ctrl+C` / `Command+C` | 复制文字选区，或复制所选整行 |
| `Ctrl+Shift+C` / `Command+Shift+C` | 复制所选整行并包含源行号 |
| `M` | 标记或取消标记当前选择 |
| `W` | 切换当前标签的自动换行 |
| `Ctrl+,` / `Command+,` | 打开设置 |
| macOS：`⌃⌘F`；全平台：`F11` | 切换当前窗口的原生全屏显示；`F11` 为兼容入口 |

其中九项常用操作可在设置中重新绑定；`F5`、转到行、带行号复制和全屏等固定命令不可配置。macOS 默认使用 `Command`，Windows/Linux 默认使用 `Ctrl`；Windows/Linux 使用 `F11` 切换全屏。

“视图”菜单会在每次打开时根据当前窗口状态显示“进入全屏 / Enter full screen”或“退出全屏 / Exit full screen”。macOS 保留 AppKit 原生全屏行为：窗口模式的标题栏为三色窗口按钮保留约 80px 外层起始空间；进入全屏后系统隐藏三色按钮，外层占位归零，标题栏组件内部仍保留约 12px 的紧凑边距。

## 从源码构建

### 环境要求

- 所有平台：Rust stable、Git；首次解析 GPUI 与 gpui-component 依赖时需要访问 GitHub
- Windows：MSVC Rust 工具链，以及包含“使用 C++ 的桌面开发”和 Windows SDK 的 Visual Studio 2022 Build Tools
- macOS：Xcode 和 Xcode Command Line Tools
- Linux：Clang、CMake、Fontconfig、Vulkan、Wayland、X11/XCB 与 xkbcommon 开发库；GitHub Actions 使用 Ubuntu 22.04

检查本机环境：

```text
rustc --version
cargo --version
git --version
```

Ubuntu/Debian 可安装与 Actions 一致的 GPUI 构建依赖：

```bash
sudo apt-get update
sudo apt-get install --yes --no-install-recommends \
  build-essential clang cmake libfontconfig-dev libglib2.0-dev libssl-dev \
  libvulkan1 libwayland-dev libx11-dev libx11-xcb-dev libxcb1-dev \
  libxkbcommon-x11-dev pkg-config
```

Windows 可执行文件在链接时为 GPUI 主线程预留 8 MiB 栈空间，以容纳首帧原生界面树构建；该设置同时覆盖 Debug、Release 和直接启动脚本。

### 启动 Debug 环境

```powershell
powershell -ExecutionPolicy Bypass -File scripts/run-debug.ps1
```

首次运行会下载并编译 GPUI 依赖，耗时通常明显长于后续启动。也可以在已有构建产物后直接传入日志路径：

```powershell
.\target\debug\vclogg2.exe .\example.log D:\logs\service.log
```

### 构建与检查

Windows Debug 构建：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build-debug.ps1
```

Windows Release 构建：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build-release.ps1
```

Windows 静态检查：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/check.ps1
```

macOS/Linux 使用对应的 Shell 脚本：

```bash
./scripts/build-debug.sh
./scripts/build-release.sh
./scripts/check.sh
```

Release 可执行文件位于 Windows 的 `target\release\vclogg2.exe` 或 macOS/Linux 的 `target/release/vclogg2`。三平台脚本使用同一份锁定的 `Cargo.lock`，检查范围和优化配置一致。

## 性能诊断

### 生成大文件测试日志

使用 Python 3 可一次生成 50 MiB、100 MiB、500 MiB 三档日志：

```bash
python3 scripts/generate-test-data.py
```

默认输出到 `target/test-data/`。日志内容随机包含不同时间、级别、服务、状态码和延迟；默认约 8% 的行包含 800–2400 个 ASCII 字符的随机 payload，用于测试横向滚动和自动换行。每个文件会精确写入对应档位的字节数。

也可以只生成指定档位、固定随机种子或覆盖已有文件：

```bash
python3 scripts/generate-test-data.py 50M 100M --seed 20260831
python3 scripts/generate-test-data.py 500M --force
```

Windows 可将上述命令中的 `python3` 替换为 `py -3`；使用 `--output-dir <目录>` 可修改输出位置。

### UI 渲染性能诊断

Debug 构建默认启用 UI 渲染线程长任务检测。单个已标记渲染作用域达到 16 ms 时，终端和应用日志会记录作用域、实际耗时、线程信息与 Rust 堆栈；同一作用域的重复堆栈默认每 2 秒最多记录一次。

可在启动前调整 1–60000 ms 范围内的阈值与限频窗口：

```powershell
$env:VCLOGG2_UI_PERF_WARN_MS = '8'
$env:VCLOGG2_UI_PERF_REPEAT_MS = '1000'
powershell -ExecutionPolicy Bypass -File scripts/run-debug.ps1
```

如需与日常实例隔离，可使用专用性能验证脚本：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/run-performance-debug.ps1 `
  -WarnAfterMilliseconds 8 `
  -RepeatAfterMilliseconds 1000 `
  -Paths D:\logs\service.log,D:\logs\worker.log
```

该脚本使用 `target\perf-debug` 作为独立构建目录，并把状态数据库、缓存和崩溃报告定向到隔离的数据目录。它还启用仅限 Debug 的 `ui-performance-profiler` 特性，用于记录 GPUI 前台任务、输入处理、绘制和平台提交等长耗时贡献者。Release 构建不执行这些性能采样。

## 打包与发布

生成 Windows x64 便携目录、ZIP 与更新元数据：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-release.ps1
```

生成 macOS ARM64 `.app` ZIP 或 Linux x86_64 便携 TAR.GZ；两者同样包含安装脚本、整包/分块哈希与更新清单：

```bash
# 在 macOS 上执行
./scripts/package-release-macos.sh

# 在 Linux 上执行
./scripts/package-release-linux.sh
```

三种平台的产物分别写入 `dist/windows-x86_64/`、`dist/macos-aarch64/` 与 `dist/linux-x86_64/`。打包脚本根据当前提交上的 `v<SemVer>` tag 和实际 runner 架构命名产物，并把同一版本写入可执行文件、更新清单和 macOS 应用信息；macOS 还会生成临时签名的 `.app`。未打 tag 的手动构建使用 `0.0.0-dev+g<commit>`，也可通过 `VCLOGG2_BUILD_VERSION` 显式覆盖。

仓库中的 [`.github/workflows/release-build.yml`](.github/workflows/release-build.yml) 会在推送 `v*` tag 时并行构建 Windows x64、macOS ARM64 和 Linux x86_64，也支持在 GitHub Actions 页面手动执行仅构建产物。构建结果作为 Actions Artifacts 保存 14 天；推送 `v*` tag 且三个构建全部成功后，工作流会复核标签、三平台清单、文件大小和 SHA-256，再自动创建带生成式发行说明的 GitHub Release。Release 同时包含安装包、blockmap 和客户端可识别的 `latest-<platform>-<architecture>.json`。

正式发行版默认通过 GitHub Releases REST API 检查 `zhaiyanqi/vclogg2` 的最新正式版本，即使“设置 → 网络”没有业务服务器地址也能更新；若已配置服务器，原有静态更新目录会作为第二来源参与检查，两边都有更新时选择版本较新的包。GitHub 返回的仓库、标签、资产名称、大小和可用摘要会先与平台清单交叉验证；资产下载只允许重定向到 GitHub 管理的 HTTPS 域名，落盘前仍逐块及整包验证 SHA-256。标记为 prerelease 的版本不会进入普通客户端的自动更新通道。

发布新版本时不需要修改 `Cargo.toml` 或 `Cargo.lock` 中的版本号。提交并推送干净的 `main` 后，创建一个 `v<SemVer>` 标签，再交给脚本校验并推送：

```bash
VERSION=2.0.7
git tag -a "v${VERSION}" -m "VCLogg2 v${VERSION}"
./scripts/publish-github-release.sh "v${VERSION}"
```

脚本不会创建或修改标签。它要求标签是合法的 `v<SemVer>`、已存在并指向当前 `main`，同时要求本地 `main` 与 `origin/main` 完全一致；运行格式、静态检查和 Clippy 后只推送该标签，标签会触发上述 GitHub Action。无人值守调用可追加 `--yes`，非默认远端可使用 `--remote <名称>`。

任一平台的产物都可通过跨平台发布脚本写入对应静态更新目录；脚本先验证清单、大小和 SHA-256，最后原子替换 `latest.json`：

```bash
./scripts/publish-update.py \
  --source dist/macos-aarch64 \
  --target /srv/vclogg2/updates-vclogg2/macos-aarch64
```

Windows 也可继续使用 PowerShell 包装脚本：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/publish-update.ps1 `
  -TargetDirectory D:\server\data\updates-vclogg2\windows-x86_64
```

GitHub Releases 与自建静态源使用同一更新协议：验证分块与整包 SHA-256，并在应用正常退出、会话完成保存后交给平台独立助手安装并重启。更新清单本身不做独立数字签名，因此仓库/更新源权限、HTTPS 以及 Windows 代码签名或 macOS Developer ID 签名/公证仍由发布环境负责。完整交付边界见 [`doc/delivery.md`](doc/delivery.md)。

## 工程结构

```text
crates/vclogg-app/     GPUI Component 桌面应用与工作区编排
crates/vclogg-core/    日志索引、按需读取与搜索核心
doc/                   架构、界面、功能状态与交付文档
scripts/               启动、检查、构建、测试数据、打包和发布脚本
```

`vclogg-app` 负责应用外壳、界面和后台任务编排；`vclogg-core` 不依赖 GPUI，负责文件快照、行索引、解码与搜索。更完整的职责边界见 [`doc/architecture.md`](doc/architecture.md)。

界面实现使用语义主题 token、rem 比例布局、桌面键盘路径、稳定元素身份及 GPUI Component 标准组件。

## 本地数据与隐私

- 会话状态保存在系统应用数据目录下的 `VCLogg2/sessions/vclogg2-state.db`，数据库使用 WAL。
- 行索引缓存在系统缓存目录中，与 SQLite 会话身份分离；缓存失效时会安全重建。
- Rust 线程 panic 报告保存在同级 `VCLogg2/crashes/panic-*.log`，默认只保留最近 20 份；应用数据目录不可写时回退到系统临时目录。
- 云端连接的公开配置写入 SQLite，Cookie 与 CSRF 仅保存在系统凭据库；公开目录离线缓存不包含账户秘密。
- 应用日志使用有界内存缓冲，不会自行创建长期日志文件；只有用户显式导出时才写入磁盘。
- Windows PDB 只进入开发侧符号包，不进入用户分发包或安装目录。

## 项目文档

| 文档 | 内容 |
| --- | --- |
| [`doc/migration-status.md`](doc/migration-status.md) | 当前功能范围、完成状态与后续交付点 |
| [`doc/feature-parity.md`](doc/feature-parity.md) | 已实现能力证据、待确认差异与验收步骤 |
| [`doc/architecture.md`](doc/architecture.md) | 模块职责、状态边界与关键实现约束 |
| [`doc/ui-layout.md`](doc/ui-layout.md) | UI 控件层级、稳定编号与交互说明 |
| [`doc/delivery.md`](doc/delivery.md) | 三平台构建、安装、更新与发布交付 |
| [`doc/ui-polish-acceptance.md`](doc/ui-polish-acceptance.md) | 界面细节与手工视觉验收清单 |

## 参与贡献

欢迎提交问题、改进建议与代码贡献。开始修改前，请先阅读仓库中的 [`RULES.md`](RULES.md) 以及相关架构、界面文档，并保持以下约定：

1. 将 `vclogg-core` 保持为不依赖 GPUI 的日志领域核心。
2. 修改功能边界、UI 层级或交付流程时，同步更新对应 `doc/` 文档。
3. 提交前在 Windows 运行 `scripts/check.ps1`，在 macOS/Linux 运行 `scripts/check.sh`，确保格式、`cargo check` 与 Clippy 检查通过。
4. 在问题或变更说明中写明复现路径、预期行为、实际行为和验证范围。

## 鸣谢

特别感谢 [klogg](https://github.com/variar/klogg) 项目在高性能日志浏览与检索领域的探索。VCLogg2 在大文件日志浏览、正文与筛选结果分离、搜索和跟随等产品思路上借鉴了 klogg 的思想，并在 Rust 与 GPUI 技术栈上继续探索原生实现。

同时感谢 Rust、[GPUI](https://github.com/zed-industries/zed) 与 [gpui-component](https://github.com/longbridge/gpui-component) 社区提供的基础设施和开源成果。

## 许可证

本项目采用 [Apache License 2.0](LICENSE) 开源许可证。
