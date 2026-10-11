# 贡献指南

[English](../en/Contributing.md) · [Home / 首页](Home.md)

按[架构](Architecture.md)划分修改职责。行为、设置、模块归属、脚本或部署变化时，同步更新中英文对应页。控件移动或重命名保留稳定编号。

提交标题使用英文 Conventional Commit，例如 `docs(wiki): migrate bilingual documentation`，正文包含含义一致的 `English:` 与 `中文:` 两段。启用仓库模板：

```sh
git config --local commit.template .gitmessage
```

只报告实际执行的验证。`git diff --check` 和 `bash scripts/check-architecture.sh` 是静态检查；`scripts/check.sh` 包含编译分析及 Clippy，应用构建、测试、基准、真实窗口验收是独立层级。命令见[构建](Building.md)。

不提交运行数据库、云端会话、私钥或本地协作记录。现有兼容补丁见[技术栈](Technology-Stack.md)，普通应用修改优先使用上游公开 API。
