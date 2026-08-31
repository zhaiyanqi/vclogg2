# VCLogg2 功能与验收记录

最后核对：2026-08-31

## 文档范围

- 当前 GPUI 详细完成范围以 [`migration-status.md`](migration-status.md) 为准；界面控件和稳定编号以 [`ui-layout.md`](ui-layout.md) 为准。
- 本文记录实现结论、待确认差异和可重复验收步骤，不替代上面两份实现合同。

## 已实现能力总览

| 能力域 | 当前状态 | 主要实现 / 验收证据 |
| --- | --- | --- |
| 文件打开、预览、索引、编码、重新加载与追加跟随 | 已实现 | `crates/vclogg-core/src/document.rs`、`crates/vclogg-app/src/workspace.rs`；安全预览与完整快照原子换入，强身份缓存失败时回退重建 |
| 单实例、多标签、多窗口、拖放、标签复制/移动与窗口生命周期 | 已实现 | `crates/vclogg-app/src/single_instance.rs`、`workspace.rs`；Windows 命名管道及 macOS/Linux Unix socket 启动请求转交，窗口级 `Workspace` / `Root`，稳定文档 ID，最后窗口关闭后退出进程 |
| 正文与搜索结果浏览、换行、导航、多选、文字选择、复制和行菜单 | 已实现 | `crates/vclogg-app/src/log_table.rs`、`global_search_table.rs`、`selectable_log_text.rs`；固定行 DataTable 与可变行高虚拟列表共享领域选择语义 |
| 当前文件/全局搜索、正则、多关键词、补全、页内查找、取消和进度 | 已实现 | `crates/vclogg-core/src/search.rs`、`search_autocomplete.rs`、`workspace.rs`；迟到轮次拒绝、全局结果整轮原子安装 |
| 标记、颜色标签、日志级别、匹配高亮与三种结果模式 | 已实现 | `crates/vclogg-app/src/color_labels.rs` 与三个表格 delegate；按文档 ID + 源行同步正文、当前结果和全局结果 |
| 会话、最近/收藏/上一次文件、历史清理与设置持久化 | 已实现 | `crates/vclogg-app/src/state_store.rs`、`history_dialog.rs`；SQLite/WAL、revision 合并、退出事务化 flush |
| 搜索结果导出、时间戳合并与临时结果回收站 | 已实现 | `crates/vclogg-app/src/result_export.rs`、`trash.rs`；流式输出、同目录原子替换、临时文件隔离 |
| 预定义过滤器与云端过滤器 | 已实现 | `crates/vclogg-app/src/predefined_filters*.rs`、`cloud_filters.rs`、`settings_dialog.rs`；设置页网络配置与 Cookie 连接测试、本地导入导出、无客户端密钥注册、冲突选择和离线只读目录 |
| 三平台文件集成与更新交付 | 主路径已实现 | Windows“打开方式”、macOS 文档类型、Linux desktop/MIME、系统废纸篓、平台包、哈希清单和安装助手均已落地；实际更新服务仍需逐平台端到端验收 |

## 本轮交付

### 单实例与外部打开

- Windows 使用用户级命名管道，macOS/Linux 使用权限受限的每用户 Unix domain socket；三个平台都只保留一个 VCLogg2 进程，并把后续启动请求交给已有进程。
- Finder、桌面环境或文件关联传入的 URL/路径会复用同一外部打开队列；macOS Dock 重新打开事件在没有可见窗口时创建新窗口。
- 再次启动不带文件参数时，已有进程创建一个新的原生窗口；带文件参数时，按最近激活顺序选择目标窗口并激活它。
- 目标窗口正在打开或重载文件时，外部路径在该窗口排队，等当前任务结束后继续普通打开链路。

### 应用退出生命周期

- 应用不创建系统托盘图标，也不允许零窗口后台驻留。
- 关闭非最后窗口时只卸载对应工作区并保存其最终会话，其他窗口继续运行。
- 关闭最后一个 GPUI 窗口后进入唯一正常退出链路，等待已经关闭窗口的保存任务，事务化保存窗口清单和文件会话，然后结束进程。

### 右键菜单上下文收敛

- 正文、当前结果和全局结果的菜单选项被收敛为 `LogContextMenuContext`，不再以多个相邻布尔参数表达不同区域。
- 此项不改变可见控件和行为；它消除了工作区 Clippy 阻断，并降低后续新增菜单能力时把“结果导出”“全局合并”等开关传错位置的风险。

## 待确认差异

| 编号 | 需求线索 | 当前 GPUI 状态 | 需要确认的产品决定 |
| --- | --- | --- | --- |
| D002 | `glass` 主题支持内置/自定义壁纸及填充、适应、拉伸、平铺、居中 | 当前明确只保留跟随系统、浅色、深色；旧 `glass` 值回退浅色，没有壁纸设置 | 是否需要以 GPUI 图片元素和主题 token 重新设计玻璃材质；不建议照搬 CSS `backdrop-filter` 与 Web 壁纸布局 |
| D003 | 独立产品级启动欢迎页、延迟加载工作区和资源压缩 | 当前启动后直接创建 GPUI `Workspace`，空工作区已提供最近、收藏和恢复入口 | 若需要独立欢迎页，应另行确认其视觉、生命周期和性能目标 |
| D004 | “右侧全文件逻辑定位条和搜索命中概览”需求 | 当前界面没有对应控件；GPUI 提供标准/逻辑虚拟滚动条 | 如仍需要，应先确认是滚动缩略图、搜索命中刻度，还是两者组合 |
| D005 | 发行更新能力需要真实服务完成闭环 | 客户端、分块校验、安装助手与发布脚本已实现，更新清单按产品决定不做密钥认证 | 提供更新目录后执行真实端到端验收；部署侧负责更新源访问控制与 HTTPS |

## 验收清单

在当前平台执行自动验收：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\check.ps1
```

```bash
./scripts/check.sh
```

该脚本依次验证格式、整个 workspace 的所有 target 静态检查，以及 `clippy -D warnings`。

单实例与退出生命周期手工验收（项目规则禁止自动启动窗口，本轮未代替用户执行）：

1. 运行 Debug 或 Release 产物，确认系统通知区域没有 VCLogg2 图标。
2. VCLogg2 运行时再次不带参数启动，确认只有一个进程且创建第二个窗口。
3. 依次激活两个窗口，通过文件关联或命令行外部打开日志，确认文件进入最后激活的窗口；连续外部打开时确认路径不会丢失。
4. 关闭其中一个窗口，确认另一个窗口和进程继续运行。
5. 关闭最后一个 VCLogg2 窗口，确认进程结束且系统进程列表中没有后台驻留实例。
6. 重新启动，检查最近文件、收藏与文件会话仍可恢复。

## 历史提交记录

本轮提交按职责拆分：

- `c2b3818 refactor(ui): encapsulate log context menu state`
- `8fd69e0 feat(windows): keep app available from system tray`（托盘行为已由当前产品决定撤销）

本文档提交会作为第三条独立记录保留，以便功能代码、平台行为和审计结论分别回溯。
