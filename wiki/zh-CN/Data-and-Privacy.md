# 本地数据与隐私

[English](../en/Data-and-Privacy.md) · [Home / 首页](Home.md)

## 日志与应用数据分开

普通浏览与搜索只读；显式进入编辑模式并保存会改写文件，详见[文件编辑](Editing.md)。搜索、标记、文字注释、颜色与会话不修改日志文件；用户显式导出时才写入所选目标。关闭标签、清除收藏和普通历史记录不会删除源文件。

应用会在自己的数据/缓存/临时目录保存状态、索引、云端公开缓存、会话文件、崩溃报告和临时结果。

| 平台 / 安装方式 | 数据位置 |
| --- | --- |
| Windows 便携版及默认 Debug | EXE 同级 `VCLogg2` 目录 |
| Windows 安装版 | 安装时选择的目录，默认 `%LOCALAPPDATA%\VCLogg2` |
| macOS | 系统应用数据、缓存和临时目录；应用数据通常为 `~/Library/Application Support/VCLogg2` |
| Linux | 系统应用数据、缓存和临时目录，随 XDG 环境变化 |

Windows 安装目录中的 `vclogg2-data-dir.txt` 保存绝对数据根。不存在则使用便携位置；存在但无效或不可写则报错退出，不自动换到其他目录。Windows 单实例按当前用户和数据根隔离。

## 保存的内容

- `sessions/vclogg2-state.db`：SQLite 状态库，包含设置、文件路径/历史、搜索记录、标记与恢复状态；WAL/SHM 与主库配套。
- 索引缓存：可重建的行偏移与身份校验信息，生命周期和文件会话分开管理。
- 云端公开目录：按服务器隔离，只覆盖曾浏览的分页，不包含账户会话秘密。
- `cloud/sessions/<服务器地址 SHA-256>.json`：Cookie/CSRF 等会话；内容不额外加密。Unix 目录/文件权限分别限制为 0700/0600，Windows 遵循数据根权限。
- 崩溃报告：包含版本、平台、线程、源码位置与 Rust panic 回溯，正常启动清理到最近 20 份。
- 应用日志：有界内存缓冲，最多 20000 条、8 MiB；按用户选择显式导出，不自动创建长期日志文件。

## 备份、清理与搬迁

备份或搬迁前退出所有使用该数据根的窗口和进程，使会话写入完成。需要完整恢复时复制整套数据目录，避免只取正在使用中的 SQLite 主库而遗漏 WAL。

存储设置用于查看库与索引占用、揭示目录和清理可重建缓存。历史管理保护正在打开、已收藏或有注释/标记的记录。临时结果回收只针对应用创建的文件，并检查是否仍被其他窗口使用。

## 分享诊断材料

先检查导出日志、崩溃报告、路径、搜索词和过滤器中是否含业务数据。不要公开原始状态库、云端会话、Cookie、CSRF 或发布凭据。需要复现时优先构造脱敏的最小日志片段。

云端操作及离线边界见[云端过滤器](Cloud-Filters.md)，平台安装见[安装](Installation.md)。


源码依据：[app_paths.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-app/src/app_paths.rs) · [state_repository.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-data/src/state_repository.rs) · [app_log.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-app/src/app_log.rs) · [crash_report.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-app/src/crash_report.rs)。

检查更新仅在用户点击时访问官网，版本信息、成功时间和失败退避状态保存在 SQLite，不发送日志内容。成功后显示更新说明和下载入口，失败时在浏览器打开 GitHub Releases，详见[构建与发布](Build-and-Release.md)。
