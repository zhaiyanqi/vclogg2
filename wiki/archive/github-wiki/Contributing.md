# 贡献指南

欢迎通过 [Issue](https://github.com/zhaiyanqi/vclogg2/issues)讨论问题和改进，通过 Pull Request 提交变更。开始前阅读[源码构建](https://github.com/zhaiyanqi/vclogg2/wiki/Building)和[架构](https://github.com/zhaiyanqi/vclogg2/wiki/Architecture)，先定位行为归属，再做范围明确的修改。

## 代码入口

| 变更 | 主要目录 |
| --- | --- |
| 文件、索引、编码、搜索算法、结果集合 | `crates/vclogg-core/src/` |
| SQLite、schema、会话记录、缓存生命周期 | `crates/vclogg-data/src/` |
| 窗口、标签、搜索调度、日志显示与交互 | `crates/vclogg-app/src/workspace/` |
| 对话框、动作与设置 | `crates/vclogg-app/src/` 中对应模块 |
| 构建、安装、签名、发布 | `scripts/` 与 `.github/workflows/` |
| 产品官网 | `doc/index.html`、`doc/zh.html`、`doc/assets/` |
| 使用与开发文档 | 独立 GitHub Wiki 仓库 |

## 修改原则

保持 core/data 与 GPUI 的依赖边界，避免把算法或 SQLite 塞进 Workspace。异步结果按文档身份、源行与 revision 校验，不能把投影位置当成稳定源行。日志按只读输入处理，显式导出才写用户目标。

界面变更同步[UI 布局](https://github.com/zhaiyanqi/vclogg2/wiki/UI-Layout)中的节点、控件类和备注，保留稳定编号。功能、构建或持久化行为变化，同步对应 Wiki 专题与[功能状态](https://github.com/zhaiyanqi/vclogg2/wiki/Implementation-Status)。

## 验证与提交说明

按改动选择格式/差异/架构检查、编译分析、Clippy、相关回归或平台验证。现有测试可通过 `cargo test --workspace --locked` 运行；部分基准是显式忽略的手动项目。构建和检查脚本见[源码构建](https://github.com/zhaiyanqi/vclogg2/wiki/Building)。

PR 描述说明问题触发条件、修改后的行为、实际执行的验证和剩余限制。静态通过不代表 GUI、签名、安装器或目标平台通过；历史数据不能冒充本次验证。

提交前排除构建产物、应用数据、凭据及本地协作资料。第三方代码保留原许可证和补丁来源。主仓库许可见[LICENSE](https://github.com/zhaiyanqi/vclogg2/blob/main/LICENSE)。

## 文档变更

Wiki 使用独立 Git 历史，主仓库的 PR 不会自动包含 Wiki 修改。在代码 PR 中注明相应 Wiki 页和文档修订，协调先后顺序；README 与官网链接变更仍在主仓库提交。页面命名、导航和迁移对应关系见[文档维护](https://github.com/zhaiyanqi/vclogg2/wiki/Documentation)。
