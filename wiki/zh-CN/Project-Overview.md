# 项目概况

[English](../en/Project-Overview.md) · [Home / 首页](Home.md)

VCLogg2 是 Apache-2.0 开源桌面应用，用于读取与搜索大型本地日志。当前采用 Rust 2024 和 GPUI Kit 0.7.1，分为 core、data、app 三层。可选的 `server/` Go 模块独立提供云端过滤器管理与分发。

当前支持索引浏览、当前/全局/目录搜索、多文件与多搜索标签、动态刷新、注释、结果导出、设置与会话恢复。支持的文件可以显式编辑，不设固定文件大小限制。AI/MCP/记忆模块与旧工作区侧栏已移除。

能力边界见[实现状态](Implementation-Status.md)，源码导航见[架构](Architecture.md)，平台环境见[构建](Building.md)。发行目标为 Windows x64、macOS Apple Silicon 和 Linux x86_64，不代表支持所有旧系统或显卡驱动。
