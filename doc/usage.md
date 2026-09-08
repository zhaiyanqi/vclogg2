# 使用指南

[项目首页](../README.md) · [中文 README](README.zh-CN.md) · [文档索引](README.md)

以下命令均在仓库根目录执行；发行包安装命令在解压后的目录执行。

## 快速开始

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

## 工作方式

| 能力 | 行为 |
| --- | --- |
| 多关键词 | 使用 `\|` 分隔普通关键词，例如 `error\|timeout\|retry` |
| 正则与大小写 | 可在搜索栏切换；无效正则不会替换上一份有效结果 |
| 搜索范围 | 当前标签、选定的已打开标签或目录；跨文件搜索并发执行 |
| 结果模式 | 标记与匹配、仅匹配、仅标记；空查询可用于只查看标记 |
| 导航与恢复 | 结果重组后按稳定文件与源行恢复选择和视口，失效锚点回退到最近结果 |
| 结果导出 | 流式导出当前或跨文件结果，支持分组输出和按日志时间戳稳定合并 |

搜索完成后统一显示正式结果，搜索期间可取消。新的搜索、重新加载或关闭文档会使旧扫描失效，迟到结果不会覆盖当前视图。

仅在窗口处于前台时监测当前查看的日志，通过系统通知刷新正文、当前文件搜索及已有全局搜索中该文件的结果；切到后台停止监听和轮询，回到前台或切换标签立即检查一次。连续通知按 400 ms 合并，每 30 秒只复核当前文件。自动刷新使用首尾快速校验，未采样中部改写并继续追加可能漏检，F5 可完整重建。“末尾跟随”仅控制正文是否滚到最新一行。持续追加、半行续写、截断和轮转的处理方式及刷新延迟边界见[动态日志文件](dynamic-log-files.md)。

## 本地数据与隐私

- 源日志始终作为只读输入；只有用户显式导出结果时才写入所选位置。
- Windows 便携版把状态库、索引缓存、崩溃报告和应用临时结果保存在可执行文件同级的 `VCLogg2` 目录；安装版全部保存在安装时选择的数据目录，默认 `%LOCALAPPDATA%\VCLogg2`。安装目录内的 `vclogg2-data-dir.txt` 记录数据目录，请保留该文件。两种版本按当前用户和数据目录隔离单实例；macOS/Linux 使用各自的系统应用数据、缓存和临时目录。
- 会话状态使用 SQLite/WAL；索引缓存与会话身份分离，失效时会安全重建。
- 云端连接的公开配置写入 SQLite，Cookie 与 CSRF 保存为应用数据目录下的 `cloud/sessions/<服务器地址 SHA-256>.json`，不使用系统钥匙串。macOS 默认路径为 `~/Library/Application Support/VCLogg2/cloud/sessions/`；会话文件内容不额外加密，macOS/Linux 目录权限为 0700、文件权限为 0600，Windows 遵循所选数据根的访问权限。
- 应用日志使用有界内存缓冲，不会自行创建长期日志文件。

## 性能诊断

使用 Python 3 生成 50 MiB、100 MiB 和 500 MiB 测试日志：

```bash
python3 scripts/generate-test-data.py
```

使用 Python 3 持续追加随机日志，观察文件增长时的行为：

```bash
python3 scripts/generate-live-log.py --output target/test-data/live.log --min-lines 5 --max-lines 30 --interval 1
```

默认每秒追加 1～20 行到 `target/test-data/live.log`，每批写入后刷新，按 Ctrl+C 停止。文件不存在时自动创建，已存在时保留内容并追加。可加 `--total-lines 1000` 在本次追加 1000 行后退出；`--seed 42` 固定随机内容和批次行数，时间戳仍为当前 UTC 时间。日志中的 `line` 从本次运行的 1 开始计数。使用 `--help` 查看全部参数。

Debug 构建默认记录超过 16 ms 的已标记 UI 渲染作用域。需要隔离应用数据、缓存和构建产物时，在 Windows 使用：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/run-performance-debug.ps1 `
  -WarnAfterMilliseconds 8 `
  -RepeatAfterMilliseconds 1000 `
  -Paths D:\logs\service.log,D:\logs\worker.log
```

诊断器能力、受控 A/B 数据和验证边界见[性能交付说明](PERFORMANCE_DELIVERY_2026-08-31.md)。
