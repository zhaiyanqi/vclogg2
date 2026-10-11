# 文档维护与 GitHub Pages

[English](../en/Documentation.md) · [Home / 首页](Home.md)

## 文档工具选择

“get book”可能指 **GitBook**，当前产品支持托管文档和 GitHub/GitLab 同步。仓库同步不等于在本仓库构建静态产物并发布到 GitHub Pages。

| 方案 | 案例与部署 | 本项目适配 |
| --- | --- | --- |
| mdBook | Rust 官方教程和 mdBook 自身文档；Markdown → 静态 HTML → Pages | 本次采用，适合 Rust 项目，自带目录和搜索，无需前端依赖树 |
| Docusaurus | Docusaurus 官方文档；Node 构建静态站后部署 Pages | 适合复杂 React 文档站，配置和依赖更多 |
| GitBook | 文档托管与仓库同步 | 适合托管编辑，本项目同仓 Pages 部署无需引入 |

参考：[mdBook 简介](https://rust-lang.github.io/mdBook/)、[mdBook CI](https://rust-lang.github.io/mdBook/continuous-integration.html)、[Docusaurus 部署](https://docusaurus.io/docs/deployment)、[GitBook Git Sync](https://www.gitbook.com/features/git-sync)、[GitHub 自定义 Pages 工作流](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages)。核对日期 2026-10-11；本项目固定 mdBook **0.4.52** 用于可复现构建，不表示它是最新版本。

## 源文件与产物

```text
wiki/README.md          仓库文档入口
wiki/en/               英文页面、SUMMARY.md、book.toml
wiki/zh-CN/            对应中文页面、SUMMARY.md、book.toml
wiki/archive/          旧 Wiki 原始快照与 SHA-256 清单，不发布
 doc/                  现有官网与资源
scripts/build-docs.py   校验、构建两种语言、合并官网、检查产物
_site/                 生成的 Pages 产物，Git 忽略
```

当前页面都有跳转同主题另一语言的链接。文件名与章节顺序成对维护，Markdown 链接在仓库内可用，mdBook 将其转换为 HTML 链接。历史证据保留原始语言。

## 本地构建与预览

安装 Python 3.9+ 和 mdBook 0.4.52，可使用[官方预编译发行包](https://github.com/rust-lang/mdBook/releases/tag/v0.4.52)，或通过 Cargo 安装文档工具：

```sh
cargo install mdbook --version 0.4.52 --locked
python3 scripts/build-docs.py
python3 -m http.server 8000 --directory _site
```

打开 `http://localhost:8000/wiki/` 选择 English 或简体中文。Windows 若无 `python3` 命令可用 `py -3`。`MDBOOK` 可指定工具路径。只检查源文档、不调用 mdBook：`python3 scripts/build-docs.py --check`。

这只构建文档，不编译、测试或启动桌面/服务端。`_site/` 为重新生成目录，源文档修改保留在 `wiki/`、`doc/`。

## 部署到现有 Pages

仓库 **Settings → Pages → Build and deployment** 选择 **GitHub Actions**，并确保 `github-pages` 环境允许 main。将修改合并/推送到 main，或手动运行 **Deploy GitHub Pages**。工作流会：

1. 检出主仓库，不克隆旧 Wiki。
2. 下载固定版本 mdBook Linux 工具并验证 SHA-256。
3. 检查源文档、构建双语站点，将官网一起放入 `_site/`。
4. 上传这一份产物，通过既有 Pages 环境部署。

文档相关 PR 也会构建检查，但没有部署权限。推送/手动部署使用 `pages: write`、`id-token: write`，不需要个人访问令牌或独立 `gh-pages` 分支。

部署成功后的预期地址：

- 官网：`https://zhaiyanqi.github.io/vclogg2/`
- 语言入口：`https://zhaiyanqi.github.io/vclogg2/wiki/`
- 英文：`https://zhaiyanqi.github.io/vclogg2/wiki/en/`
- 中文：`https://zhaiyanqi.github.io/vclogg2/wiki/zh-CN/`

官网与文档必须一起发布：第二份 Pages 部署会替换第一份产物。触发路径包含 `doc/**`、`wiki/**`、文档构建脚本和工作流本身。公开目录不存凭据或运行数据。

Pages 不能重定向 GitHub 的旧 Wiki 地址。原始快照与可选旧 Wiki 迁移提示见[归档](Archive.md)。本地构建成功不表示远端已部署，推送后需查看 Actions 部署结果。

## 内容维护

行为、依赖和模块职责以当前代码为准。中英文对应页和目录同步修改；UI 编号稳定保留。历史测试结果放归档，新检查如实记录。构建发布命令与脚本一致；发布前运行源检查和完整文档构建。

2026-10-11 已通过仓库 Pages API 核对，发布类型已经是 `workflow`（GitHub Actions），本次未修改远程设置。
