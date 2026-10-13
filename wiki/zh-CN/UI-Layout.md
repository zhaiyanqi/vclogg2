# UI 布局与稳定编号

[English](../en/UI-Layout.md) · [Home / 首页](Home.md)

本页是当前结构图，保留既有编号，不重复全部历史控件；完整旧编号表在[归档](Archive.md)。未列出的旧编号继续保留，不能复用。多个运行实例可以对应同一语义控件，编号不是列表下标。

```text
AppShell [A001] — Root / Workspace
├─ TitleBar [A002] — menus / 菜单
├─ FileToolbar [A011] — file commands / 文件操作
│  ├─ Open [A012] — 打开日志
│  ├─ Reload [A045] — 重新加载
│  ├─ Follow tail [A015] — 末尾跟随
│  └─ Path [A014] — 路径面包屑
├─ DocumentTabBar [A016] — horizontal file tabs / 横向文件标签
│  └─ DocumentTab [A017] — one instance per document / 每文档实例
│     └─ Close [A018] — 关闭当前标签
├─ WorkspaceBody [A004] — content / 内容
│  └─ DocumentSplit [A024] — resizable / 可调整分区
│     ├─ LogDataTable [A025] — virtual log / 虚拟日志
│     │  ├─ Marks [A051] — 行标记
│     │  ├─ Line numbers [A026] — 源行号
│     │  └─ Log text [A027] — 日志正文
│     └─ Search panel
│        ├─ SearchBar [A034] — scope and query / 范围与查询
│        │  ├─ Case [A036] — 大小写
│        │  ├─ Regex [A037] — 正则
│        │  ├─ Input [A035] — 查询输入
│        │  ├─ Search [A039] — 执行
│        │  └─ Clear [A038] — 清除
│        └─ SearchResultsPanel [A029] — results / 结果
│           └─ ResultDataTable [A032] — source rows / 源行
├─ StatusBar [A007] — search tabs and status / 搜索标签与状态
│  ├─ Collapse track [A804] — 独立收起搜索标签轨道
│  └─ All search tabs [A805] — 搜索标签列表
└─ Root overlays [A041] — dialogs and notifications / 弹窗与通知

SettingsWindow [A094] — shared native window / 进程唯一原生窗口
├─ TitleBar [A800] — native close/resize / 原生关闭与缩放
├─ Sidebar [A309] — categories / 分类
│  ├─ Search [A310] — 设置搜索
│  └─ Navigation [A311]
│     ├─ General [A312] — 常规
│     ├─ Network [A389] — 网络
│     ├─ Appearance [A313] — 外观
│     ├─ Highlight [A801] — 高亮
│     ├─ Search [A314] — 搜索
│     ├─ History [A802] — 历史
│     ├─ Storage [A317] — 存储
│     ├─ Shortcuts [A318] — 快捷键
│     ├─ Advanced [A316] — 高级
│     └─ About [A395] — 关于
├─ Details [A319] — category content / 分类内容
└─ Actions [A553]
   ├─ Cancel [A554] — 取消
   └─ Save [A555] — 保存
```

编辑时正文切换为编辑器并隐藏搜索面板，见[文件编辑](Editing.md)。空标签显示打开、最近文件与恢复入口。AI 控件和工作区侧栏已移除；设置是独立窗口，不属于工作区弹层树。搜索标签轨道折叠与搜索面板展开互相独立。

本次补充此前编号表遗漏的编辑分支：

```text
WorkspaceBody [A004]（引用）
├─ 已有文件编辑器 [A806] — document_editing.rs / EditorState
└─ 新建文件草稿编辑器 [A807] — document_editing.rs / NewFileDraft
```

A806/A807 用于已实现的显式编辑器，后续移动时保留编号。源码归属为 `crates/vclogg-app/src/` 下的 `workspace/render_shell.rs`、`tab_view.rs`、`search_tab_ui.rs`、`settings_window.rs`、`document_editing.rs` 与 `settings_dialog.rs`（工作区文件位于 `workspace/`）。

文字标记自身的右键菜单复用现有语义编号，由 `workspace/row_tags.rs` 生成，正文、当前结果和全局结果共用：

```text
文字标记右键菜单
├─ 编辑标记… [A569] — 打开标记编辑弹窗
├─ 删除标记 [A572] — 删除当前文字标记
└─ 清除所有标记 [A601] — 清除所属文件全部文字标记，包括不可见行；保留行标记、复用历史和其他文件标记
```

A601 同时保留在日志行的“文字标记”子菜单内；无文字标记、文档已失效、正在打开文件或状态库不可用时禁用。

性能录制位于“设置 → 高级”，沿用既有编号：

```text
性能录制 [A808] — 应用级 Recorder，关闭设置仍持续录制及保存
├─ 开始录制 [A809] — 新建 cache/traces 会话文件，后台持续增量写入新事件
├─ 停止录制 [A810] — 停止采集，写完余量与摘要并关闭文件，不再选择文件位置
├─ 重试保存 [A811] — 最终保存失败后重试原缓存路径
├─ 放弃重试 [A812] — 释放待重试数据，不删除已写文件
└─ 打开目录 [A813] — 打开 trace 缓存目录，录制中也可使用
```

冷启动录制按[命令行时长](Building.md#冷启动性能录制)自动停止，期间禁用手动开始和停止；可打开目录。显示录制路径及写入错误，写入失败自动重试；最终保存失败时允许重试或放弃。

FileDropOverlay [A130] 已移除，编号继续保留。FileDropObserver / FileDropTarget [A128] 继续接收外部文件和文件夹拖放，松手后打开，不再显示悬停提示浮层。

检查更新成功后显示发布说明，命中缓存和没有新版时也显示：

```text
Root overlays [A041]（引用）
└─ 更新内容弹窗 [A814] — workspace/update_checks.rs
   ├─ 版本与状态 [A815] — 当前/最新版本、发布日期、缓存状态
   ├─ 更新说明 [A816] — 可选择的 Markdown；标准弹窗正文滚动
   ├─ 关闭 [A817] — 关闭弹窗；Esc 同样关闭并恢复焦点
   └─ 打开下载页面 [A818] — 打开已校验的 Release 链接
```

官网清单包含完整 Release 正文。说明为空或旧清单不含说明时显示明确提示。检查失败仍打开 Releases 首页。
