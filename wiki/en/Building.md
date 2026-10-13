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

Quit all VCLogg2 instances, then launch an existing executable (this command does not build):

```sh
./target/release/vclogg2 --trace-startup ./startup-trace.json --trace-duration 15
```

Use `./target/release/vclogg2.exe` on Windows. Debug executables support the same options. `--trace-startup` selects the JSON output file; its parent directory must exist. `--trace-duration` accepts an integer from 1 to 600 seconds and defaults to 15. Append log paths to open, quoting paths with spaces; everything after `--` is treated as a file path.

Capture starts before platform, font, storage and window initialization. An independent collector stops at the deadline and atomically writes the file, replacing an existing destination. Export does not require a responsive UI. An early return from `main` flushes sooner; forced termination or an aborting crash may prevent export. If another instance is running, normal request forwarding still applies and cannot capture that existing process's startup.

Open the JSON in [Perfetto UI](https://ui.perfetto.dev/). Start with `Startup.*`, then inspect `Document.*`, `Search.*`, `Tab.*`, `Viewport.*`, `Settings.apply` and existing control timings. Application scopes still running at the capture boundary are clipped and marked `incomplete=true`. GPUI task polls, input, actions, drawing, presentation, frame latency and skipped-frame markers are included. Task locations identify spawn sites, not sampled call stacks. Latency markers and nested slices are not additive CPU time; uninstrumented internals may require further investigation.

The capture retains up to 200,000 completed events plus 4,096 active scopes. `Recording summary.lost_events` reports buffer overwrites, contention and capacity loss. During cold-start capture, Settings → Advanced shows automatic recording status and disables duplicate starts. Manual capture continues to use Start recording / Stop and export trace.

UI tracks in exported traces start with `GPUI UI / Main`; other threads start with `Background`. Both manual and cold-start capture retain the actual main-thread Rust ThreadId rather than guessing from names or the export worker. Each event carries `thread_role` / `source_thread`, and `Recording summary.ui_thread` identifies the foreground thread. Each actual Rust thread maps to exactly one display track and one JSON tid; categories remain in event cat fields. The tid is an export mapping, not an OS thread ID. Ordinary slices nest on that track; crossing intervals are split and continued at boundaries, preserving source_event, continued, split and original_duration_us. Frame latency is an instant marker on the same track with elapsed_us. Export is bounded to 404,096 event segments; pending work beyond this limit is reported in omitted_export_segments and lost_events.

Foreground tasks share the UI thread with input, layout and drawing; `cx.spawn` does not move work to a background worker. Inspect long Draw, Application, GPUI tasks and input slices under `GPUI UI / Main`, then correlate frame latency. Durations are elapsed wall time, including blocking waits, not CPU samples; gaps alone do not prove idle time or a stall. Background CPU/lock/memory contention, GPU work and presentation delays can still produce visible stutter. This trace does not capture OS scheduling or GPU execution time.

## Performance overlay and console logging

The former slow-task warnings, rendering contributor logs and dedicated monitoring thread have been removed. Debug and Release use on-demand trace scopes; ordinary application error logging is unchanged. `VCLOGG2_UI_PERF_WARN_MS` / `VCLOGG2_UI_PERF_REPEAT_MS` no longer apply.

The isolated Windows Debug launcher remains available: `powershell -ExecutionPolicy Bypass -File scripts/run-performance-debug.ps1`. It runs `cargo run -p vclogg2 --locked --target-dir target/perf-debug`, setting `RUST_BACKTRACE=full` and `VCLOGG2_DEV_DATA_DIR` (default `target/perf-debug/local-app-data`, overridable with `-DataDirectory`). It no longer accepts `-WarnAfterMilliseconds` / `-RepeatAfterMilliseconds`. Enable the overlay or record traces in Settings → Advanced. This command builds and launches the app.

In the pinned dependency, overlay `MAX` is the maximum elapsed time in milliseconds between `begin_draw` and `end_draw` across the current window's latest 1000 draw samples (or fewer). This measures the UI-thread Draw phase, not the longest task across the entire main thread, frame intervals or GPU execution time. Stalls in input handling, tasks or presentation outside Draw may not appear in this value; inspect the trace as well.
