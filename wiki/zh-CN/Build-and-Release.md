# 交付说明

[English](../en/Build-and-Release.md) · [Home / 首页](Home.md)

[Wiki 首页](Home.md) · [文档维护](Documentation.md)


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

首次执行需要网络访问来下载 GPUI Kit 及其依赖；后续可复用 Cargo 缓存。平台依赖和完整构建命令见[源码构建](Building.md)。

## 三平台 Release 产物

### Windows x86_64

Windows 使用 Inno Setup 6.3 或更新的 6.x 版本生成安装向导。GitHub `windows-2022` runner 使用镜像预装的 Inno Setup 6。默认 `None` 模式同时生成未签名的便携 ZIP 和安装 EXE，包名不再包含 `unsigned`。

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-release.ps1
```

打包前可通过 Windows Package Manager 安装 Inno Setup：

```powershell
winget install --id JRSoftware.InnoSetup --exact --source winget
```

脚本从 PATH 和 Inno Setup 6 默认目录查找 `ISCC.exe`；自定义位置可通过 `-InnoSetupCompiler "D:\Tools\Inno Setup 6\ISCC.exe"` 指定。

Windows x64 产物输出位于 `dist/windows-x86_64/`：

- `vclogg-<version>-windows-x86_64-portable/`：包含 `vclogg2.exe`、`rg.exe`、README 和许可证的暂存目录；
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

双击挂载 DMG，将 `VCLogg2.app` 拖动到镜像窗口中的 `Applications` 文件夹即可安装。应用包通过 `crates/vclogg-app/resources/macos/Info.plist` 声明文档类型及简体中文、英文支持；打包脚本复制模板后填入发行版本。同一模板也嵌入 macOS 可执行文件，使直接运行 Debug/Release 二进制时具有相同语言声明。AppKit 启动前使用应用已保存的语言；修改语言后须保存并重启才能切换系统窗口菜单。Actions 产物只使用临时签名，没有 Apple Developer ID 签名和公证；正式对外分发应在打包后增加 Developer ID 签名、公证与 stapling。

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
- 终端参数和多文件拖放；macOS 声明文档类型，Linux 安装器注册 desktop/MIME，Windows 两种包不主动注册文件关联；
- 仅对应用自有临时结果提供系统废纸篓清理，普通浏览不写源日志，显式编辑保存除外；
- 平台强文件身份索引缓存：Windows 使用卷/文件标识与 USN，macOS/Linux 使用设备号、inode 与 ctime；
- “帮助 → 检查更新”读取官网版本清单，新版或检查失败时打开 GitHub Releases；
- 相同日志查看、搜索、导出、会话、云端过滤器和多窗口行为。

macOS 默认快捷键主修饰键为 `Command`，Windows/Linux 为 `Ctrl`。这是原生交互约定，不是功能裁剪。

## 版本发布与更新入口

“帮助 → 检查更新”读取官网 `https://zhaiyanqi.github.io/vclogg2/latest.json`，按 SemVer 比较稳定版本。发现新版时打开对应 Release 下载页面；网络、HTTP 或清单校验失败时打开 Releases 首页；没有新版时显示检查结果。成功结果缓存 24 小时，失败按 1、2、4、8、16、32、60 分钟退避，之后保持 60 分钟；退避期间点击只打开 Releases，不重新请求官网。缓存与退避状态保存在本地 SQLite，跨重启有效；存储不可用时保留进程内缓存。多个窗口共享进行中的检查。连接超时 5 秒、请求总超时 10 秒，不立即重试。仅用户点击触发，不在启动时联网，不自动下载或安装。

Pages 部署在 `_site/latest.json` 生成清单，使用工作流令牌读取 GitHub 最新正式 Release，校验稳定版标签、发布时间、官方链接和四种桌面安装包；不把 main 的占位版本或预发布版写入清单。生成失败则不部署，保留上一份官网。Release 发布、编辑、删除，以及标签发布构建成功后会重新部署官网；后者使用 `workflow_run`，覆盖 `GITHUB_TOKEN` 创建 Release 不触发普通 Release 事件的情况。仅部署端访问 Releases API，客户端无须令牌。

发布正式版本前，提交并推送干净的 `main`，创建指向当前提交的 `v<SemVer>` 标签，然后执行：

```bash
VERSION=2.0.8
git tag -a "v${VERSION}" -m "VCLogg2 v${VERSION}"
./scripts/publish-github-release.sh "v${VERSION}"
```

脚本只接受干净且与 `origin/main` 完全一致的 `main`，校验已存在且指向当前提交的标签，运行 `scripts/check.sh` 后推送标签。GitHub Action 收到标签后负责桌面三平台和服务端构建与 Release 创建；`--yes` 可跳过交互确认，`--remote <名称>` 可替换默认 `origin`。普通推送 `main` 不触发常规 CI；涉及 `doc/**`、`wiki/**`、文档构建脚本或 Pages 工作流的变更会触发官网与文档一同部署；手动运行 release workflow 只保留 Artifacts，不创建 GitHub Release。

GitHub 仓库写权限构成发布信任边界。Windows 未配置签名时生成未签名的便携版和安装版，文件名不区分签名状态；Microsoft Artifact Signing 与 PFX 后端中，前者通过 GitHub OIDC 使用 HSM 托管私钥，后者从加密 Secrets 临时还原适用的可导出证书。启用任一后端时必须通过 Authenticode 信任链与 RFC 3161 时间戳验证，上传前还会解压便携 ZIP 并校验安装 EXE 的签名；PFX 临时文件会在成功或失败后删除。macOS 正式公开分发仍应增加 Developer ID 签名和公证。

## GitHub Actions

`.github/workflows/release-build.yml` 包含四个独立构建 job：

- Windows Server 2022 x64：默认 `unsigned` 模式不读取签名 Secrets，分别生成便携 ZIP 与安装 EXE，包名不追加签名标记；配置 `WINDOWS_SIGNING_PROVIDER=artifact-signing` 时通过 Azure OIDC 与 Artifact Signing 完成 HSM 签名和 Microsoft RFC 3161 时间戳；配置为 `pfx` 时从 `WINDOWS_SIGNING_CERTIFICATE_BASE64` 与 `WINDOWS_SIGNING_CERTIFICATE_PASSWORD` Secrets 临时还原证书，并使用可选 `WINDOWS_TIMESTAMP_URL`；
- macOS 15 ARM64：`package-release-macos.sh`；
- Ubuntu 22.04 x86_64：安装 GPUI 原生依赖后执行 `package-release-linux.sh`；
- 服务端：Ubuntu 22.04 执行 Go 格式/vet/测试，安装 Node.js 24 与锁定的 pnpm，调用 `package-server-release.sh` 生成 Windows/Linux amd64 包及 SHA-256 校验文件，并检查内嵌管理资源与源码一致。

`.github/workflows/ci.yml` 仅通过 `workflow_dispatch` 手动执行常规检查；`.github/workflows/release-build.yml` 在推送 `v*` tag 时触发，也支持手动运行。Windows 分别上传便携与安装产物，其他平台各上传一个产物，均保留 14 天。普通构建 job 保持 `contents: read`；Windows job 额外取得 `id-token: write`，只用于 Artifact Signing 的 Azure OIDC 登录。只有已存在的 `v*` tag 触发且桌面三平台及服务端全部成功时，末尾 `release` job 才取得 `contents: write`。该 job 核对四个桌面包、两个服务端包和服务端校验清单的名称与标签版本，验证服务端包摘要后创建 GitHub Release；带 `-` 的 SemVer 标签发布为 prerelease，其余版本显式标为 Latest。

## 验证脚本

- `scripts/check.ps1` / `scripts/check.sh`：格式、workspace 全 target 编译分析（cargo check）和 Clippy，不等同于仅文本检查；
- `scripts/build-debug.ps1` / `scripts/build-debug.sh`：Debug 二进制；
- `scripts/build-release.ps1` / `scripts/build-release.sh`：Release 二进制；
- `scripts/windows-installer.iss`：当前用户安装向导、自定义安装/数据目录、快捷方式与保留数据的卸载；
- `scripts/sign-windows.ps1`：使用 Windows SDK SignTool 对 PE 执行 SHA-256 Authenticode 签名、RFC 3161 时间戳及双重验证；
- `scripts/package-release.ps1` / `package-release-macos.sh` / `package-release-linux.sh`：平台 Release 包；Windows 同时生成便携 ZIP 与安装 EXE；
- `scripts/publish-github-release.sh`：校验本地发布状态及已存在的版本标签，再推送该标签以触发 GitHub Release；


## 服务端发布包

服务端代码位于 `server/`，打包不依赖桌面构建。工具链安装命令见 [Building](Building.md#云端过滤器服务端)。从仓库根目录执行：

```bash
./scripts/package-server-release.sh
./scripts/package-server-release.sh --version v2.2.14 --output-dir ./dist/server
```

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-server-release.ps1
powershell -ExecutionPolicy Bypass -File scripts/package-server-release.ps1 -Version v2.2.14 -OutputDirectory .\dist\server
```

两个入口调用同一 Node.js 打包实现。也可在 `server/` 运行 `pnpm run build`；原 `server/scripts/build-release.ps1` 和 `build-server.bat` 保留为兼容入口。默认版本依次取显式参数、`VCLOGG2_BUILD_VERSION`、当前提交的 `v*` 标签，最后为 `0.0.0-dev+g<commit>`；Git 元数据不可用时为 `0.0.0`。

输出到 `dist/server/`：`vclogg-server-<version>-windows-amd64.zip`、`vclogg-server-<version>-linux-amd64.tar.gz`、`vclogg-server-<version>-SHA256SUMS.txt`。发布包只含程序、配置示例、README、LICENSE 和 VERSION；部署数据和本机配置不打包。Linux 解压后可执行 `chmod +x vclogg-server` 恢复 Windows 本机构建包的执行权限。

工作流手动参数 `component` 可选 `all`（默认）、`desktop`、`server`；手动运行只保存 Artifacts。推送 `v*` 标签始终构建全部组件，共用一个 GitHub Release。原 `scripts/publish-github-release.sh` 推送已存在标签的入口自动覆盖服务端，无需额外服务端标签；本地打包脚本不执行 Git 推送、上传或部署。

可选公开描述文件：Bash 使用 `--client-key-file /path/to/client-public-key.json`，PowerShell 使用 `-ClientKeyFile D:\path\to\client-public-key.json`。也可使用 `VCLOGG_CLIENT_KEY_ID` 和 `VCLOGG_CLIENT_PUBLIC_KEY`；CI 从同名 Repository Variables 读取。默认鉴权配置要求部署者提供受信任公钥；不设置构建公钥时依然能打包，运行前需配置 `client_auth.public_keys` 或按实际协议调整服务配置。私钥不属于服务端构建输入。

验证范围：此次仅完成脚本/工作流静态检查；未运行 Go/管理端构建、Go 测试、PowerShell 打包或远程 GitHub Actions，也未创建发布版本。

## 当前打包前置条件

桌面三个打包脚本仍要求 ripgrep：Windows 从 PATH 查找 `rg.exe`；macOS/Linux 从 PATH 查找 `rg`，不存在即失败。程序与 ripgrep 许可证一起打包。安装命令见[源码构建](Building.md#桌面打包额外依赖ripgrep)。本次仅核对脚本，没有执行打包或验证发行产物。
