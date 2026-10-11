# 文件编辑与剪贴板文档

[English](../en/Editing.md) · [Home / 首页](Home.md)

普通浏览、搜索、行标记和文字注释不改变源文本。**进入编辑模式并保存会替换文件。**

在非输入框区域按 **I** 编辑当前文档。进入编辑的限制为 100 × 1024 × 1024 字节（100 MiB，界面写作“100 MB”）。编辑会完整读取并解码文件，与虚拟日志浏览不同。不支持或无效的编码会被拒绝；UTF-8/BOM、UTF-16 BOM 和支持的传统编码会保留，无法按原编码写回的文字会被拒绝。

编辑器内使用 **Ctrl+S / ⌘S** 保存。已有文件在写入前和替换前比较大小与修改时间，在同目录写入并同步临时文件、保留权限后替换目标。这不是文件锁或内容哈希并发保证，应避免编辑其他进程正在写入的文件。检测到磁盘变化时需重新打开后编辑。

新建文件草稿通过保存对话框选择路径，首次保存拒绝覆盖已有文件。退出未保存编辑时按界面的保存、放弃或取消流程处理，不要把关闭草稿理解为保存。

非输入框区域的 **Ctrl+V / ⌘V** 将剪贴板文本写入应用临时文件并进入编辑模式，是否先确认由粘贴确认设置决定；输入框仍正常粘贴。需要长期保存时请主动保存副本。编辑期间搜索面板隐藏。

源码：[document_editing.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-app/src/workspace/document_editing.rs)、[clipboard_paste.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-app/src/workspace/clipboard_paste.rs)、[actions.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-app/src/actions.rs)。
