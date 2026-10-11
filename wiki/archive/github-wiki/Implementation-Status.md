# 功能实现状态

更新日期：2026-10-09，源码 `b14c8db`。基于当前源码整理；**已实现**表示主路径存在，不代表此次运行过自动测试、GUI 或三平台验收。旧阶段验收见[历史档案](https://github.com/zhaiyanqi/vclogg2/wiki/Archive)，旧实现明细保留在[迁移前状态记录](https://github.com/zhaiyanqi/vclogg2/wiki/Archive-Implementation-Status)。

| 能力 | 当前状态与边界 | 详细说明 |
| --- | --- | --- |
| 打开与编码 | 多选、拖放、命令行、打开预览、索引、自动编码检测、Binary 十六进制显示 | [使用](https://github.com/zhaiyanqi/vclogg2/wiki/User-Guide) |
| 大文件显示 | 正文/结果虚拟列表、有界可见内容、源行身份与独立视口 | [架构](https://github.com/zhaiyanqi/vclogg2/wiki/Architecture) |
| 主搜索 | 多关键词、正则、大小写、取消、完整结果提交，无临时进度预览 | [搜索](https://github.com/zhaiyanqi/vclogg2/wiki/Search-and-Filters) |
| 搜索范围 | 当前、选定已打开文件、目录；支持按文件源行起止限制 | [搜索](https://github.com/zhaiyanqi/vclogg2/wiki/Search-and-Filters) |
| 数量上限 | 0 不限；各文件独立保留源序前 N 条匹配 | [设置](https://github.com/zhaiyanqi/vclogg2/wiki/Settings) |
| 页内查找 | 正文/当前结果/全局结果目标、全词/大小写/正则、前后定位 | [快捷键](https://github.com/zhaiyanqi/vclogg2/wiki/Keyboard-Shortcuts) |
| 动态日志 | 仅前台活动文档通知与兜底复核；头尾快速校验有中部改写漏检边界，运行验收待补 | [动态日志](https://github.com/zhaiyanqi/vclogg2/wiki/Live-Logs) |
| 选择与标记 | 文本、多行、跨结果源行定位、行标记、独立文字标记与复用历史 | [交互](https://github.com/zhaiyanqi/vclogg2/wiki/Interaction) · [标记](https://github.com/zhaiyanqi/vclogg2/wiki/Row-Tags) |
| 高亮与外观 | 系统字体枚举、独立搜索控件尺寸、匹配/选择样式、三态主题、四款图标 | [设置](https://github.com/zhaiyanqi/vclogg2/wiki/Settings) |
| 多标签多窗口 | 文件标签与统一搜索标签、排序、关闭、重命名、复制/安全移动、单实例路由；关闭文件最后一个搜索标签会关闭该文件 | [交互](https://github.com/zhaiyanqi/vclogg2/wiki/Interaction) |
| 数据恢复 | SQLite/WAL、文件会话、最近/收藏/上一次文件、缓存管理 | [数据](https://github.com/zhaiyanqi/vclogg2/wiki/Data-and-Privacy) |
| 导出 | 完整已提交结果、文件分组、指定时间戳合并、临时结果独立管理 | [导出](https://github.com/zhaiyanqi/vclogg2/wiki/Result-Export) |
| 过滤器 | UUID 分支、v5 JSON、旧格式兼容、可选云端修订/冲突与离线缓存 | [过滤器](https://github.com/zhaiyanqi/vclogg2/wiki/Cloud-Filters) |
| 快捷键与通知 | 10 项可配置快捷键，固定命令；进程内通知历史和弹出开关 | [快捷键](https://github.com/zhaiyanqi/vclogg2/wiki/Keyboard-Shortcuts) · [界面](https://github.com/zhaiyanqi/vclogg2/wiki/Interface) |
| Windows 交付 | x86_64 便携 ZIP + 安装 EXE，签名可选，包名不区分签名状态 | [发布](https://github.com/zhaiyanqi/vclogg2/wiki/Build-and-Release) |
| macOS / Linux 交付 | ARM64 DMG / x86_64 tar.gz；macOS 当前无 Developer ID 公证 | [安装](https://github.com/zhaiyanqi/vclogg2/wiki/Installation) |
| 更新 | 菜单“检查更新”打开 Releases；无客户端自动检查/下载/安装 | [排障](https://github.com/zhaiyanqi/vclogg2/wiki/Troubleshooting) |
| CI | 常规 CI 仅手动触发；v* 标签触发 Release；官网有独立 Pages 工作流 | [发布](https://github.com/zhaiyanqi/vclogg2/wiki/Build-and-Release) |

## 当前版本调整

- 设置为进程共享的独立窗口，立即显示，无入场动画；高亮与历史管理并入设置。
- 搜索标签统一显示，提供新增范围菜单、可搜索列表和固定折叠按钮；新标签立即出现，关闭与拖动保留过渡。
- AI、MCP、记忆和工作区左右侧边栏已移除；通知面板仍保留。

## 尚未由当前实现或本次验收保证

- 目录持续监测、新日志自动发现、后台所有标签实时刷新。
- macOS Developer ID 公证部署、任意 Windows 包均已签名。
- 其他架构与操作系统版本的预编译发行包。
- 任意 GPU/虚拟机环境下的流畅性、持续写入场景的固定最大延迟。
- 任意时间戳格式的自动规范化和全局时间线排序。
- 云端服务的部署、可用性和所有旧服务版本的双向写兼容。

功能变更时更新本表及相应专题；验收记录必须注明提交、平台、数据规模与实际执行的检查。
