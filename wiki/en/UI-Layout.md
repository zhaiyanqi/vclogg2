# UI layout and stable IDs

[简体中文](../zh-CN/UI-Layout.md) · [Home / 首页](Home.md)

This current structural map preserves existing IDs. It is not a reproduction of every old control: the complete historical registry remains in [Archive](Archive.md). IDs omitted here are reserved, not available for reuse. Repeated runtime instances share a semantic control ID; IDs are not list indices.

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

Editing replaces log reading with an editor and hides the search panel; see [Editing](Editing.md). Empty tabs show open/recent/recovery content. AI controls and workspace sidebars are removed. Settings is a separate window, not inside the workspace overlay tree. Search-track collapse and search-panel expansion are independent.

New semantic controls introduced for this map:

```text
WorkspaceBody [A004] (reference)
├─ Existing-file editor [A806] — document_editing.rs / EditorState
└─ New-file draft editor [A807] — document_editing.rs / NewFileDraft
```

A806/A807 document the explicit editor branches previously absent from the Wiki registry. Keep these IDs when their placement changes. Source ownership: `workspace/render_shell.rs`, `tab_view.rs`, `search_tab_ui.rs`, `settings_window.rs`, `document_editing.rs`, and `settings_dialog.rs` under `crates/vclogg-app/src/` (workspace files under `workspace/`).

The text tag context menu reuses existing semantic IDs and is composed in `workspace/row_tags.rs` for the log body, local results and global results:

```text
Text tag context menu
├─ Edit mark… [A569] — open the tag editor
├─ Delete mark [A572] — remove the selected text tag
└─ Clear all marks [A601] — remove all text tags in the owning file, including invisible rows; retain row marks, reuse history and other files’ tags
```

A601 also remains in the row’s “Text mark” submenu; it is disabled when there are no tags, the document is stale, a file is opening or the state store is unavailable.

Performance recording is in Settings → Advanced, retaining its existing node IDs:

```text
Performance recording [A808] — application Recorder; recording and saving continue after Settings closes
├─ Start recording [A809] — create a trace cache file, append new events in background batches
├─ Stop recording [A810] — stop collection, drain remaining events and close the file, without a path dialog
├─ Retry save [A811] — retry a failed final save to the same cache path
├─ Dismiss retry [A812] — release retained data without deleting saved files
└─ Open folder [A813] — open the trace cache directory, including during recording
```

Cold-start capture stops at its [CLI deadline](Building.md#cold-start-performance-capture). Manual starts and stops are disabled during that capture; Open folder remains available. The recording path and write errors are displayed; failed writes retry automatically, and a failed final save allows retry or dismissal.

FileDropOverlay [A130] has been removed; its identifier remains reserved. FileDropObserver / FileDropTarget [A128] still receives external file and folder drops, opening them on release without a hover overlay.

Release notes after a successful update check (including cached results and no-newer-version results):

```text
Root overlays [A041] (reference)
└─ Release notes dialog [A814] — workspace/update_checks.rs
   ├─ Version and status [A815] — installed/latest version, publication date, cache status
   ├─ Release notes [A816] — selectable Markdown; standard dialog body scrolls
   ├─ Close [A817] — dismiss; Escape also closes and restores focus
   └─ Open download page [A818] — open the verified Release URL
```

The website manifest includes the full release body. Empty or legacy notes show an explicit fallback message. Failed checks continue to open the Releases index.

The update dialog immediately shows “Checking for updates” [A819], independent of notification settings, and replaces its content with the result. Close or Escape dismisses it while pending; background work may finish caching, but will not reopen the dialog or launch the browser after dismissal.

```text
Release notes dialog [A814] (reference)
├─ Version and status [A815] — installed/latest versions, release count, cache and partial-history notice
├─ Breaking changes and migration requirements [A820] — important notes from all selected releases, always visible
├─ Release notes [A816] — grouped by descending version
│  └─ Version disclosure button [A821] — stable version identity; newest three initially expanded
└─ View full release history [A822] — open the Releases index
```
