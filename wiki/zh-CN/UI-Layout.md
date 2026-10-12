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
