# 键盘快捷键

以下是默认绑定。Windows/Linux 的主修饰键为 **Ctrl**，macOS 为 **Command（⌘）**；用户保存的快捷键优先。可以从“帮助 → 键盘快捷键”或“工具 → 设置 → 快捷键”查看并编辑。

## 可配置的 10 项操作

| 操作 | Windows / Linux | macOS | 使用位置 |
| --- | --- | --- | --- |
| 打开日志 | Ctrl+O | ⌘O | 工作区 |
| 聚焦主搜索栏 | Ctrl+F | ⌘F | 工作区 |
| 页内查找 | Ctrl+Shift+F | ⌘⇧F | 最后实际点击的正文或结果区域 |
| 关闭当前标签 | Ctrl+W | ⌘W | 工作区 |
| 打开 / 关闭设置窗口 | Ctrl+, | ⌘, | 工作区及设置窗口 |
| 切换搜索大小写 | Alt+C | ⌥C | 工作区 |
| 文件末尾 | Ctrl+End | ⌘End | 工作区 |
| 轮换颜色标签 | Ctrl+D | ⌘D | 有选中文字或日志行 |
| 自动换行 | W | W | 工作区 |
| 新增文字标记 | N | N | 日志区域，选中单条源行 |

可直接按组合键录入，支持清除及单项重置。重复绑定会阻止保存；保存后运行时重新绑定并写入状态库。Esc 和 Tab 保留导航用途。

## 固定命令

| 操作 | Windows / Linux | macOS |
| --- | --- | --- |
| 新建原生窗口 | Ctrl+Shift+N | ⌘⇧N |
| 完整重新加载当前文件 | F5 | F5 |
| 转到源行 | Ctrl+G | ⌘G |
| 复制文字 / 所选行 | Ctrl+C | ⌘C |
| 复制行并带源行号 | Ctrl+Shift+C | ⌘⇧C |
| 全选当前日志区域 | Ctrl+A | ⌘A |
| 标记 / 取消标记行 | M | M |
| 文件开头 | Ctrl+Home | ⌘Home |
| 全屏切换 | F11 | ⌃⌘F，兼容 F11 |

## 区域导航与输入

| 操作 | 默认键 / 手势 |
| --- | --- |
| 上下移动 | ↑ / ↓ |
| 按页移动 | PageUp / PageDown |
| 区域首尾导航 | Home / End |
| 扩展行选择 | Shift + ↑ / ↓ / PageUp / PageDown / Home / End |
| 提交主搜索 | 主搜索框内 Enter |
| 页内查找下一处 / 上一处 | 页内查找框内 Enter / Shift+Enter；浮层内 F3 / Shift+F3 |
| 关闭页内查找或最上层弹层 | Esc |
| 控件焦点切换 | Tab / Shift+Tab |
| 日志字号调整 | Ctrl + 鼠标滚轮 |

## 焦点规则

多数应用绑定带 `!Input` 条件：正在输入文字时，M/N/W 等不会作为日志命令触发，Ctrl/⌘+A、C 等由输入控件处理。日志区域中的 M 对一行或多行统一标记；N 要求单行目标，二者含义不同。

macOS 紧凑键盘上 Home/End/Page 键可能需要 Fn 组合，F5/F11 也可能受系统功能键设置影响。表格列出应用收到的键，不假设特定键盘映射。


源码依据：[actions.rs](https://github.com/zhaiyanqi/vclogg2/blob/65fa1d4fa7f44984825fa84758515a03d2157aeb/crates/vclogg-app/src/actions.rs) · [state_store.rs](https://github.com/zhaiyanqi/vclogg2/blob/65fa1d4fa7f44984825fa84758515a03d2157aeb/crates/vclogg-app/src/state_store.rs) · [settings_dialog.rs](https://github.com/zhaiyanqi/vclogg2/blob/65fa1d4fa7f44984825fa84758515a03d2157aeb/crates/vclogg-app/src/settings_dialog.rs)。

设置窗口内，Ctrl/⌘+S 保存，Ctrl/⌘+W 或 Esc 关闭；未提交预览随取消恢复。
