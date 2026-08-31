# 交付说明

## 可直接启动的开发环境

在仓库根目录执行：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/run-debug.ps1
```

脚本使用锁定的 `Cargo.lock` 启动 `vclogg2`。首次执行需要 GitHub 网络访问来解析 GPUI 与 gpui-component；后续可复用 Cargo 缓存。

## Windows Release 产物

更新打包不需要签名密钥。在仓库根目录执行：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-release.ps1
```

脚本先完成锁定依赖的 Release 构建，再在 `dist/` 生成：

- `vclogg2-<version>-win-x64/`：可直接运行的便携目录；
- `vclogg2-<version>-win-x64.zip`：分发压缩包；
- `vclogg2-<version>-win-x64-symbols.zip`：仅供崩溃分析留存的匹配 PDB，不向用户分发；
- `latest.json`：版本、架构、文件大小和整包 SHA-256；
- `vclogg2-<version>-win-x64.blockmap.json`：1 MiB 分块 SHA-256 清单，供客户端下载时逐块校验。

便携目录包含 `vclogg2.exe`、安装辅助、README 和 Apache 2.0 许可证。PDB 使用最小行号调试信息生成并独立压缩；发布侧应按版本保留符号包，但 `publish-update.ps1` 不会把它复制到用户更新目录。安装新版时，安装脚本会清理旧版本曾安装的 `vclogg2.pdb`，避免过期符号继续占用空间或与新版 EXE 错配。应用日志使用有界内存缓冲，不会自行创建长期日志文件；需要诊断材料时由用户在“设置 → 高级”中显式导出。

把更新产物发布到服务端静态目录时执行：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/publish-update.ps1 `
  -TargetDirectory D:\log-viewer-server\data\updates-vclogg2\win-x64
```

脚本会重新核对压缩包大小和 SHA-256，先复制版本化包与 blockmap，最后替换 `latest.json`。服务端把该目录暴露为配置服务器根地址下的 `/updates-vclogg2/win-x64/`，默认对应 `data/updates-vclogg2/win-x64/`。

解压后可直接运行 `vclogg2.exe`。需要固定安装位置和开始菜单入口时，在解压目录执行：

```powershell
powershell -ExecutionPolicy Bypass -File .\Install-VCLogg2.ps1
```

默认安装到 `%LOCALAPPDATA%\Programs\VCLogg2`；可通过 `-InstallDirectory` 指定其他目录，通过 `-Launch` 在安装后立即启动。安装脚本还会在 `HKCU` 注册 VCLogg2 的应用能力、ProgID 与六种受支持扩展名的“打开方式”候选，并通知 Windows Shell 刷新关联；它不会写入 `UserChoice` 或自动替换默认应用。安装后可在日志文件“属性 → 打开方式 → 更改”中由用户选择 VCLogg2。

可执行文件支持从终端接收一个或多个绝对/相对路径：

```powershell
vclogg2.exe .\service.log D:\logs\worker.trace
```

现有实例会接管这些路径并保持参数顺序。

## 验证边界

- `scripts/check.ps1`：工作区格式检查、编译检查和 Clippy；
- `scripts/build-debug.ps1`：Debug 二进制；
- `scripts/build-release.ps1`：优化后的 Release 二进制；
- `scripts/package-release.ps1`：Release 构建加 Windows 便携分发包与更新交接元数据。
- `scripts/publish-update.ps1`：验证并发布包、分块清单和最后生效的清单。

应用内更新仅在 Release 构建启用：启动 15 秒后对已配置服务器检查一次，也可从工具栏手动触发。客户端直接读取更新清单，下载并校验分块与整包 SHA-256；用户确认安装后，独立助手等待应用完成正常退出和多窗口状态保存，再更新当前目录并重启。清单与哈希不做密钥认证，因此更新源的访问控制与 HTTPS 安全由部署环境负责。
