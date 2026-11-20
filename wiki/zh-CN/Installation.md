# 选择安装包与升级方式

[English](../en/Installation.md) · [指南首页](Home.md)

在发行页展开 **Assets**，按下表选择桌面端文件。文件名里的 `<version>` 是该次发行的版本号。[下载 VCLogg2](https://github.com/zhaiyanqi/vclogg2/releases/latest)

| 使用环境 | 下载哪个文件 | 第一次打开 |
| --- | --- | --- |
| Windows x64，想从开始菜单启动 | `vclogg-<version>-windows-x86_64-setup.exe` | 运行向导，选好程序目录和数据目录；安装后从开始菜单打开。 |
| Windows x64，想解压即用 | `vclogg-<version>-windows-x86_64-portable.zip` | 解压整个压缩包，双击其中的 `vclogg2.exe`。 |
| macOS，Apple 芯片 | `vclogg2-<version>-macos-aarch64.dmg` | 打开 DMG，将 VCLogg2.app 拖入「应用程序」，再从那里启动。 |
| Linux x86_64 | `vclogg2-<version>-linux-x86_64.tar.gz` | 解压后在包含安装脚本的目录中运行下方命令。 |

```sh
./Install-VCLogg2-linux.sh --launch
```

Linux 安装到 `~/.local/lib/vclogg2`，同时创建桌面入口和 `~/.local/bin/vclogg2`。以后在日志所在目录可运行 `~/.local/bin/vclogg2 service.log worker.trace`，一次打开两份文件。看到空工作区就可以开始练习。[用五行日志排查超时](Getting-Started.md)

## Windows 两种版本怎样保存数据？

安装版按当前用户安装。默认程序目录是 `%LOCALAPPDATA%\Programs\VCLogg2`，数据目录是 `%LOCALAPPDATA%\VCLogg2`。便携版将数据放在可执行文件旁的 `VCLogg2` 子目录，解压位置需要可写。想在资源管理器中右键打开日志，可在应用设置里启用文件右键菜单。[打开文件与文件夹](Opening-Files.md)

## 检查更新与升级

选择 **帮助 → 检查更新** 查看版本说明和下载入口。联网失败时，窗口会标明缓存结果的时间；没有可用缓存时会打开发行页。安装包的来源与系统签名提示见[签名政策](Code-Signing-Policy.md)。

退出应用后再升级。Windows 安装版重新运行新版向导；便携版更新解压出的程序文件，保留旁边的 `VCLogg2` 数据目录。macOS 替换「应用程序」中的应用包，Linux 运行新版包里的安装脚本。Windows 卸载保留数据，重新指定数据目录时需自行搬迁。要带走历史、标记和设置，先关闭所有使用同一数据目录的实例，再复制整个数据目录。[备份与搬迁](Data-and-Privacy.md)
