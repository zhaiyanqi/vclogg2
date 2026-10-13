---
name: vclogg-trace
description: 解析 VCLogg2 导出的 Perfetto / Chrome Trace JSON，识别 GPUI UI 线程、还原拆分事件、定位控件和行为耗时，分析卡顿或冷启动。适用于 trace 文件分析、采集说明及埋点覆盖判断；不用于普通业务日志解析。
---

# VCLogg2 trace 分析

先读取项目根目录 `RULES.md`。分析已有录制不代表授权编译、启动应用、添加测试或修改外部 GPUI 源码。项目技能随仓库纳入 Git 管理；分析过程资料及产物仅本地保留，放 `.local/notes/`，不强制加入 Git。

## 实现与格式来源

以下路径相对于项目根目录。以生成文件的版本为准；先看 trace summary，遇到格式差异或解释不确定时读对应实现，不必每次通读所有源码。

- `crates/vclogg-app/src/ui_performance/trace.rs`：`collect` 事件映射、应用 `scope`。
- `crates/vclogg-app/src/ui_performance/trace/stream.rs`：增量 JSON 数组、线程边界索引、跨批次切片与失败重试。
- `crates/vclogg-app/src/ui_performance/trace/live.rs`：独立增量写入线程与停止排空。
- `crates/vclogg-app/src/ui_performance/trace/startup.rs`：冷启动参数、期限。
- `crates/vclogg-app/src/ui_performance.rs`：控件阶段包装、叠加层入口。
- `crates/vclogg-app/src/app_paths.rs`：缓存路径与 Debug 隔离。
- `wiki/zh-CN/Building.md`：采集操作与当前叠加层 MAX 定义。

## 取得稳定快照

手动录制在“设置 → 高级 → 性能录制”，支持开始、停止、打开目录及保存失败后重试。冷启动命令供用户执行；仅在明确获准启动时才代为运行已有可执行文件：

```sh
./target/release/vclogg2 --trace-startup --trace-duration 15
```

Windows 使用 `.exe`。`--trace-startup` 是开关，不接输出路径；默认 15 秒，时长可取 1–600 整数秒。打开日志可在末尾加 `-- "日志路径"`。已有实例可能接收打开请求，不能据此认为录到了原进程冷启动。

文件名为 `vclogg2-trace-时间-UUID.json`。macOS/Linux 位于系统缓存根的 `VCLogg2/traces`；非 Windows Debug 设置 `VCLOGG2_DEV_DATA_DIR` 时位于该根的 `cache/VCLogg2/traces`。Windows 位于应用配置数据根的 `cache/traces`。优先用用户提供文件或“打开目录”确认位置。

当前 format_version=2 是顶层 JSON 事件数组；后台按最多 2048 个新完成事件分批写入，排空后等待约 250 ms，每批只覆盖末尾一个 `]` 并追加新事件与新的 `]`，再 sync_data。停止仅处理余量、活跃 scope 和最终摘要，不重写全量。写到一半崩溃时，Perfetto 支持事件数组尾部截断，保留此前完整事件；严格 JSON 解析器可能拒绝。未采集或未完整写入的事件不保证保留，断电和存储故障不在该保证内。

分析正在写入的文件时先复制到 `.local/notes/`，仅分析该副本。严格解析失败且确认是 v2 数组截断时，可用 JSONDecoder.raw_decode 从 `[` 后依次读完整对象，到第一个解码失败处停止，报告偏移和尾部截断；不得跳过中间损坏继续猜测，也不要修改原文件或补造事件。旧格式是包含 traceEvents 的对象、周期完整快照；按实际版本分支。

## 数据质量与单位

用本地 JSON 解析器读取顶层数组，兼容旧格式对象的 `traceEvents`，避免打印整份大文件。优先取最后一条 `Recording summary`，若无则取最新 `Recording checkpoint` 的 args；后者表示尚未正常收尾：

| 字段 | 分析含义 |
| --- | --- |
| `version`、`build`、`duration_ms` | 版本、Debug/Release、窗口长度；Debug 开销不可直接外推 Release |
| `recording` | 是否仍接收事件；true 常见于实时快照，不是坏文件；false 不保证无丢失 |
| `ui_thread` | 捕获的真实 Rust UI 线程身份 |
| `lost_events` | 采集缓冲丢失、容量或锁竞争丢失、导出省略的合计，不只是缓冲已满 |
| `omitted_export_segments` | v2 为累计省略事件/剩余逻辑尾段/活跃观测数；旧版为导出结束待处理条目数。已计入 lost_events，不再次相加，不是潜在递归切片的精确数 |
| `limit`、`active_scope_limit`、`export_segment_limit` | 当前为 200000、4096、404096；v2 活跃观测也占片段额度，metadata/周期摘要除外，以文件为准 |
| `includes_active_scopes` | 包含尚未结束的应用 scope，不等于完整调用栈 |

缺少字段时标记旧格式或未知版本，按实际字段降级分析。JSON 不完整时按版本确认数组尾部截断或实际损坏，不补造尾部事件。丢失、录制边界及埋点缺失都应降低结论可信度。

`displayTimeUnit: "ms"`（旧版对象）仅是展示单位，v2 数组不含此字段。JSON 的 `ts`、`dur`、`elapsed_us`、`original_ts_us`、`original_duration_us` 都是**微秒**，除以 1000 才是毫秒；summary 的 `duration_ms` 已是毫秒。ts 相对录制开始，不是 Unix 时间；没有外部共同基准不能直接对齐不同录制。

## 识别 UI 线程与事件

用 `ph=M`、`name=thread_name` 的 metadata 获取线程名。当前 UI 为 `GPUI UI / Main · ThreadId(...)`、tid=1；其他为 `Background · ThreadId(...)`。结合 summary 和 `args.thread_role` 核验，不只硬编码 tid。

pid=1 和导出 tid 是文件内标识，不是 OS PID/TID。`args.source_thread` 是 Rust ThreadId，同样不是系统线程 ID。当前每个来源线程一个轨道，类别不新建线程；轨道上下层是事件嵌套。旧版多条 Main 可能为同一线程分轨，不能据数量推断 UI 多线程渲染。

`cx.spawn` 的 future 在前台轮询，poll 内同步逻辑可能占用 UI；await 通常让出执行，不会自动把后续代码搬到后台。以具体 Task 来源线程判断执行位置；整个异步生命周期耗时不能当作主线程连续占用。

| 类别 | 含义及读取方式 |
| --- | --- |
| `Application`，ph=X | 应用 scope 的 wall time，可能含子调用、阻塞或等待；检查是否跨 await |
| `GPUI draw`、`GPUI present` | UI 侧绘制/提交相关区间，不是 GPU 时间 |
| `GPUI actions`、`GPUI input` | 操作/输入分发耗时 |
| `GPUI tasks` | 任务 poll；Task 位置是 spawn 来源，不是完整调用栈 |
| `GPUI frame latency`，ph=i | `Dirty to present`、`Callback delivery`，用 args.elapsed_us；ts 为测量起点，不是持续占用 UI 的执行 slice |
| `GPUI markers`，ph=i | pending、callback、skipped、idle 等状态线索，不是有 dur 的执行事件 |
| `Short polls: N (... ms total)` | 聚合次数/累计值在名称中；poll 间有空隙，不能用首尾跨度当 CPU 忙碌 |
| `Journal gap: ... (observed here)` | ts 是观察到断档时刻，不是精确丢失起点 |
| `Application active scopes`，ph=i | 约每秒一次活跃观测，以 active_scope_id 关联，elapsed_us 是从原始起点到该次观测的时长；取最后观测，不累加，也不与正式 X 区间叠加 |

v2 录制中的活跃 Application scope 以瞬时观测写入，完成或停止后才写 X 区间，使用 active_scope_id 关联；旧版快照可直接包含活跃 X scope；GPUI 正在执行的长任务不一定已进入完成队列。停止时会尝试加入 `Task at capture boundary ...`。卡住时没有 GPUI 长 slice 不能证明没有阻塞。

## 还原拆分事件

导出器把交叉区间切成合法嵌套片段以保留单线程单轨。Perfetto 展示的父子层级可能来自该转换，不能直接当作真实调用关系。

1. 对 ph=X，在**同一快照内**按 `(pid, tid, args.source_event)` 分组。缺少 source_event 的旧事件各自保留，不能把所有缺失值合成一组。
2. `continued=true` 为续段，`split=true` 表示片段被切分。一次逻辑事件只统计一次。
3. `original_ts_us`、`original_duration_us` 给出逻辑区间，但已被裁剪到录制边界，不是录制前后完整生命周期。
4. 不重复累加每个片段的 original_duration_us。片段齐全时 sum(dur) 应接近逻辑时长；片段省略时分别给出逻辑时长和实际可见片段覆盖时间。
5. `incomplete=true` 表示未结束或被截断，仅给出已观察耗时，不声称完整执行时间。

v2 的 source_event 在同一录制中稳定，跨录制不唯一；active_scope_id 单独用于关联活跃观测与正式区间。旧版 source_event 来自每次快照数组索引，跨快照不稳定。任何版本都不要把同一录制的多个文件副本相加。

统计某线程在某窗口的已记录覆盖时间：复原区间 → 裁剪观察窗口 → 求区间并集。不得直接累加嵌套 scope；并集仍是 wall-time 覆盖，不是 CPU 利用率。只有验证真实调用嵌套与覆盖充分时才估算排除子区间后的耗时，并标为估算；展示层级不足以证明 self time。

## 从卡顿定位代码

确认用户操作、时间段、窗口和刷新率，再在 UI 轨道检查长 Draw、Input、Action、Task、Application。60 Hz 约 16.7 ms、120 Hz 约 8.3 ms 只是帧预算参考；缺少帧时序证据不能把所有超阈值事件都叫丢帧。

对照同一窗口的 pending、callback、Dirty to present、Callback delivery，分别考虑执行慢、回调交付延迟、呈现等待。窗口 ID 不代表独立 UI 线程。按最长单次与重复累计分别排名；给出复原后的次数、最大/总 inclusive 时长、关键时间范围及 incomplete 状态。不同名称总时长可重叠，不用它们相加计算整体占比。

精确搜索事件名回到代码：

```sh
rg -n -F 'SelectableLogText::prepaint' crates/vclogg-app/src
rg -n 'ui_performance::scope|ui_performance::element' crates/vclogg-app/src
rg -n 'scope\(|Startup' crates/vclogg-app/src/main.rs
```

常见线索：Workspace::render、LogRegionSurface::render、Workspace::render_wrapped_log_rows、SelectableLogText 的 request_layout/prepaint/paint、Startup.*。render 仅构造元素，不含后续所有布局和绘制；element 包装可能包含整个子树，不要归因成外层控件自身耗时。埋点清单以源码为准，不声称所有控件、扫描及数据库内部逻辑都已覆盖。

后台长任务不必然直接阻塞 UI；锁、同步等待、CPU/内存/IO 争用、大量回调可能影响流畅度。结合 UI 等待点及数据交接查证，无关联证据时只列候选原因。空白轨道可能是等待、未埋点、丢失、未完成或平台工作，不能单凭空白证明空闲或卡顿。

本 trace 无系统调度、CPU 采样栈、GPU 计时及自动跨线程因果关系。证据不足就明确指出，建议应用层具体区间补点；分析请求本身不授权实施修改。不要自动修改依赖、增加常驻检测线程或恢复性能超时控制台日志。

叠加层 MAX 的当前定义是该窗口最近最多 1000 次 begin_draw 到 end_draw 的最大经过时间，仅覆盖 Draw 阶段，不是所有主线程任务最大值、帧间隔或 GPU 时间。依赖升级后须重新核对。

## 输出结果

先给问题点及证据强度，再给精简表格：线程、事件、相对起点 ms、逻辑耗时 ms、次数/完整性、源码位置。区分事实、候选原因与缺失证据，交代实际丢失数、快照状态和构建类型。没有文件时说明采集方法，不虚构排名。

Perfetto 可直接打开完整 JSON。自动统计优先解析原始 JSON；使用 Perfetto SQL 时先核对当前 schema、导入单位及 args 映射，不假定同原始字段完全一致。无需为通用说明安装工具或上传文件。
