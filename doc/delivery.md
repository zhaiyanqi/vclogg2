# 交付说明

VCLogg2 在 Windows、macOS 与 Linux 上提供同一套产品能力和更新协议。三平台使用同一份 `Cargo.lock`，分别由对应原生 runner 构建；平台差异只存在于安装格式、系统注册方式、代码签名和运行库。

## 开发环境

Windows 在仓库根目录执行：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/run-debug.ps1
```

macOS/Linux 可先构建再启动 Debug 二进制：

```bash
./scripts/build-debug.sh
./target/debug/vclogg2
```

首次执行需要网络访问来解析 GPUI 与 gpui-component；后续可复用 Cargo 缓存。平台原生依赖见仓库 `README.md` 的“从源码构建”章节。

## 三平台 Release 产物

### Windows x86_64

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-release.ps1
```

输出位于 `dist/windows-x86_64/`：

- `vclogg2-<version>-windows-x86_64/`：便携目录，仅包含程序、README 和许可证；
- `vclogg2-<version>-windows-x86_64.zip`：用户分发包；
- `vclogg2-<version>-windows-x86_64-symbols.zip`：开发侧崩溃分析 PDB；
- `vclogg2-<version>-windows-x86_64.blockmap.json` 与 `latest.json`：更新校验和版本清单。

解压后直接启动：

```powershell
.\vclogg2.exe
```

Windows Release 不附带 PowerShell 安装或更新脚本，不创建开始菜单快捷方式，也不注册文件关联。应用内更新助手由可执行程序内置，退出应用后直接从已校验的更新包替换程序与随附文档并重新启动。

旧版 Windows 更新助手会在下载包内查找 `Install-VCLogg2.ps1`，因此从这类旧版本迁移到首个纯三文件版本时不能依赖应用内更新，必须发布为手动替换，或者先发布一个仍含旧安装脚本的过渡版本。用户启动纯三文件版本一次后，后续版本均走新的内置替换流程。

### macOS aarch64

```bash
./scripts/package-release-macos.sh
```

输出位于 `dist/macos-aarch64/`：

- `vclogg2-<version>-macos-aarch64/VCLogg2.app`：包含文档类型声明的原生应用包；
- `vclogg2-<version>-macos-aarch64.zip`：用户分发包，包含当前用户安装与更新辅助；
- 同名 `.blockmap.json` 与 `latest.json`：更新校验和版本清单。

解压后执行：

```bash
./Install-VCLogg2-macos.sh --launch
```

默认目标为 `~/Applications/VCLogg2.app`，安装后通过 Launch Services 注册文件打开能力。Actions 产物只使用临时签名，没有 Apple Developer ID 签名和公证；正式对外分发应在打包后增加 Developer ID 签名、公证与 stapling。

### Linux x86_64

```bash
./scripts/package-release-linux.sh
```

输出位于 `dist/linux-x86_64/`：

- `vclogg2-<version>-linux-x86_64/`：便携目录，包含程序、图标、安装与更新辅助、README 和许可证；
- `vclogg2-<version>-linux-x86_64.tar.gz`：用户分发包；
- 同名 `.blockmap.json` 与 `latest.json`：更新校验和版本清单。

解压后执行：

```bash
./Install-VCLogg2-linux.sh --launch
```

默认目标为 `~/.local/lib/vclogg2`，同时创建 `~/.local/bin/vclogg2` 入口以及用户级 `.desktop`、图标和 MIME 注册。运行环境需要兼容的 glibc、Fontconfig、Vulkan 与 Wayland/X11 库。

## 功能一致性边界

三个发行包均提供：

- 每用户单实例和多原生窗口；Windows 使用命名管道，macOS/Linux 使用权限受限的 Unix domain socket；
- 终端参数、系统文件关联、Finder/桌面环境打开事件和多文件拖放；
- 系统废纸篓删除，不做不可恢复的直接删除；
- 平台强文件身份索引缓存：Windows 使用卷/文件标识与 USN，macOS/Linux 使用设备号、inode 与 ctime；
- 同一应用内更新清单、1 MiB 分块 SHA-256、正常退出后独立安装及重启；
- 相同日志查看、搜索、导出、会话、云端过滤器和多窗口行为。

macOS 默认快捷键主修饰键为 `Command`，Windows/Linux 为 `Ctrl`。这是原生交互约定，不是功能裁剪。

## 更新发布

正式发行版默认调用 `https://api.github.com/repos/zhaiyanqi/vclogg2/releases/latest` 获取最新正式 Release，并选择当前平台的唯一清单；如果“设置 → 网络”中存在服务器地址，原有静态目录同时作为第二来源参与检查，两边都有可用更新时选择版本较新的包：

```text
latest-windows-x86_64.json
latest-macos-aarch64.json
latest-linux-x86_64.json
```

客户端交叉验证 Release 标签、清单版本、仓库资产名称、文件大小和 GitHub 可用的 SHA-256 摘要。GitHub 资产允许的 `302` 仅可落到 GitHub 管理的 HTTPS 域名；普通 HTTP/HTTPS 静态源仍维持禁止重定向。标为 prerelease 的 Release 不由普通客户端自动发现。

发布正式版本前，先修改 workspace 版本并同步 `Cargo.lock`，提交和推送 `main` 后执行：

```bash
./scripts/publish-github-release.sh
```

脚本只接受干净且与 `origin/main` 完全一致的 `main`，运行 `scripts/check.sh` 后创建带注释的 `v<版本>` 标签并推送。GitHub Action 收到标签后负责三平台构建与 Release 创建；`--yes` 可跳过交互确认，`--remote <名称>` 可替换默认 `origin`。

自建静态更新源继续受支持，其目录从服务器根地址派生：

```text
/updates-vclogg2/windows-x86_64/
/updates-vclogg2/macos-aarch64/
/updates-vclogg2/linux-x86_64/
```

发布任一平台：

```bash
./scripts/publish-update.py \
  --source dist/linux-x86_64 \
  --target /srv/vclogg2/updates-vclogg2/linux-x86_64
```

脚本验证 `latest.json`、压缩包大小与 SHA-256，先复制版本化压缩包和 blockmap，最后原子替换清单。Windows 还可以使用：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/publish-update.ps1 `
  -TargetDirectory D:\log-viewer-server\data\updates-vclogg2\windows-x86_64
```

Release 构建会在启动 15 秒后从 GitHub Releases 检查一次，也可由用户手动触发；未配置云端过滤器服务器也能完成更新，已配置时则额外检查对应静态源。下载过程中即时校验分块与整包 SHA-256；用户确认后，独立助手等待应用完成会话保存与正常退出，再完成平台对应的文件替换或安装流程并重启。

清单和哈希不提供独立发布者签名；GitHub 模式依赖仓库写权限与 GitHub HTTPS，自建模式依赖更新源访问控制与 HTTPS。正式公开分发还应对 Windows 二进制进行代码签名，对 macOS 应用进行 Developer ID 签名和公证。

## GitHub Actions

`.github/workflows/release-build.yml` 包含三个独立 job：

- Windows Server 2022 x64：`check.ps1` + `package-release.ps1`；
- macOS 15 ARM64：`check.sh` + `package-release-macos.sh`；
- Ubuntu 22.04 x86_64：安装 GPUI 原生依赖后执行 `check.sh` + `package-release-linux.sh`。

工作流在推送或 PR 指向 `main`、推送 `v*` tag 时触发，也支持手动触发。每个平台上传压缩包、blockmap 与 `latest.json`，保留 14 天。普通构建 job 保持 `contents: read`；只有已存在的 `v*` tag 触发且三平台全部成功时，末尾 `release` job 才取得 `contents: write`。该 job 验证标签版本、平台/架构、安装包大小与 SHA-256，把三份同名 `latest.json` 复制为唯一的 `latest-<platform>-<architecture>.json`，再连同三平台安装包和 blockmap 创建 GitHub Release。带 `-` 的 SemVer 标签发布为 prerelease，其余版本显式标为 Latest。

## 验证脚本

- `scripts/check.ps1` / `scripts/check.sh`：格式、workspace 全 target 静态检查和 Clippy；
- `scripts/build-debug.ps1` / `scripts/build-debug.sh`：Debug 二进制；
- `scripts/build-release.ps1` / `scripts/build-release.sh`：Release 二进制；
- `scripts/package-release.ps1` / `package-release-macos.sh` / `package-release-linux.sh`：平台 Release 包与更新元数据；
- `scripts/publish-github-release.sh`：校验本地发布状态，创建并推送触发 GitHub Release 的版本标签；
- `scripts/publish-update.py`：三平台通用发布；`scripts/publish-update.ps1` 是 Windows 专用包装。
