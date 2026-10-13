# Installation and upgrades

[简体中文](../zh-CN/Installation.md) · [Home / 首页](Home.md)

Download a package from [GitHub Releases](https://github.com/zhaiyanqi/vclogg2/releases/latest). Replace `<version>` in examples with the downloaded version.

| Platform | Package and installation |
| --- | --- |
| Windows x64 | Extract `vclogg-<version>-windows-x86_64-portable.zip` and run `vclogg2.exe`, or run `vclogg-<version>-windows-x86_64-setup.exe` |
| macOS Apple Silicon | Open `vclogg2-<version>-macos-aarch64.dmg` and drag VCLogg2.app to Applications |
| Linux x86_64 | Extract `vclogg2-<version>-linux-x86_64.tar.gz` and run `./Install-VCLogg2-linux.sh --launch` inside it |

Windows installation is per user. The default application directory is `%LOCALAPPDATA%\Programs\VCLogg2`; the separately selectable data directory defaults to `%LOCALAPPDATA%\VCLogg2`. Portable builds store data in `VCLogg2` next to the executable. The installer creates a Start menu shortcut, optionally a desktop shortcut, and an uninstaller; uninstall preserves data. Windows packages do not register file associations.

Linux installs by default to `~/.local/lib/vclogg2`, creates `~/.local/bin/vclogg2`, and registers desktop/MIME integration. Current release runners are Windows 2022, macOS 15 ARM64 and Ubuntu 22.04 x86_64. macOS packages use ad-hoc signing without notarization. Windows signing depends on the configured release backend; the filename does not prove signing status.

Open files from a terminal with `vclogg2 service.log worker.trace`. A later invocation forwards paths to the most recently active window of the running instance; Windows instance scope also includes the data directory.

Exit the application before upgrading or moving data. Replace portable program files while retaining the data directory; rerun a newer installer for installed Windows/Linux copies, or replace the macOS app bundle. Choosing a new Windows data directory does not migrate old data. Copy the complete data root only after all writers have exited, including SQLite sidecars where present.

**Help → Check for updates** checks the website version manifest and shows release notes on success, with a download-page button. Connection failure opens Releases. Successful results are cached for 24 hours; failed checks use persistent backoff. It does not automatically download or install updates. See [privacy and storage](Data-and-Privacy.md) and [release details](Build-and-Release.md).
