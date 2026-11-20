# Choose a package and upgrade

[简体中文](../zh-CN/Installation.md) · [Guide home](Home.md)

Open **Assets** on the release page and choose the desktop package for your machine. In the filenames below, `<version>` stands for the release number. [Download VCLogg2](https://github.com/zhaiyanqi/vclogg2/releases/latest)

| Machine and preference | Download | First launch |
| --- | --- | --- |
| Windows x64, Start menu entry | `vclogg-<version>-windows-x86_64-setup.exe` | Run the installer, choose the program and data directories, then open VCLogg2 from Start. |
| Windows x64, portable copy | `vclogg-<version>-windows-x86_64-portable.zip` | Extract the entire archive and double-click `vclogg2.exe`. |
| macOS on Apple silicon | `vclogg2-<version>-macos-aarch64.dmg` | Open the DMG, drag VCLogg2.app into Applications and launch it from there. |
| Linux x86_64 | `vclogg2-<version>-linux-x86_64.tar.gz` | Extract the archive and run the command below from the directory containing the installer. |

```sh
./Install-VCLogg2-linux.sh --launch
```

The Linux installer writes to `~/.local/lib/vclogg2` and creates a desktop entry and `~/.local/bin/vclogg2`. Later, run `~/.local/bin/vclogg2 service.log worker.trace` from the directory containing your logs to open both together. Once you see the empty workspace, try the sample. [Trace your first timeout](Getting-Started.md)

## Where do the Windows editions keep my data?

Setup installs for the current user. The program defaults to `%LOCALAPPDATA%\Programs\VCLogg2` and the data directory to `%LOCALAPPDATA%\VCLogg2`. Portable copies keep data in a `VCLogg2` directory next to the executable, so extract to a writable folder. For an Explorer context menu entry, enable the file context menu in Settings. [Open files and folders](Opening-Files.md)

## Check for updates and upgrade

Choose **Help → Check for updates** for release notes and downloads. If the network request fails, the app dates any cached result it displays or opens Releases if it has no usable result. For package provenance and system prompts, see the [code signing policy](Code-Signing-Policy.md).

Exit VCLogg2 before replacing program files. On Windows, run the newer installer or update the extracted portable program files while keeping the adjacent `VCLogg2` data directory. On macOS replace the application bundle. On Linux rerun the installer from the new package. Windows uninstall keeps your data. Choosing a different data directory does require moving the data yourself. To keep history, marks and settings, close all instances using that directory and copy the entire directory. [Backup and migration](Data-and-Privacy.md)
