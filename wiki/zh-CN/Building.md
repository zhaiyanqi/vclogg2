# 从源码构建

[English](../en/Building.md) · [Home / 首页](Home.md)

下面的命令是给开发者手动执行的完整路径；本次文档整理没有执行编译、测试或启动应用。除发行包安装命令外，均在主项目仓库根目录运行，不要在 wiki/ 文档目录内执行构建命令。

## 获取源码和 Rust

先安装 Git，并从 [Rust 官方安装页](https://www.rust-lang.org/tools/install)安装 rustup。Windows 使用 MSVC 工具链；macOS/Linux 使用本机原生工具链。

```bash
git clone https://github.com/zhaiyanqi/vclogg2.git
cd vclogg2
rustup toolchain install stable --profile minimal --component rustfmt --component clippy
rustc --version
cargo --version
```

仓库 `rust-toolchain.toml` 自动选择 stable；初次下载依赖需要可访问 crates.io 与所用源码地址。当前 UI 入口为精确版本 GPUI Kit 0.7.1。三平台共用 `Cargo.lock`，不要为普通构建删除锁文件或运行无关的依赖升级。

## Windows

安装 Visual Studio 2022 Build Tools，选中“使用 C++ 的桌面开发”和 Windows SDK。使用 Rust 官方 `rustup-init.exe` 安装适合本机的工具链；当前发布目标为 x86_64 MSVC。

```powershell
rustup toolchain install stable-x86_64-pc-windows-msvc --profile minimal --component rustfmt --component clippy
powershell -ExecutionPolicy Bypass -File scripts/build-debug.ps1
.\target\debug\vclogg2.exe
```

也可使用 `powershell -ExecutionPolicy Bypass -File scripts/run-debug.ps1`，该脚本执行 `cargo run -p vclogg2 --locked`。Release：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build-release.ps1
```

生成安装包还需要 Inno Setup 6.3 或更新的 6.x 版本，详见[构建与发布](Build-and-Release.md)。

## macOS

macOS 构建会将 `crates/vclogg-app/resources/macos/Info.plist` 的中英文语言声明嵌入可执行文件，发行打包复用同一模板；无需额外工具或构建参数。系统窗口菜单在启动时读取已保存的应用语言，切换后需保存并重启。

安装 Xcode 与 Command Line Tools。以下配置示例假定 Xcode 位于标准位置：

```bash
xcode-select --install
sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
xcodebuild -version
./scripts/build-debug.sh
./target/debug/vclogg2
```

已安装 Command Line Tools 时无需重复安装。若 Xcode 首次启动仍有组件或许可待处理，先在 Xcode 中完成。当前 Actions 使用 macOS 15 ARM64；其他系统版本需自行验证。

Release 构建：

```bash
./scripts/build-release.sh
```

## Linux（Ubuntu / Debian）

先安装与 Ubuntu 22.04 Actions 一致的原生依赖：

```bash
sudo apt-get update
sudo apt-get install --yes --no-install-recommends \
  build-essential clang cmake libfontconfig-dev libglib2.0-dev libssl-dev \
  libvulkan1 libwayland-dev libx11-dev libx11-xcb-dev libxcb1-dev \
  libxkbcommon-x11-dev pkg-config
./scripts/build-debug.sh
./target/debug/vclogg2
```

Release 使用 `./scripts/build-release.sh`。运行需要可用的图形桌面和兼容的渲染驱动；编译成功本身不证明虚拟机或远程桌面的 GPU 路径正常。

## 脚本对应关系

| 脚本（.sh / .ps1） | 实际工作 |
| --- | --- |
| build-debug | `cargo build --workspace --locked` |
| build-release | `cargo build --workspace --release --locked` |
| check | fmt 检查 + `cargo check --workspace --all-targets --locked` + 严格 Clippy |
| check-architecture.sh | 检查 crate 依赖方向与模块结构，不编译 |

Debug 二进制在 `target/debug/`，Release 在 `target/release/`，文件名为 `vclogg2`（Windows 带 `.exe`）。Bash 构建脚本会定位仓库；PowerShell 脚本依赖当前目录，因此统一建议从仓库根运行。

## 验证层级

仅检查格式与结构：

```bash
cargo fmt --all -- --check
git diff --check
bash scripts/check-architecture.sh
```

编译分析与 Clippy：

```bash
./scripts/check.sh
```

Windows 使用 `powershell -ExecutionPolicy Bypass -File scripts/check.ps1`。这些检查会编译分析代码，可能构建依赖，不等于“仅文本检查”。显式运行自动测试：

```bash
cargo test --workspace --locked
```

测试、Clippy、二进制构建、实际窗口验收及目标平台打包验证是不同层级，记录结果时分别说明。打包、标签与 Release 发布见[构建与发布](Build-and-Release.md)。


源码依据：[rust-toolchain.toml](https://github.com/zhaiyanqi/vclogg2/blob/main/rust-toolchain.toml) · [build-debug.sh](https://github.com/zhaiyanqi/vclogg2/blob/main/scripts/build-debug.sh) · [build-release.sh](https://github.com/zhaiyanqi/vclogg2/blob/main/scripts/build-release.sh) · [check.sh](https://github.com/zhaiyanqi/vclogg2/blob/main/scripts/check.sh) · [ci.yml](https://github.com/zhaiyanqi/vclogg2/blob/main/.github/workflows/ci.yml)。


## 云端过滤器服务端

服务端在同一源码仓库的 `server/` 下，使用独立 `go.mod` 与 `pnpm-lock.yaml`；无需 Rust/GPUI 或桌面平台库。需要 Go 1.24+、Node.js 22.12+（CI 使用 24）、pnpm 11.9.0、tar，以及 macOS/Linux 的 zip。

macOS（已安装 Homebrew）：

```bash
brew install go node@24 zip
export PATH="$(brew --prefix node@24)/bin:$PATH"
npm install --global pnpm@11.9.0
go version
node --version
./scripts/package-server-release.sh
```

Windows（已安装 winget；安装 Go/Node 后重新打开终端以载入 PATH）：

```powershell
winget install --exact --id GoLang.Go
winget install --exact --id OpenJS.NodeJS.LTS
# 在新终端执行以下命令；Windows 10/11 自带 tar 和 PowerShell。
npm install --global pnpm@11.9.0
go version
node --version
powershell -ExecutionPolicy Bypass -File scripts/package-server-release.ps1
```

Linux x86_64（Ubuntu；按官方版本接口安装工具链到用户目录，不替换系统工具）：

```bash
sudo apt-get update
sudo apt-get install --yes ca-certificates curl python3 tar xz-utils zip
server_tools="$HOME/.local/vclogg-server-tools"
mkdir -p "$server_tools"
# 下载 Go 官方当前稳定版与 SHA-256。
read -r server_go_archive server_go_sha < <(
  curl -fsSL 'https://go.dev/dl/?mode=json' | python3 -c 'import json,sys; releases=json.load(sys.stdin); f=next(f for r in releases if r["stable"] for f in r["files"] if f["os"]=="linux" and f["arch"]=="amd64" and f["kind"]=="archive"); print(f["filename"],f["sha256"])'
)
curl -fL "https://go.dev/dl/$server_go_archive" -o "$server_tools/$server_go_archive"
printf '%s  %s\n' "$server_go_sha" "$server_tools/$server_go_archive" | sha256sum --check
mkdir -p "$server_tools/$server_go_archive.extracted"
tar -xzf "$server_tools/$server_go_archive" -C "$server_tools/$server_go_archive.extracted"
# Node.js 24 的官方 Linux x64 包和对应摘要。
curl -fsSL https://nodejs.org/dist/latest-v24.x/SHASUMS256.txt -o "$server_tools/node-SHASUMS256.txt"
read -r server_node_sha server_node_archive < <(awk '$2 ~ /-linux-x64.tar.xz$/ {print $1, $2}' "$server_tools/node-SHASUMS256.txt")
curl -fL "https://nodejs.org/dist/latest-v24.x/$server_node_archive" -o "$server_tools/$server_node_archive"
printf '%s  %s\n' "$server_node_sha" "$server_tools/$server_node_archive" | sha256sum --check
tar -xJf "$server_tools/$server_node_archive" -C "$server_tools"
export PATH="$server_tools/$server_go_archive.extracted/go/bin:$server_tools/${server_node_archive%.tar.xz}/bin:$PATH"
npm install --global pnpm@11.9.0
go version
node --version
./scripts/package-server-release.sh
```

打包脚本自动在 `server/` 执行 `pnpm install --frozen-lockfile`、`pnpm run typecheck`、`pnpm run build:admin`，再执行 `go build -mod=readonly -trimpath -ldflags '-s -w'`，分别设置 `GOOS=windows/linux`、`GOARCH=amd64`、`CGO_ENABLED=0`。不需要 C 编译器。参数、包名和可选公钥说明见 [Build-and-Release](Build-and-Release.md)。

独立校验（仅需校验时主动执行）：

```bash
pnpm --dir server install --frozen-lockfile
pnpm --dir server run typecheck
go -C server vet ./...
go -C server test ./...
```

Windows 也保留 `powershell -ExecutionPolicy Bypass -File server/scripts/verify.ps1`，包含内嵌资源一致性检查和真实构建。这些命令均非静态检查；本次迁移未执行编译、测试或真实启动。

## 桌面打包额外依赖：ripgrep

当前三个桌面打包脚本仍将 `rg` 和许可证放入发行包。即使当前版本已移除 AI，打包前仍需安装；单独 build-debug/build-release 不检查此工具。macOS/Linux 从 PATH 查找 `rg`，Windows 从 PATH 查找 `rg.exe`。

```bash
# macOS，已安装 Homebrew
brew install ripgrep
# Ubuntu / Debian
sudo apt-get install --yes ripgrep
```

```powershell
# 已安装 Chocolatey，与 Windows CI 的安装方式一致
choco install ripgrep --yes --no-progress
# 安装后重新打开终端，使 PATH 生效
rg --version
```

## 冷启动性能录制

先退出所有 VCLogg2 实例，再运行已有的可执行文件（以下命令不会编译）：

```sh
./target/release/vclogg2 --trace-startup ./startup-trace.json --trace-duration 15
```

Windows 使用 `./target/release/vclogg2.exe`；Debug 可执行文件同样支持这些参数。`--trace-startup` 指定 JSON 文件，父目录必须已存在；`--trace-duration` 为 1–600 的整数秒，省略时为 15 秒。可在参数后追加要打开的日志路径，含空格时加引号；以 `--` 分隔后，后续参数均作为文件路径。

录制在平台、字体、状态库和窗口初始化前开启。独立采集线程到时自动停止并原子写入指定文件（会替换同名文件），无需在界面点击停止；UI 卡住时也不依赖 UI 线程来触发导出。`main` 提前返回会尽早导出，强制结束进程或崩溃中止不能保证落盘。若已有实例运行，本次启动仍按原规则转发打开请求，无法录制已有进程的启动。

使用 [Perfetto UI](https://ui.perfetto.dev/) 打开 JSON。先查看 `Startup.*` 初始化节点，再看 `Document.*`、`Search.*`、`Tab.*`、`Viewport.*`、`Settings.apply` 及已有控件耗时；停止时仍未结束的应用节点会裁剪到录制边界，带 `incomplete=true`。同时导出 GPUI 任务 poll、输入、Action、Draw、Present、帧等待与跳帧标记。任务位置是 spawn 位置，不是采样调用栈；帧等待和嵌套事件不能相加作为 CPU 耗时，未埋点内部逻辑仍需继续定位。

最多保留 200,000 条已完成事件和 4,096 个活动 scope；`Recording summary` 中的 `lost_events` 表示缓冲覆盖、竞争或容量超限造成的丢失。冷启动录制期间“设置 → 高级”会显示自动录制状态并禁用重复开始。手动录制仍使用“开始录制 / 停止并导出 trace”。

导出的 UI 轨道统一以 `GPUI UI / Main` 开头，其他线程以 `Background` 开头；手动录制与冷启动录制均保存实际主线程的 Rust ThreadId，不按线程名称或导出线程猜测。每个事件的 `thread_role` / `source_thread` 和 `Recording summary.ui_thread` 可用于核对归属。每个实际 Rust 线程只对应一个显示轨道和一个 JSON tid，类别仅保留在事件 cat 中；tid 是导出映射编号，不是系统线程 ID。普通事件在同轨道嵌套；交叉区间在边界处分段衔接，source_event、continued、split 和 original_duration_us 可用于还原同一原始事件。帧延迟使用同轨道瞬时标记，elapsed_us 保留测量时长。导出最多 404,096 个事件片段，超过上限的待导出部分计入 omitted_export_segments 和 lost_events。

GPUI 的前台任务与输入、布局、绘制共享 UI 线程，`cx.spawn` 不会自动移到后台。优先检查 `GPUI UI / Main` 下的 Draw、Application、GPUI tasks 和输入长区间，再关联帧延迟。区间记录经过时间，包含阻塞等待，不是 CPU 采样；空白区间也不能单独证明 UI 空闲或卡顿。后台 CPU/锁/内存争用、GPU 或呈现延迟仍可能造成可感知卡顿；本 trace 不含系统调度及 GPU 执行时间。

## 性能叠加层与控制台日志

旧的性能超时警告、渲染阶段耗时明细和专用检测线程已移除；Debug / Release 均通过按需 trace 记录控件耗时，普通业务错误日志保持原有行为。`VCLOGG2_UI_PERF_WARN_MS` / `VCLOGG2_UI_PERF_REPEAT_MS` 不再生效。

Windows 的隔离 Debug 启动脚本仍可用：`powershell -ExecutionPolicy Bypass -File scripts/run-performance-debug.ps1`。它执行 `cargo run -p vclogg2 --locked --target-dir target/perf-debug`，设置 `RUST_BACKTRACE=full` 和 `VCLOGG2_DEV_DATA_DIR`（默认 `target/perf-debug/local-app-data`，可通过 `-DataDirectory` 覆盖）；不再接受 `-WarnAfterMilliseconds` / `-RepeatAfterMilliseconds`，使用“设置 → 高级”开启叠加层或录制 trace。此命令会编译并启动应用。

当前依赖的叠加层 `MAX` 取当前窗口最近最多 1000 次 `begin_draw` 到 `end_draw` 的最大经过时间（毫秒），对应 UI 线程上的 Draw 阶段，不是全主线程任务最大耗时、帧间隔或 GPU 执行时间。Draw 之外的输入处理、任务阻塞和等待呈现等卡顿不一定反映在此值中，应结合 trace 查看。
