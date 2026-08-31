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

- `vclogg2-<version>-windows-x86_64/`：便携目录，包含程序、安装与更新辅助、README 和许可证；
- `vclogg2-<version>-windows-x86_64.zip`：用户分发包；
- `vclogg2-<version>-windows-x86_64-symbols.zip`：开发侧崩溃分析 PDB；
- `vclogg2-<version>-windows-x86_64.blockmap.json` 与 `latest.json`：更新校验和版本清单。

安装到当前用户目录并注册开始菜单及“打开方式”候选：

```powershell
powershell -ExecutionPolicy Bypass -File .\Install-VCLogg2.ps1 -Launch
```

默认目标为 `%LOCALAPPDATA%\Programs\VCLogg2`。安装脚本注册 `.log`、`.txt`、`.out`、`.trace`、`.csv` 和 `.json`，但不会写入 `UserChoice` 或替用户更改默认应用。

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

客户端从配置服务器根地址派生平台更新源：

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

Release 构建会在启动 15 秒后检查一次，也可由用户手动触发。下载过程中即时校验分块与整包 SHA-256；用户确认后，独立助手等待应用完成会话保存与正常退出，再调用对应平台安装脚本并重启。

清单和哈希不提供发布者身份认证。部署环境必须保护更新源并使用 HTTPS；正式公开分发还应对 Windows 二进制进行代码签名，对 macOS 应用进行 Developer ID 签名和公证。

## GitHub Actions

`.github/workflows/release-build.yml` 包含三个独立 job：

- Windows Server 2022 x64：`check.ps1` + `package-release.ps1`；
- macOS 15 ARM64：`check.sh` + `package-release-macos.sh`；
- Ubuntu 22.04 x86_64：安装 GPUI 原生依赖后执行 `check.sh` + `package-release-linux.sh`。

工作流在推送或 PR 指向 `main`、推送 `v*` tag 时触发，也支持手动触发。每个平台上传压缩包、blockmap 与 `latest.json`，保留 14 天。普通构建 job 保持 `contents: read`；只有已存在的 `v*` tag 触发且三平台全部成功时，末尾 `release` job 才取得 `contents: write`，下载三份 Artifact 并创建带自动发行说明的 GitHub Release。Release 附件包含三平台安装包与各自 blockmap；同名的三个 `latest.json` 继续留在平台 Artifact 和独立更新目录中，不上传到 Release。

## 验证脚本

- `scripts/check.ps1` / `scripts/check.sh`：格式、workspace 全 target 静态检查和 Clippy；
- `scripts/build-debug.ps1` / `scripts/build-debug.sh`：Debug 二进制；
- `scripts/build-release.ps1` / `scripts/build-release.sh`：Release 二进制；
- `scripts/package-release.ps1` / `package-release-macos.sh` / `package-release-linux.sh`：平台 Release 包与更新元数据；
- `scripts/publish-update.py`：三平台通用发布；`scripts/publish-update.ps1` 是 Windows 专用包装。
