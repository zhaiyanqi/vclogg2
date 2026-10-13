# 架构

[English](../en/Architecture.md) · [Home / 首页](Home.md)

Cargo workspace 将应用呈现、文件搜索算法、持久存储分为三层。`crates/vclogg-app` 构建的包和二进制名为 **vclogg2**，Cargo 选择包时使用 `-p vclogg2`。

```text
crates/vclogg-app (vclogg2)
  ├── vclogg-core：文件、索引、解码、搜索、压缩行集合
  └── vclogg-data：SQLite、状态记录、路径编码、缓存生命周期
server/：独立 Go 模块与 Vue 管理界面
compat/xim-ctext/：旧 XIM 解码接口桥接
vendor/block/：现有 macOS 依赖兼容补丁
wiki/：双语 mdBook 源文档
 doc/：官网与资源
```

app 组合 core/data；core 不依赖 data/app，data 不依赖 app。core/data 不持有 GPUI 实体。`scripts/check-architecture.sh` 静态检查这些边界。

## 能力与源码对应

| 能力 | 源码与职责 |
| --- | --- |
| 窗口组合 | `workspace.rs`、`workspace/render_shell.rs`、`window_registry.rs`：原生窗口、焦点、外部请求、退出快照 |
| 文档生命周期 | `document_opening.rs`、`document_tasks.rs`、`document_lifecycle.rs`：有界准备、安装、重载、恢复 |
| 文件标签 | `tab_lifecycle.rs`、`tab_drag.rs`、`cross_window_tab_drag.rs`、`tab_motion.rs`：激活、跨窗口传输、排序与过渡 |
| 搜索标签 | `search_tabs.rs`、`search_tab_ui.rs`、`search_tab_order.rs`、`search_tab_activation.rs`、`search_tab_tasks.rs`：独立状态、排队、发布 |
| 搜索执行 | `search_orchestration.rs`、`search_limits.rs`、`quick_find.rs`：范围、源行边界、已提交查询与页内查找 |
| 日志呈现 | `log_presentation.rs`、`viewport_orchestration.rs`、`log_viewport.rs`、`virtual_log_list.rs`、`virtual_log_lines.rs`：可见行、源行身份、选择和换行 |
| 设置 | `settings_dialog.rs`、`workspace/settings_window.rs`、`preferences.rs`、`highlight_preferences.rs`：唯一设置窗口、草稿预览和保存 |
| 显式写入 | `document_editing.rs`、`clipboard_paste.rs`、`result_export_flow.rs`：编辑、剪贴板临时文档与结果快照导出 |
| 文件引擎 | core 的 `document.rs`、`document/index_cache.rs`、`document/text_window.rs`：不可变快照、位置读取、解码和行索引 |
| 搜索引擎 | core 的 `search.rs`、`search/cache.rs`、`result_set.rs`、`cancellation.rs`：匹配、缓存、压缩源行、取消 |
| 持久化 | data 的 `state_repository.rs`、`state.rs`、`path_codec.rs`、`index_cache.rs`：事务、稳定 DTO、无损路径和缓存维护 |

前七行路径位于 `crates/vclogg-app/src/`；未加前缀的能力模块文件位于 `workspace/`。

## 数据与状态

`StateRepository` 独占 SQLite 连接、SQL 和迁移，**当前 schema 为 v22**。app 的 `StateStore` 将稳定记录适配为展示模型，不持有 SQL。WAL 与按 revision 合并会话字段保留不同窗口的独立修改。路径使用无损原生身份，显示字符串不作为文件身份。

文件标记、文字注释和颜色规则属于文件；搜索标签拥有查询草稿、已完成查询、范围、行边界、结果模式、选择、折叠和视口。每窗口同时执行一个搜索，其余排队；后台完成不切换标签，过期任务发布前必须被拒绝。

行索引与压缩结果避免保存全部解码文本。可见行、搜索快照和导出保持稳定源行身份。增量刷新有明确的首尾采样边界，见[动态日志](Live-Logs.md)。

普通浏览和注释不写源日志；显式[编辑](Editing.md)是独立写入路径，进入编辑限制为 100 MiB，并进行磁盘版本检查。关闭或重命名标签只改变工作区，不改变源文件名。

可选 Go 服务独立构建。其默认签名鉴权不自动兼容当前桌面客户端的无签名注册，见[云端过滤器](Cloud-Filters.md)。

源码：[工作区模块](https://github.com/zhaiyanqi/vclogg2/tree/main/crates/vclogg-app/src/workspace)、[core](https://github.com/zhaiyanqi/vclogg2/tree/main/crates/vclogg-core/src)、[状态仓储](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-data/src/state_repository.rs)。

## 性能 trace

`ui_performance/trace.rs` 保留应用级 Recorder；`trace/startup.rs` 在 `main` 的平台初始化之前解析并启动命令行录制，再在 GPUI 初始化回调中接入 foreground journal。独立线程每 50 ms 排空 journal 与任务缓冲，以统一 Instant 基准导出 Chrome Trace Event JSON。冷启动到时自动落盘；手动录制停止后选择保存位置，取消或失败保留数据供重试。

应用 scope 在进入时登记线程与起点，退出时记录完成事件；停止冻结时保留尚在执行的 scope，并补取 GPUI 边界任务快照，以 `incomplete` 标识。事件和活动 scope 分别限制为 200,000 / 4,096，丢失数量写入摘要。跳帧、空闲、挂起和短 poll 聚合用瞬时事件表示；dirty-to-present 和回调投递延迟使用带测量时长的瞬时标记，不冒充 CPU 工作。应用埋点覆盖启动、文件准备/索引、搜索执行与结果安装、标签激活、可见行加载和设置应用，未录制时新增 scope 不读取时钟。参见[冷启动参数](Building.md#冷启动性能录制)。

每次 Capture 显式保留 UI 线程的 Rust ThreadId，手动入口取 GPUI 前台回调线程，冷启动入口取 main 线程。导出用该身份统一标注 `GPUI UI / Main` / `Background`，将原始 Rust 线程身份写入事件参数，避免按事件类别误把 UI 上的任务或应用 scope 当成后台工作。每个实际 Rust 线程只映射到一个 tid；按时间与外层优先顺序导出嵌套片段，交叉区间在父区间终点切分后续接到同一线程。片段保留原始事件编号及时间，避免创建虚构线程。帧延迟用带 elapsed_us 的瞬时标记表示，导出片段数量受限且显式报告丢失。
