# 项目概况

## 项目定位

VCLogg2 是一个开源、本地运行的原生桌面日志查看器。它围绕“打开大日志 → 搜索信号 → 查看上下文 → 标记与归档结果”设计，适合开发排错、服务日志分析、客户端与设备日志调查，以及多个文件之间的时间线对照。

主要界面由 Rust 与 GPUI Kit 绘制，通过统一入口使用 GPUI 和输入、菜单、对话框等桌面组件；不是基于浏览器的日志编辑页面。项目采用 Apache-2.0 许可证，借鉴 klogg 在大文件日志浏览与搜索方面的思路。

## 典型工作流

| 场景 | 建议路径 |
| --- | --- |
| 单个很大的服务日志 | 打开文件，等待索引就绪，关键词筛选，再从结果跳转源行 |
| 多个模块同时报错 | 打开多文件，配置全局搜索参与列表，按文件查看结果 |
| 尚未打开的日志目录 | 选择目录范围，设置后缀、递归和隐藏目录规则后搜索 |
| 复现过程中的实时日志 | 将日志保持为前台活动标签，开启末尾跟随 |
| 保存排查结论 | 标记关键行、添加文字标记；将已提交结果导出为文件 |
| 重复使用业务查询 | 保存为预定义过滤器，可选连接兼容服务共享过滤器 |

## 核心能力

- **大文件处理**：后台建立行索引，提供有界打开预览，正文与结果使用虚拟列表按需显示；索引缓存可减少重复准备成本。
- **检索**：多字面量、正则、大小写选项，支持当前文件、选定已打开文件及目录；另有作用于当前阅读区域的页内查找。
- **分析**：源行跳转、文本与多行选择、行标记、文字标记、颜色标签、日志级别着色、按文件限制搜索起止行。
- **工作区**：多标签、多原生窗口、拖放、标签复制和移动、收藏、最近文件和恢复入口。
- **导出**：导出完整已提交结果，跨文件分组输出，支持指定格式的日志时间戳合并。
- **持久化**：SQLite/WAL 保存设置和文件会话；云端过滤器是可选工作流。

## 平台与分发

| 平台 | 当前构建流程的目标 | 用户产物 |
| --- | --- | --- |
| Windows | windows-2022 runner，x86_64 / MSVC；README 面向 Windows 10/11 x64 | 便携 ZIP、当前用户安装 EXE |
| macOS | macos-15 runner，Apple Silicon / aarch64 | DMG |
| Linux | Ubuntu 22.04 runner，x86_64 | tar.gz 与用户级安装脚本 |

以上是当前仓库定义的构建与发行目标，不是其他系统版本、Intel Mac 或 ARM Linux 的兼容性承诺。具体包名、数据目录和签名见[安装](https://github.com/zhaiyanqi/vclogg2/wiki/Installation)与[构建与发布](https://github.com/zhaiyanqi/vclogg2/wiki/Build-and-Release)。

## 当前边界

- 源日志是只读输入，行标记和注释不会写回日志正文；应用不是通用文本编辑器。
- “检查更新”打开 GitHub Releases，客户端不自动检测版本、下载或安装升级。
- 自动监测只覆盖前台活动窗口的当前 Ready 文档；目录搜索不持续发现新文件。
- 快速追加校验可能漏检“未采样中部改写后继续追加”，需要严格重建时使用 F5。
- 大文件虚拟显示仍需要索引和搜索结果存储，不能据此承诺恒定内存或任意文件都即时完成。
- 云端过滤器服务不包含在这个 Rust workspace 中，服务兼容性与连通性需要单独确认。
- 当前 macOS 发布流程使用 ad-hoc 签名，没有 Developer ID 公证；Windows 签名取决于发布配置。

参见[功能状态](https://github.com/zhaiyanqi/vclogg2/wiki/Implementation-Status)和[历史档案](https://github.com/zhaiyanqi/vclogg2/wiki/Archive)，区分功能落地、历史基准与实际平台验收。


源码依据：[Cargo.toml](https://github.com/zhaiyanqi/vclogg2/blob/65fa1d4fa7f44984825fa84758515a03d2157aeb/Cargo.toml) · [Cargo.toml](https://github.com/zhaiyanqi/vclogg2/blob/65fa1d4fa7f44984825fa84758515a03d2157aeb/crates/vclogg-app/Cargo.toml) · [release-build.yml](https://github.com/zhaiyanqi/vclogg2/blob/65fa1d4fa7f44984825fa84758515a03d2157aeb/.github/workflows/release-build.yml)。
