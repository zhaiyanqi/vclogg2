# Building from source

[简体中文](../zh-CN/Building.md) · [Home / 首页](Home.md)

Run commands from the main repository root, not `wiki/`. These are developer instructions; documenting them does not mean they were executed during migration.

## Rust and source

Install Git and [rustup](https://www.rust-lang.org/tools/install), then:

```sh
git clone https://github.com/zhaiyanqi/vclogg2.git
cd vclogg2
rustup toolchain install stable --profile minimal --component rustfmt --component clippy
rustc --version
cargo --version
```

The repository selects stable, Rust 2024 and GPUI Kit 0.7.1. Keep the committed Cargo.lock and use locked builds.

## Windows

Install Visual Studio 2022 Build Tools with **Desktop development with C++** and Windows SDK. Use the x64 MSVC Rust toolchain:

```powershell
rustup toolchain install stable-x86_64-pc-windows-msvc --profile minimal --component rustfmt --component clippy
powershell -ExecutionPolicy Bypass -File scripts/build-debug.ps1
.\target\debug\vclogg2.exe
# Release
powershell -ExecutionPolicy Bypass -File scripts/build-release.ps1
```

`scripts/run-debug.ps1` runs `cargo run -p vclogg2 --locked`. Packaging also requires Inno Setup 6.3+ (6.x) and ripgrep:

```powershell
winget install --id JRSoftware.InnoSetup --exact --source winget
# With Chocolatey available, as in Windows CI:
choco install ripgrep --yes --no-progress
```

Reopen the terminal after installation if PATH has changed.

## macOS

Install Xcode and Command Line Tools. For a standard Xcode installation:

```sh
xcode-select --install
sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
xcodebuild -version
./scripts/build-debug.sh
./target/debug/vclogg2
# Release
./scripts/build-release.sh
# Packaging dependency, with Homebrew installed:
brew install ripgrep
```

Complete Xcode's first-run setup/license if required; skip Command Line Tools installation when already present. Actions uses macOS 15 ARM64. `resources/macos/Info.plist` supplies language declarations for both executable and packaged app; native menu language is selected at startup.

## Linux (Ubuntu / Debian)

```sh
sudo apt-get update
sudo apt-get install --yes --no-install-recommends   build-essential clang cmake libfontconfig-dev libglib2.0-dev libssl-dev   libvulkan1 libwayland-dev libx11-dev libx11-xcb-dev libxcb1-dev   libxkbcommon-x11-dev pkg-config ripgrep
./scripts/build-debug.sh
./target/debug/vclogg2
# Release
./scripts/build-release.sh
```

The release runner is Ubuntu 22.04 x86_64. Running requires a graphical desktop and compatible rendering drivers; successful compilation does not validate a VM or remote desktop GPU path.

## Scripts and verification levels

| Script (.sh / .ps1) | Operation |
| --- | --- |
| build-debug | `cargo build --workspace --locked` |
| build-release | `cargo build --workspace --release --locked` |
| check | fmt + `cargo check --workspace --all-targets --locked` + strict Clippy |
| check-architecture.sh | Static crate/module boundary checks, no compilation |

Outputs are `target/debug/vclogg2` or `target/release/vclogg2` (`.exe` on Windows). Bash scripts locate the root; use the root as working directory for PowerShell.

Static checks:

```sh
cargo fmt --all -- --check
git diff --check
bash scripts/check-architecture.sh
```

When compilation/testing is intended, run `./scripts/check.sh` (PowerShell: `scripts/check.ps1`) and separately `cargo test --workspace --locked`. Compilation, automated tests, packaging, performance benchmarks and real-window acceptance are distinct checks.

## Optional server

`server/` has its own Go module and pnpm lockfile. It requires Go 1.24+, Node.js 22.12+ (CI uses 24), pnpm 11.9.0, tar and, on macOS/Linux, zip. It does not require Rust/GPUI.

macOS with Homebrew:

```sh
brew install go node@24 zip
export PATH="$(brew --prefix node@24)/bin:$PATH"
npm install --global pnpm@11.9.0
./scripts/package-server-release.sh
```

Windows with winget (reopen the terminal after Go/Node installation):

```powershell
winget install --exact --id GoLang.Go
winget install --exact --id OpenJS.NodeJS.LTS
npm install --global pnpm@11.9.0
powershell -ExecutionPolicy Bypass -File scripts/package-server-release.ps1
```

Linux: install [Go](https://go.dev/doc/install) and [Node.js 24](https://nodejs.org/en/download) for your architecture, then:

```sh
sudo apt-get update
sudo apt-get install --yes ca-certificates curl python3 tar xz-utils zip
npm install --global pnpm@11.9.0
go version
node --version
./scripts/package-server-release.sh
```

Packaging installs frozen pnpm dependencies, typechecks and rebuilds the embedded admin UI, then builds Windows/Linux amd64 with `CGO_ENABLED=0`, `-mod=readonly`, `-trimpath` and stripped symbols. Optional verification commands are `pnpm --dir server run typecheck`, `go -C server vet ./...` and `go -C server test ./...`; Windows `server/scripts/verify.ps1` also performs real builds/resource consistency checks.

See [release packaging](Build-and-Release.md) and [documentation-only builds](Documentation.md).

## Cold-start performance capture

Quit all VCLogg2 instances and launch an existing executable (this does not build):

```sh
./target/release/vclogg2 --trace-startup --trace-duration 15
```

The Debug profile sets `opt-level = 3` for `gpui-component` and `gpui-base`, matching the already optimized GPUI runtime to reduce unoptimized framework work such as component initialization. Application code keeps its default Debug optimization level. The first rebuild recompiles these dependencies, and optimization may affect stepping through their internals. Release is unchanged.

`Startup.initialize_services` covers service registration only; window creation is recorded separately under `Startup.open_workspace_window` and its child scopes. Older traces include window creation in the service scope, so a shorter scope alone does not demonstrate a performance improvement.

On Unix, startup retries binding immediately after the first stale instance socket cleanup. Repeated cleanups and other connection failures retain the 40 ms backoff and the overall 5-second timeout. `Startup.cleanup_stale_instance_socket` identifies cleanup that actually occurred.

Use `./target/release/vclogg2.exe` on Windows. Debug supports the same options. `--trace-startup` is now a flag with no output-path argument; remove that path from older commands. `--trace-duration` accepts integer seconds from 1 to 600, defaulting to 15. Append `-- service.log` to open logs, quoting paths with spaces. If another instance is running, normal forwarding applies and cannot capture that process's startup.

Manual and cold-start recordings save automatically into the application's trace cache with timestamp-and-UUID filenames. macOS/Linux use `VCLogg2/traces` under the system cache (or the isolated Debug cache root); Windows uses `cache/traces` under the configured data root. Settings → Advanced → Performance recording → Open folder is available during recording.

An independent writer publishes an initial snapshot, then waits one second after each write before saving again; large captures or busy disks extend this interval. It builds complete Chrome Trace Event JSON and atomically replaces the session file, with a final snapshot on stop. There is no save dialog. Live files are complete snapshots, not per-event append streams, and writing does not require a responsive UI. Forced termination leaves the last published snapshot; the latest interval or an unfinished write may be lost. Files are not automatically deleted.

Manual capture uses Start recording / Stop recording. Write errors are shown and retried automatically. Failed final saves retain data for Retry save; Dismiss retry releases memory without deleting existing files. Cold-start capture stops automatically; when Settings is open it also displays write errors and retains failed final data for retry. The terminal reports the final cold-start save result.

Open JSON in [Perfetto UI](https://ui.perfetto.dev/). Inspect `Startup.*`, `Document.*`, `Search.*`, `Tab.*`, `Viewport.*`, `Settings.apply` and control timings. Active application scopes are clipped at the snapshot boundary with `incomplete=true`. Capture retains up to 200,000 completed events and 4,096 active scopes. Once full, snapshots and loss counts keep updating without unbounded memory growth. `Recording summary.recording` distinguishes live from stopped snapshots; `lost_events` reports losses.

Each actual Rust thread has one track: `GPUI UI / Main` or `Background`. Event arguments `thread_role` / `source_thread` retain identity; tids are export mappings, not OS IDs. Crossing intervals continue as segments on the same track, with source_event, continued, split and original_duration_us. Frame latency is an instant with elapsed_us. Export is bounded to 404,096 segments; pending work beyond the limit is included in omitted_export_segments and lost_events.

`cx.spawn` still executes on the UI thread. Inspect long Draw, Application, GPUI tasks and input slices. Durations are elapsed wall time including waits, not CPU samples; task locations are spawn sites, not complete call stacks. Gaps alone do not prove idle time or stalls, and nested slices are not additive CPU time. Background resource contention, GPU work and presentation delays may also cause stutter; OS scheduling and GPU execution are not captured.

## Performance overlay and console logging

The former slow-task warnings, rendering contributor logs and dedicated monitoring thread have been removed. Debug and Release use on-demand trace scopes; ordinary application error logging is unchanged. `VCLOGG2_UI_PERF_WARN_MS` / `VCLOGG2_UI_PERF_REPEAT_MS` no longer apply.

The isolated Windows Debug launcher remains available: `powershell -ExecutionPolicy Bypass -File scripts/run-performance-debug.ps1`. It runs `cargo run -p vclogg2 --locked --target-dir target/perf-debug`, setting `RUST_BACKTRACE=full` and `VCLOGG2_DEV_DATA_DIR` (default `target/perf-debug/local-app-data`, overridable with `-DataDirectory`). It no longer accepts `-WarnAfterMilliseconds` / `-RepeatAfterMilliseconds`. Enable the overlay or record traces in Settings → Advanced. This command builds and launches the app.

In the pinned dependency, overlay `MAX` is the maximum elapsed time in milliseconds between `begin_draw` and `end_draw` across the current window's latest 1000 draw samples (or fewer). This measures the UI-thread Draw phase, not the longest task across the entire main thread, frame intervals or GPU execution time. Stalls in input handling, tasks or presentation outside Draw may not appear in this value; inspect the trace as well.
