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

文件关联和命令行请求首先将最后活动窗口置前，等待窗口呈现后才查找文件归属和操作标签。文件路由延后到帧回调释放窗口后执行，避免把 GPUI 暂时借用窗口误判为窗口不存在。已打开文件随后激活所属窗口，该窗口也先呈现再处理文档。外部打开完成标签激活后，复用拖入文件的定位机制，将新建或已打开的当前文件标签滚动到可见位置。冷启动先呈现空工作区，再创建初始文件标签；无路径重新打开应用时激活已有窗口。新文件立即创建加载中标签，包括冷启动恢复状态和其他文件仍在加载时。加载界面呈现后才开始文件准备，共享加载器继续保留有界并发和会话恢复。关闭排队标签会取消待打开请求；后台完成时保留当前标签，显式搜索导航仍按原有流程完成。

## 数据与状态

`StateRepository` 独占 SQLite 连接、SQL 和迁移，**当前 schema 为 v22**。app 的 `StateStore` 将稳定记录适配为展示模型，不持有 SQL。WAL 与按 revision 合并会话字段保留不同窗口的独立修改。路径使用无损原生身份，显示字符串不作为文件身份。

文件标记、文字注释和颜色规则属于文件；搜索标签拥有查询草稿、已完成查询、范围、行边界、结果模式、选择、折叠和视口。每窗口同时执行一个搜索，其余排队；后台完成不切换标签，过期任务发布前必须被拒绝。

行索引与压缩结果避免保存全部解码文本。可见行、搜索快照和导出保持稳定源行身份。增量刷新有明确的首尾采样边界，见[动态日志](Live-Logs.md)。

普通浏览和注释不写源日志；显式[编辑](Editing.md)是独立写入路径，进入编辑限制为 100 MiB，并进行磁盘版本检查。关闭或重命名标签只改变工作区，不改变源文件名。

可选 Go 服务独立构建。其默认签名鉴权不自动兼容当前桌面客户端的无签名注册，见[云端过滤器](Cloud-Filters.md)。

源码：[工作区模块](https://github.com/zhaiyanqi/vclogg2/tree/main/crates/vclogg-app/src/workspace)、[core](https://github.com/zhaiyanqi/vclogg2/tree/main/crates/vclogg-core/src)、[状态仓储](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-data/src/state_repository.rs)。

## 性能 trace

`ui_performance/trace.rs` 保留应用级 Recorder。`trace/startup.rs` 在平台初始化前开启命令行录制，GPUI 初始化回调接入 foreground journal；采集线程每 50 ms 排空 journal 和任务缓冲。`trace/live.rs` 的独立写入线程排空后等待 250 ms，`trace/stream.rs` 按最多 2048 个新完成事件编码批次，仅覆盖 JSON 数组末尾一个右括号并追加事件及新的右括号，每批 sync_data。Perfetto 支持数组文件末尾截断，崩溃前的完整事件仍可导入。停止仅排空余量、补记活跃 scope 和最终摘要，随后关闭文件。

Stream 由 Capture 持有，保存文件句柄、已提交字节偏移、事件游标、线程区间边界索引和失败批次；重试从原偏移重写同一失败批次，不重放已提交历史。仅在短锁内克隆新事件 Arc 与活动 scope，排序、序列化、落盘在后台执行。缓存路径和时间/UUID 命名不变；最终失败时保留状态供手动重试。录制期间的活跃 scope 以约每秒一次的瞬时观测呈现，最终截断区间与正常完成区间不会因周期保存被重复计时。

应用 scope 在进入时登记线程与起点，退出时记录完成事件；停止冻结时保留尚在执行的 scope，并补取 GPUI 边界任务快照，以 `incomplete` 标识。事件和活动 scope 分别限制为 200,000 / 4,096，丢失数量写入摘要。跳帧、空闲、挂起和短 poll 聚合用瞬时事件表示；dirty-to-present 和回调投递延迟使用带测量时长的瞬时标记，不冒充 CPU 工作。应用埋点覆盖启动、文件准备/索引、搜索执行与结果安装、标签激活、可见行加载和设置应用，未录制时新增 scope 不读取时钟。参见[冷启动参数](Building.md#冷启动性能录制)。

每次 Capture 显式保留 UI 线程的 Rust ThreadId，手动入口取 GPUI 前台回调线程，冷启动入口取 main 线程。导出用该身份统一标注 `GPUI UI / Main` / `Background`，将原始 Rust 线程身份写入事件参数，避免按事件类别误把 UI 上的任务或应用 scope 当成后台工作。每个实际 Rust 线程只映射到一个 tid；仅在批内按时间与外层优先排序，迟到的交叉区间依据已写片段的边界索引切分后续接到同一线程，不修改历史片段。片段保留原始事件编号及时间，避免创建虚构线程。帧延迟用带 elapsed_us 的瞬时标记表示，导出片段数量受限且显式报告丢失。

控件计时在 Debug / Release 中统一委托给按需 trace scope；未录制时不安装计时元素包装。旧的阈值控制台诊断、聚合明细和后台 HangDetector 监测已移除，避免诊断日志本身增加渲染开销。`ui-performance-profiler` feature 仅保留为既有命令的兼容别名。
