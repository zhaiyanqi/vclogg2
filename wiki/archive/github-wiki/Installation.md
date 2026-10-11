# 安装与升级

从[GitHub Releases](https://github.com/zhaiyanqi/vclogg2/releases/latest)选择适合平台与架构的包。以下 `<version>` 是版本占位符，执行命令前请替换为下载包中的实际版本。


### Windows

Windows x64 同时提供便携版和安装版：

- **便携版**：下载 `vclogg-<version>-windows-x86_64-portable.zip`，解压后运行 `vclogg2.exe`；数据保存在 EXE 同级的 `VCLogg2` 目录。
- **安装版**：下载并运行 `vclogg-<version>-windows-x86_64-setup.exe`。安装向导可分别选择安装目录和数据目录；默认安装到 `%LOCALAPPDATA%\Programs\VCLogg2`，数据默认保存到 `%LOCALAPPDATA%\VCLogg2`。按当前用户安装，无需管理员权限，所选目录须对当前用户可写。

便携版启动命令：

```powershell
.\vclogg2.exe
```

安装版创建开始菜单入口，可选创建桌面快捷方式，并提供系统卸载入口。覆盖升级默认沿用原安装目录和数据目录；选择其他数据目录不会自动搬迁旧数据，卸载也保留数据。便携版不创建快捷方式。两种版本均不注册文件关联，应用不会自动下载或安装更新；便携版手动替换 EXE，安装版重新运行新版安装包。

产物名称不区分签名状态；未配置 Windows 签名后端时，两种包均未签名。启用 PFX 或 Artifact Signing 时，发布流程签署程序和安装包并验证签名。

### macOS

下载 `vclogg2-<version>-macos-aarch64.dmg`，打开镜像后将 `VCLogg2.app` 拖入 `Applications`。当前 Actions 产物使用临时签名，未使用 Apple Developer ID 签名或公证，首次打开时系统可能要求确认。

### Linux

下载 `vclogg2-<version>-linux-x86_64.tar.gz` 后执行：

```bash
tar -xzf vclogg2-<version>-linux-x86_64.tar.gz
cd vclogg2-<version>-linux-x86_64
./Install-VCLogg2-linux.sh --launch
```

安装器默认写入 `~/.local/lib/vclogg2`，创建 `~/.local/bin/vclogg2` 入口，并注册桌面应用和支持的 MIME 类型。

### 从命令行打开日志

```text
vclogg2 <service.log> <worker.trace>
```

如果 VCLogg2 已在运行，路径会按参数顺序交给现有进程，并在最近激活的窗口中打开。


## 升级与保留数据

升级前正常退出正在使用该数据目录的程序。Windows 便携版替换程序文件并保留 `VCLogg2` 数据目录；安装版重新运行新版安装包，默认沿用旧路径。macOS 替换 Applications 下的应用包；Linux 重新运行新版包内的安装脚本。

Windows 安装版更改数据目录不会自动搬迁旧数据，卸载保留数据。需要搬迁时，先退出应用，再完整复制所选数据根；不要只复制一个正在打开的 SQLite 主库文件而遗漏 WAL。

“帮助 → 检查更新”只打开 Releases 页面。确认实际下载包的版本与签名，不要用文件名推断签名状态。

下一步：[快速上手](https://github.com/zhaiyanqi/vclogg2/wiki/Getting-Started) · [数据与隐私](https://github.com/zhaiyanqi/vclogg2/wiki/Data-and-Privacy) · [排障](https://github.com/zhaiyanqi/vclogg2/wiki/Troubleshooting)。


源码依据：[package-release.ps1](https://github.com/zhaiyanqi/vclogg2/blob/65fa1d4fa7f44984825fa84758515a03d2157aeb/scripts/package-release.ps1) · [windows-installer.iss](https://github.com/zhaiyanqi/vclogg2/blob/65fa1d4fa7f44984825fa84758515a03d2157aeb/scripts/windows-installer.iss) · [package-release-macos.sh](https://github.com/zhaiyanqi/vclogg2/blob/65fa1d4fa7f44984825fa84758515a03d2157aeb/scripts/package-release-macos.sh) · [Install-VCLogg2-linux.sh](https://github.com/zhaiyanqi/vclogg2/blob/65fa1d4fa7f44984825fa84758515a03d2157aeb/scripts/Install-VCLogg2-linux.sh)。
