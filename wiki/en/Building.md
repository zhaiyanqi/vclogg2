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
