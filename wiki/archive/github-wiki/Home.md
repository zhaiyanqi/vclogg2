# VCLogg2 Wiki

VCLogg2 是使用 **Rust 与 GPUI Kit** 构建的跨平台原生桌面日志查看器，面向大文件阅读、关键词检索、跨文件排查和持续增长的日志。源日志保持只读；设置、会话、标记与缓存由应用独立保存。

[项目仓库](https://github.com/zhaiyanqi/vclogg2) · [下载安装](https://github.com/zhaiyanqi/vclogg2/releases/latest) · [中文官网](https://zhaiyanqi.github.io/vclogg2/zh.html) · [English website](https://zhaiyanqi.github.io/vclogg2/) · [反馈问题](https://github.com/zhaiyanqi/vclogg2/issues)

## 从这里开始

| 你想做什么 | 阅读入口 |
| --- | --- |
| 了解项目定位、能力和边界 | [项目概况](https://github.com/zhaiyanqi/vclogg2/wiki/Project-Overview) · [功能状态](https://github.com/zhaiyanqi/vclogg2/wiki/Implementation-Status) |
| 安装并完成第一次日志排查 | [安装](https://github.com/zhaiyanqi/vclogg2/wiki/Installation) → [快速上手](https://github.com/zhaiyanqi/vclogg2/wiki/Getting-Started) |
| 找到菜单、搜索栏、结果和设置 | [界面导览](https://github.com/zhaiyanqi/vclogg2/wiki/Interface) · [交互操作](https://github.com/zhaiyanqi/vclogg2/wiki/Interaction) · [快捷键](https://github.com/zhaiyanqi/vclogg2/wiki/Keyboard-Shortcuts) |
| 搜索多个文件、定位上下文、保存结果 | [搜索与过滤](https://github.com/zhaiyanqi/vclogg2/wiki/Search-and-Filters) · [结果导出](https://github.com/zhaiyanqi/vclogg2/wiki/Result-Export) |
| 跟踪持续写入的日志 | [动态日志](https://github.com/zhaiyanqi/vclogg2/wiki/Live-Logs) |
| 标记重点、调整颜色和字体 | [标记与高亮](https://github.com/zhaiyanqi/vclogg2/wiki/Marks-and-Highlighting) · [文字标记](https://github.com/zhaiyanqi/vclogg2/wiki/Row-Tags) · [设置](https://github.com/zhaiyanqi/vclogg2/wiki/Settings) |
| 编译或参与开发 | [技术栈](https://github.com/zhaiyanqi/vclogg2/wiki/Technology-Stack) → [源码构建](https://github.com/zhaiyanqi/vclogg2/wiki/Building) → [架构](https://github.com/zhaiyanqi/vclogg2/wiki/Architecture) → [贡献指南](https://github.com/zhaiyanqi/vclogg2/wiki/Contributing) |
| 遇到问题 | [常见问题与排障](https://github.com/zhaiyanqi/vclogg2/wiki/Troubleshooting) · [数据与隐私](https://github.com/zhaiyanqi/vclogg2/wiki/Data-and-Privacy) |

## 文档分区

### 入门

- [项目概况](https://github.com/zhaiyanqi/vclogg2/wiki/Project-Overview)：产品定位、典型场景、平台和当前限制。
- [安装](https://github.com/zhaiyanqi/vclogg2/wiki/Installation)、[快速上手](https://github.com/zhaiyanqi/vclogg2/wiki/Getting-Started)、[使用指南](https://github.com/zhaiyanqi/vclogg2/wiki/User-Guide)。
- [功能状态](https://github.com/zhaiyanqi/vclogg2/wiki/Implementation-Status)：当前已实现能力与尚待验证的边界。

### 使用与交互

- [界面导览](https://github.com/zhaiyanqi/vclogg2/wiki/Interface)、[交互操作](https://github.com/zhaiyanqi/vclogg2/wiki/Interaction)、[键盘快捷键](https://github.com/zhaiyanqi/vclogg2/wiki/Keyboard-Shortcuts)。
- [搜索与过滤](https://github.com/zhaiyanqi/vclogg2/wiki/Search-and-Filters)、[动态日志](https://github.com/zhaiyanqi/vclogg2/wiki/Live-Logs)、[结果导出](https://github.com/zhaiyanqi/vclogg2/wiki/Result-Export)。
- [标记与高亮](https://github.com/zhaiyanqi/vclogg2/wiki/Marks-and-Highlighting)、[行内文字标记](https://github.com/zhaiyanqi/vclogg2/wiki/Row-Tags)。
- [设置与个性化](https://github.com/zhaiyanqi/vclogg2/wiki/Settings)、[云端过滤器](https://github.com/zhaiyanqi/vclogg2/wiki/Cloud-Filters)、[数据与隐私](https://github.com/zhaiyanqi/vclogg2/wiki/Data-and-Privacy)。

### 开发与技术

- [技术栈](https://github.com/zhaiyanqi/vclogg2/wiki/Technology-Stack)、[源码构建](https://github.com/zhaiyanqi/vclogg2/wiki/Building)、[构建与发布](https://github.com/zhaiyanqi/vclogg2/wiki/Build-and-Release)。
- [架构总览与实现约束](https://github.com/zhaiyanqi/vclogg2/wiki/Architecture)、[UI 布局层级及稳定控件编号](https://github.com/zhaiyanqi/vclogg2/wiki/UI-Layout)。
- [搜索性能设计及历史测量](https://github.com/zhaiyanqi/vclogg2/wiki/Search-Performance)、[贡献指南](https://github.com/zhaiyanqi/vclogg2/wiki/Contributing)。

### 维护与验收

- [排障](https://github.com/zhaiyanqi/vclogg2/wiki/Troubleshooting)、[文档组织和迁移索引](https://github.com/zhaiyanqi/vclogg2/wiki/Documentation)。
- [界面验收规格](https://github.com/zhaiyanqi/vclogg2/wiki/UI-Acceptance)、[过滤器分支验收清单](https://github.com/zhaiyanqi/vclogg2/wiki/Filter-Branch-Acceptance)。
- [历史档案索引](https://github.com/zhaiyanqi/vclogg2/wiki/Archive)：阶段性交付记录，保留日期与验证范围。

## 版本与阅读约定

设置、搜索标签、布局、依赖与构建说明更新至 2026-10-09 的源码 `b14c8db`；历史验收页保留原日期与范围。AI 和工作区侧边栏已移除，旧说明转入历史档案。Wiki 描述的是文档标明的源码基线，已下载的 Release 可能更早；请先在“帮助 → 关于”核对版本。**已实现不等于已经在所有平台、显卡、窗口尺寸和日志负载下验收。** 平台安装、安全提示及性能以对应发行包和实际环境为准。

正文以简体中文维护，页面文件名使用稳定的英文短名，便于链接和 Git 维护。右侧分类导航在每页都可使用。README 保留产品入口，详细文档统一在 Wiki 维护。
