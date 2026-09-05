# GPUI Kit 编码指南优化记录

依据：[GPUI Kit 编码指南](https://gpui-kit.com/zh-CN/docs/coding-guides/)。检查日期：2026-09-05。API 以项目 `Cargo.lock` 中的 GPUI `99b0ed6`、gpui-component `38b2f65` 源码为准。

## 已落实

| 指南要求 | 原实现问题 | 本次改动 |
| --- | --- | --- |
| 状态由最窄生命周期的所有者持有 | 每次打开文档都向窗口订阅集合追加三个句柄，关闭标签不会移除这些句柄 | `DocumentTab` 持有正文、当前结果和结果模式订阅，统一随标签释放 |
| 渲染只组合已安装的呈现数据 | 历史窗口每次重绘都扫描两份完整集合、转换小写并创建筛选集合 | 输入变化、初始化和后台数据安装时更新下标投影，重绘直接消费投影 |
| 长集合使用虚拟化 | 历史记录为全部匹配会话生成节点、提示文字和按钮 | 固定行高的 `uniform_list` 只构造可见范围及测量行，保留原有行布局与边缘滚动条 |
| 使用稳定领域身份 | 临时结果行和删除按钮按筛选后的序号编号 | 原生路径 `ElementId::Path` 加子命名空间；不把有损显示路径当唯一身份 |
| 后台任务有明确状态且不能重复提交 | 两类删除使用独立标志，共享一个可被覆盖的任务句柄 | `HistoryDeletion` 将操作与任务放在同一状态中，两页共享提交限制 |
| 避免每帧克隆大集合 | “清理全部”每次渲染都会复制全部可清理路径 | 状态更新时计算数量，点击时才收集实际路径 |
| 保持依赖方向 | 守卫未覆盖 `gpui-kit`、资源包、平台入口及点号式依赖声明 | 补齐包名、点号式声明、依赖表和显式 `package` 别名检查 |

行为与状态仍归应用层的 `HistoryDialog` 和 `DocumentTab`；SQLite 与日志读取边界没有移动。历史行的打开、保护判定、行内确认及临时文件清理仍调用既有命令入口。

历史筛选依然按原规则匹配完整路径和保存查询，不额外裁剪空格。下标投影仅用于查找源记录，元素身份仍由数据库 ID 或原生路径决定。可见行渲染不做 I/O、不注册订阅、不修改业务状态；GPUI 每次布局重新测量现有 rem 行高。

## 验证边界

前两轮按 `RULES.md` 默认只做静态检查。随后用户明确要求编译，执行 `cargo build --workspace --locked`，发现窗口注册代码遗漏了两处订阅字段改名；已将其改为 `workspace._subscriptions`，窗口激活与外观订阅仍由 Workspace 持有。修复后 Debug 编译通过。

`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo fmt --all -- --check`、`git diff --check` 和架构守卫均通过；守卫同时检查常规 PATH 和无 `rg` 的 `/usr/bin:/bin` PATH，Shell 语法检查此前已通过。

没有新增或运行测试用例，也没有运行 Release 编译或真实窗口验收。当前验证确认本机 Debug 构建及所有目标的 Clippy 检查通过，不证明键盘与焦点交互正确、滚动视觉效果或实际性能提升。

## 后续仍需处理的范围

- 当前仍依赖已锁定的 `gpui` / `gpui-component` 分层包，尚未迁移到新版指南的单一 `gpui-kit` 入口。迁移需要独立核对组件和平台 API，并完成编译与交互验证。
- 临时结果卡片仍是可变高度的普通列表；本次仅固定会话行采用虚拟化。历史筛选仍在输入事件中同步执行，数据极大时需要先测量，再决定是否引入后台投影与版本校验。
- `predefined_filters_dialog.rs`、`settings_dialog.rs` 等大型能力模块及其他渲染时筛选路径仍可继续拆分或调整，需要按状态所有权划分，避免只为缩短文件而搬代码。
- `workspace/render_shell.rs` 等处仍有固定像素值；后续需区分产品尺寸、测量几何与平台边界，再逐项迁移到语义尺寸。现有界面不能据此宣称已经全面符合指南。

## 组件用法复查

继续对照 [组件文档](https://gpui-kit.com/zh-CN/docs/)、[ElementId](https://gpui-kit.com/zh-CN/docs/element_id/)、[Dialog](https://gpui-kit.com/zh-CN/docs/components/dialog/)、[ColorPicker](https://gpui-kit.com/zh-CN/docs/components/color-picker/) 和锁定版本源码检查。在线文档与项目版本存在差异，具体事件、方法签名及生命周期以锁定源码为准。

| 位置 | 发现与处理 |
| --- | --- |
| `color_labels_dialog.rs` | 日志级别行和颜色标签行的订阅原来累积在对话框中；现在每行持有三个句柄，删除时一起释放。父级只在文字内容改变时重绘，Input 自己处理焦点等交互状态 |
| `predefined_filters_dialog.rs` | 每次新增、导入或云端合并重建行都追加三条输入订阅；现在由 `FilterDraft` 持有，删除与替换源行时自动取消。固定连接输入的订阅继续归对话框所有，并保留过滤器输入的既有焦点通知 |
| `dialog_focus.rs` | 固定外层 `ElementId` 曾搭配每帧新建的 `FocusHandle`，两者生命周期不一致；现在使用 `RenderOnce` 与 `window.use_keyed_state` 保留派发焦点，沿用 GPUI Base Button 的生命周期机制 |

同时核对了以下实现，未发现需要据此修改的用法：

- 初始化先调用 `gpui_component::init`，每窗口有唯一 `Root`；Workspace 只各挂载一次 Dialog、Sheet、Notification 图层。
- 两个直接修改主题的入口都调用 `Theme::sync_base`，Base 与组件主题同步。
- 锁定源码中 `ColorPickerState::set_value` 不发出 `Change`，所以设置页的恢复默认颜色不会由该调用重新标记为自定义颜色；用户提交颜色后仍执行原有触发焦点恢复。
- `SelectState::set_selected_index` 用于恢复选择，不发出用户 `Confirm` 事件；当前结果模式恢复路径符合这一契约。
- 日志换行测量的应用层失效键已包含字体、字号和窗口 rem，不能只因低层列表没有同名字段就判定缺少缩放失效处理。
- 当前依赖的 `TabBar` 内部按位置给 `Tab` 分配 ID，没有应用可设置的稳定 Tab ID 接口。应用提供的关闭、右键控件已按文档 ID 命名；内部身份的改进需要组件版本支持，不通过猜测新 API 修补。

这轮保持已有布局、动作路由和测试文件。编译与 Clippy 验证范围见上文；未运行现有焦点回归或真实窗口验收，新增焦点保留行为的运行时效果尚未验证。
