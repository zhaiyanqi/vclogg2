# 交付说明

VCLogg2 在 Windows、macOS 与 Linux 上提供同一套产品能力。三平台使用同一份 `Cargo.lock`，分别由对应原生 runner 构建；平台差异只存在于安装格式、系统注册方式、代码签名和运行库。

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

Windows 使用 Inno Setup 6.3 或更新的 6.x 版本生成安装向导。安装命令与 `ISCC.exe` 自定义路径见 `README.md`；GitHub `windows-2022` runner 使用镜像预装的 Inno Setup 6。默认 `None` 模式同时生成未签名的便携 ZIP 和安装 EXE，包名不再包含 `unsigned`。

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-release.ps1
```

Windows x64 产物输出位于 `dist/windows-x86_64/`：

- `vclogg-<version>-windows-x86_64-portable/`：仅包含 `vclogg2.exe`、README 和许可证的暂存目录；
- `vclogg-<version>-windows-x86_64-portable.zip`：便携版，解压后直接运行，数据写入 EXE 同级 `VCLogg2` 子目录；
- `vclogg-<version>-windows-x86_64-setup.exe`：安装版；
- `vclogg2-<version>-windows-x86_64-symbols.zip`：仅供开发侧崩溃分析的 PDB，不上传为用户包。

以标签 `v2.2.4` 为例，Actions 分别上传 `vclogg-2.2.4-windows-x86_64-portable` 和 `vclogg-2.2.4-windows-x86_64-setup` 两个产物，各含上述 ZIP / EXE。GitHub Release 直接附加这两个文件，以及 macOS DMG、Linux tar.gz。版本取自构建标签或 `VCLOGG2_BUILD_VERSION`，没有固定为 2.2.4；无标签的手动构建沿用开发版本规则。

#### 安装与数据目录

安装版按当前用户安装，无需管理员权限；默认程序目录为 `%LOCALAPPDATA%\Programs\VCLogg2`。向导始终显示安装目录选择页，并提供独立的数据目录选择页，默认 `%LOCALAPPDATA%\VCLogg2`，允许选择当前用户可写的其他绝对目录。开始菜单入口默认创建，桌面快捷方式可选，卸载入口由 Inno Setup 注册。当前没有文件关联或自动更新。

静默安装可指定两个目录：

```powershell
.\vclogg-2.2.4-windows-x86_64-setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /DIR="D:\Apps\VCLogg2" /DATADIR="D:\VCLogg2Data"
```

安装器检查数据目录可创建、可写并可清理检查文件后，通过 Inno 的文件安装流程写入 EXE 同级 `vclogg2-data-dir.txt`。该文件采用 UTF-8，内容为所选绝对路径；所选目录本身就是数据根，不再追加一层 `VCLogg2`。状态库、索引、公开云目录缓存、崩溃报告和临时结果分别位于该根下的 `sessions`、`index`、`cloud`、`crashes`、`temp`。

程序在启动时只解析一次配置：没有该文件则为便携版（包括默认 Debug 启动），配置存在但无法读取、路径无效或目录不可写则显示错误并退出，不静默切换到其他数据目录。Windows 单实例命名管道同时以当前用户和规范化数据目录为范围，避免安装版和不同便携目录互相转交启动请求；同一数据目录仍使用同一进程。

覆盖升级默认复用已安装的程序目录及其数据配置，显式 `/DATADIR` 优先于旧配置。更换数据目录只改变后续使用的位置，不自动复制或删除旧数据；如需搬迁，应先关闭使用该目录的程序，再手动完整复制数据。卸载移除程序、快捷方式和安装配置，保留用户数据。便携版更新仅替换程序文件并保留同级 `VCLogg2` 目录。

#### Windows 签名

- `None`：不读取签名 Secrets，两种用户包均未签名。
- `Pfx`：从 `VCLOGG2_WINDOWS_SIGNING_CERTIFICATE_PATH`、`VCLOGG2_WINDOWS_SIGNING_CERTIFICATE_PASSWORD`、`VCLOGG2_WINDOWS_TIMESTAMP_URL` 读取配置，先签署应用 EXE，再打包并签署安装 EXE。
- `PreSigned -SkipBuild`：验证外部后端已签署的应用 EXE，然后生成两种包；新生成的安装 EXE 仍须由外部后端签署。Actions 的 Artifact Signing 分支在打包之后执行第二次签名。

签名分支发布前会解压便携 ZIP 验证程序，并单独验证安装 EXE 的 Authenticode 信任链与 RFC 3161 时间戳，任一步失败都不上传用户包。当前签名范围是应用和安装 EXE；Inno 生成的卸载程序未单独签名。产物名不代表签名状态，可在 Windows 文件属性的“数字签名”页查看。

### macOS aarch64

```bash
./scripts/package-release-macos.sh
```

输出位于 `dist/macos-aarch64/`：

- `vclogg2-<version>-macos-aarch64.dmg`：包含原生应用包及 `/Applications` 快捷方式的拖拽安装镜像。

双击挂载 DMG，将 `VCLogg2.app` 拖动到镜像窗口中的 `Applications` 文件夹即可安装。应用包通过 `Info.plist` 声明支持的文档类型。Actions 产物只使用临时签名，没有 Apple Developer ID 签名和公证；正式对外分发应在打包后增加 Developer ID 签名、公证与 stapling。

### Linux x86_64

```bash
./scripts/package-release-linux.sh
```

输出位于 `dist/linux-x86_64/`：

- `vclogg2-<version>-linux-x86_64/`：便携目录，包含程序、图标、当前用户安装脚本、README 和许可证；
- `vclogg2-<version>-linux-x86_64.tar.gz`：用户分发包；

解压后执行：

```bash
./Install-VCLogg2-linux.sh --launch
```

默认目标为 `~/.local/lib/vclogg2`，同时创建 `~/.local/bin/vclogg2` 入口以及用户级 `.desktop`、图标和 MIME 注册。运行环境需要兼容的 glibc、Fontconfig、Vulkan 与 Wayland/X11 库。

## 功能一致性边界

三个发行包均提供：

- 单实例和多原生窗口；Windows 按用户与数据目录使用命名管道，macOS/Linux 使用权限受限的 Unix domain socket；
- 终端参数、系统文件关联、Finder/桌面环境打开事件和多文件拖放；
- 系统废纸篓删除，不做不可恢复的直接删除；
- 平台强文件身份索引缓存：Windows 使用卷/文件标识与 USN，macOS/Linux 使用设备号、inode 与 ctime；
- “帮助 → 更新”统一在系统浏览器中打开项目 GitHub Releases 页面；
- 相同日志查看、搜索、导出、会话、云端过滤器和多窗口行为。

macOS 默认快捷键主修饰键为 `Command`，Windows/Linux 为 `Ctrl`。这是原生交互约定，不是功能裁剪。

## 版本发布与更新入口

客户端不访问 GitHub Releases API，不检查版本，不下载发布包，也不启动独立安装助手。“帮助 → 更新”直接调用系统 URL 打开能力访问 `https://github.com/zhaiyanqi/vclogg2/releases`。菜单中的“关于 ver.<版本>”使用当前可执行文件的构建版本，方便用户在下载前自行核对。

发布正式版本前，提交并推送干净的 `main`，创建指向当前提交的 `v<SemVer>` 标签，然后执行：

```bash
VERSION=2.0.8
git tag -a "v${VERSION}" -m "VCLogg2 v${VERSION}"
./scripts/publish-github-release.sh "v${VERSION}"
```

脚本只接受干净且与 `origin/main` 完全一致的 `main`，校验已存在且指向当前提交的标签，运行 `scripts/check.sh` 后推送标签。GitHub Action 收到标签后负责三平台构建与 Release 创建；`--yes` 可跳过交互确认，`--remote <名称>` 可替换默认 `origin`。普通推送 `main` 只触发 CI；手动运行 release workflow 只保留 Artifacts，不创建 GitHub Release。

GitHub 仓库写权限构成发布信任边界。Windows 未配置签名时生成未签名的便携版和安装版，文件名不区分签名状态；Microsoft Artifact Signing 与 PFX 后端中，前者通过 GitHub OIDC 使用 HSM 托管私钥，后者从加密 Secrets 临时还原适用的可导出证书。启用任一后端时必须通过 Authenticode 信任链与 RFC 3161 时间戳验证，上传前还会解压便携 ZIP 并校验安装 EXE 的签名；PFX 临时文件会在成功或失败后删除。macOS 正式公开分发仍应增加 Developer ID 签名和公证。

## GitHub Actions

`.github/workflows/release-build.yml` 包含三个独立 job：

- Windows Server 2022 x64：默认 `unsigned` 模式不读取签名 Secrets，分别生成便携 ZIP 与安装 EXE，包名不追加签名标记；配置 `WINDOWS_SIGNING_PROVIDER=artifact-signing` 时通过 Azure OIDC 与 Artifact Signing 完成 HSM 签名和 Microsoft RFC 3161 时间戳；配置为 `pfx` 时从 `WINDOWS_SIGNING_CERTIFICATE_BASE64` 与 `WINDOWS_SIGNING_CERTIFICATE_PASSWORD` Secrets 临时还原证书，并使用可选 `WINDOWS_TIMESTAMP_URL`；
- macOS 15 ARM64：`check.sh` + `package-release-macos.sh`；
- Ubuntu 22.04 x86_64：安装 GPUI 原生依赖后执行 `check.sh` + `package-release-linux.sh`。

`.github/workflows/ci.yml` 在推送或 PR 指向 `main` 时执行常规检查；`.github/workflows/release-build.yml` 在推送 `v*` tag 时触发，也支持手动运行。Windows 分别上传便携与安装产物，其他平台各上传一个产物，均保留 14 天。普通构建 job 保持 `contents: read`；Windows job 额外取得 `id-token: write`，只用于 Artifact Signing 的 Azure OIDC 登录。只有已存在的 `v*` tag 触发且三平台全部成功时，末尾 `release` job 才取得 `contents: write`。该 job 核对四个包名与标签版本后创建 GitHub Release；带 `-` 的 SemVer 标签发布为 prerelease，其余版本显式标为 Latest。

## 验证脚本

- `scripts/check.ps1` / `scripts/check.sh`：格式、workspace 全 target 静态检查和 Clippy；
- `scripts/build-debug.ps1` / `scripts/build-debug.sh`：Debug 二进制；
- `scripts/build-release.ps1` / `scripts/build-release.sh`：Release 二进制；
- `scripts/windows-installer.iss`：当前用户安装向导、自定义安装/数据目录、快捷方式与保留数据的卸载；
- `scripts/sign-windows.ps1`：使用 Windows SDK SignTool 对 PE 执行 SHA-256 Authenticode 签名、RFC 3161 时间戳及双重验证；
- `scripts/package-release.ps1` / `package-release-macos.sh` / `package-release-linux.sh`：平台 Release 包；Windows 同时生成便携 ZIP 与安装 EXE；
- `scripts/publish-github-release.sh`：校验本地发布状态，创建并推送触发 GitHub Release 的版本标签；
