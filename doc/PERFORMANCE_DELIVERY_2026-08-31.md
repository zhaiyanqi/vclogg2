# UI 渲染性能诊断与优化交付说明（2026-08-31）

## 交付范围

本轮只增强 dev/Debug 诊断能力并修复已确认的日志表格渲染热点。Release 路径不包含
计时、聚合、堆栈或监控线程；`ui-performance-profiler` 特性在非 Debug 构建中会直接
拒绝编译。本轮未构建 Release。

## 已交付能力

- Debug 渲染作用域超过阈值时，日志输出作用域、净耗时、UI 线程身份、限频统计和
  完整 Rust 调用堆栈。
- GPUI 完整帧分析器记录前台任务、输入、动作、绘制、提交、无效化次数和主要耗时
  贡献者，能发现应用 `Render` 之外的渲染线程长任务。
- 元素包装器分别测量 `request_layout`、`prepaint` 和 `paint`，并汇总子作用域调用
  次数、总耗时与最大耗时。
- 子作用域采集堆栈或写日志产生的停顿会从父作用域净耗时中扣除，避免诊断器制造
  多层数百毫秒误报。GPUI 完整帧数据仍保持原始墙钟时间，便于观察诊断本身的影响。
- 性能脚本使用独立的 Debug 产物、应用数据、缓存、会话数据库和单实例标识，不会
  污染日常实例。

## 已定位并修复的热点

`TextSelectionHandle::register` 会在每次参与者注册后向当前全部文本选择参与者发布
快照。日志表格原先在每个可见行的 `prepaint` 都注册一次，参与者数量为 N 时形成
O(N²) 的重复遍历。每一行 `paint` 前后还分别读取一次全局 `selected_text`，造成另一
组 O(N²) 遍历。

修复后，空闲帧只注册鼠标所在行作为拖选起点；任一选择激活后，再注册全部可见行，
保持跨行拖选和双击选词行为。每行绘制只更新自身选择投影，不再前后扫描全局选中文本。
缓存项释放或选择清除时会同步维护激活计数。

## 受控 A/B 验证

验证输入大小为 52,428,860 字节。两组都使用同一 Debug 二进制目录、同一窗口与应用数据、8 ms 告警
阈值、60 秒同作用域堆栈限频，各采样约 6 秒。基准组只临时恢复逐行注册，采样后已
恢复优化代码且重新编译。稳态绘制统计排除了大于 50 ms 的堆栈采集扰动样本。

| 指标 | 旧逐行注册 | 按需注册 | 结果 |
| --- | ---: | ---: | ---: |
| 空闲帧选择注册次数 | 53 | 0 | 消除常态注册广播 |
| 超过 8 ms 的表格预绘制帧 | 29 | 12 | -58.6% |
| 超阈值表格预绘制均值 | 10.581 ms | 9.514 ms | -10.1% |
| 超阈值表格预绘制最大值 | 17.391 ms | 15.526 ms | -10.7% |
| GPUI 稳态绘制均值 | 21.812 ms | 20.164 ms | -7.6% |
| GPUI 稳态绘制中位数 | 21.289 ms | 19.640 ms | -7.7% |
| GPUI 稳态绘制最大值 | 31.759 ms | 23.653 ms | -25.5% |

优化组首个表格预绘制帧包含 53 个可见文本元素，但明细中没有
`SelectableLogText::register_selection`；旧路径同一帧记录 53 次注册、合计
0.799 ms。该直接耗时之外，按需注册同时消除了注册过程中对全部参与者反复发布快照
产生的间接成本和无效化。

## 剩余瓶颈与边界

- 日志表格稳定预绘制仍约 8–10 ms。明细显示应用委托的 `render_td` 通常低于
  1.3 ms，文本 `request_layout` 合计约 0.15 ms，其余主要位于 gpui-component
  `DataTable` 的纵向 uniform list 与每行横向 virtual list 布局/预绘制路径。
- 直接把消息列改为固定列会破坏现有水平滚动；复制并维护整个组件库来改一个内部
  列表策略风险也较高，因此本轮没有用功能回退换取数字改善。
- 首帧 `Workspace::paint` 可达到约 74 ms，同时 DirectX 日志显示 GPU 管线缓冲区扩容，
  属于冷启动资源分配。完整帧分析器会保留该原始数据。
- 8 ms 阈值适合定位，日常开发建议从默认 16 ms 开始；完整堆栈会暂停 Debug UI
  线程，因此比较 GPUI 完整帧数据时应排除堆栈采集帧或提高阈值。

## 验证命令与结果

最终代码执行以下 Debug 检查：

```powershell
cargo build --workspace --locked --target-dir target/perf-debug
cargo clippy --workspace --all-targets --locked --target-dir target/perf-debug -- -D warnings
cargo build --workspace --locked --features vclogg2/ui-performance-profiler --target-dir target/perf-debug
cargo clippy --workspace --all-targets --locked --features vclogg2/ui-performance-profiler --target-dir target/perf-debug -- -D warnings
```

真实窗口验证使用隔离脚本加载上述 50 MiB 日志；窗口标题正确显示
`sample-log.txt — VCLogg2`，日志确认数据目录隔离、应用作用域堆栈、阶段汇总和 GPUI
完整帧事件均正常输出。依据仓库规则，本轮未新增或运行测试，未构建 Release。

## 相关中文提交

- `e002272 开发版添加UI渲染长任务堆栈日志`
- `686b1ef 细化UI渲染性能探针覆盖范围`
- `4936d55 添加隔离的Debug性能验证环境`
- `43e9567 添加性能侦测手段`（复测期间出现的现有提交，包含完整帧诊断和热点修复）
- `ac4bfe7 说明UI性能诊断净耗时口径`
