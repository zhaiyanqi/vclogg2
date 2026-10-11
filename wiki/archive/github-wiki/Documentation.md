# 文档组织与维护

## 单一维护位置

主仓库保留英文 `README.md` 与中文 `README.zh-CN.md`，用于产品简介、下载和文档入口；详细项目说明、使用、技术和验收资料在这个 Wiki 维护。`doc/` 继续承载 GitHub Pages 官网与静态资源。

第三方源码自带 README、补丁来源 README、图标资源 README 和许可证保留在源码旁，避免破坏上游归属和素材维护上下文。它们是随源码分发的局部说明，不再作为项目使用文档目录。

## 组织约定

- `Home.md` 是首页，按阅读任务给出入口；`_Sidebar.md` 提供跨页分类导航；`_Footer.md` 提供项目入口和版本提醒。
- 页面使用稳定英文短名，标题和正文以简体中文为主；更名时同步所有导航及 README/官网链接。
- 新用户操作放在使用区，实现约束放在开发区，阶段性检查与数据放在档案区。
- 图片随 Wiki 放在 `assets/`；不链接开发机绝对路径。本次迁移的源码证据使用固定提交链接，便于核对。
- UI 结构在 `UI-Layout` 维护，沿用已有 A 编号，不因为迁移重编号。

该结构采用 GitHub 官方支持的[首页及页面编辑](https://docs.github.com/en/communities/documenting-your-project-with-wikis/adding-or-editing-wiki-pages)、[自定义侧栏与页脚](https://docs.github.com/en/communities/documenting-your-project-with-wikis/creating-a-footer-or-sidebar-for-your-wiki)机制。

## 在本地编辑 Wiki

Wiki 已初始化后，它是独立 Git 仓库，可放在主仓库旁边：

```bash
git clone https://github.com/zhaiyanqi/vclogg2.wiki.git
cd vclogg2.wiki
git pull --ff-only
# 编辑对应 Markdown、检查页面链接与 assets
git diff --check
git add Home.md _Sidebar.md
git commit -m "docs: update wiki navigation"
git push origin master
```

本 Wiki 当前分支为 `master`；主项目默认分支是 `main`，二者不要混用。推送 Wiki 不会触发主项目 Release 工作流；主仓库的 README、网站和删除旧文档需要另外提交。

## 何时更新哪页

| 变更 | 同步文档 |
| --- | --- |
| 功能范围或限制 | Implementation-Status、相关使用专题 |
| 快捷键、默认设置 | Keyboard-Shortcuts、Settings |
| UI 节点、菜单、交互 | UI-Layout、Interface、Interaction |
| 分层、状态和异步协议 | Architecture、Technology-Stack |
| 构建、打包、签名或 Actions | Building、Build-and-Release、Installation |
| 数据路径、缓存或隐私 | Data-and-Privacy |
| 新的性能/平台验收 | 对应日期档案及 Archive 索引 |

## 官网与 README

英文官网 `doc/index.html` 与中文官网 `doc/zh.html` 共用 `doc/assets/styles.css` 和 `doc/assets/main.js`。两种语言的内容、章节编号和语言切换链接应同步，切换保留当前章节。各页分别维护 lang、canonical、Open Graph 和 JSON-LD，通过 hreflang 关联，x-default 指向英文首页；站点地图包含两种入口。

README 和官网所需图片继续放在 `doc/assets/`。英文分享图使用 `readme-hero.jpg`，中文使用 `readme-hero-zh-CN.jpg`；主题规格预览分别有中英文 SVG。Wiki 引用的规格 SVG 在自身 assets 中保留副本，规格变更时同步两处，避免把独立仓库中的链接指向已移动的资源。

GitHub Pages 从主仓库 `doc/` 发布；不要把 Wiki checkout、本地协作资料或应用数据放入这个发布目录。

## 旧路径迁移索引

| 主仓库原文件 | 现在的位置 |
| --- | --- |
| `doc/architecture.md` | [Architecture](https://github.com/zhaiyanqi/vclogg2/wiki/Architecture) |
| `doc/delivery.md` | [Build-and-Release](https://github.com/zhaiyanqi/vclogg2/wiki/Build-and-Release) |
| `doc/dynamic-log-files.md` | [Live-Logs](https://github.com/zhaiyanqi/vclogg2/wiki/Live-Logs) |
| `doc/feature-parity.md` | [Archive-Feature-Acceptance-2026-08-31](https://github.com/zhaiyanqi/vclogg2/wiki/Archive-Feature-Acceptance-2026-08-31) |
| `doc/filter-uuid-branch-manual-checklist.md` | [Filter-Branch-Acceptance](https://github.com/zhaiyanqi/vclogg2/wiki/Filter-Branch-Acceptance) |
| `doc/migration-status.md` | [Implementation-Status](https://github.com/zhaiyanqi/vclogg2/wiki/Implementation-Status) |
| `doc/row-tags.md` | [Row-Tags](https://github.com/zhaiyanqi/vclogg2/wiki/Row-Tags) |
| `doc/search-performance.md` | [Search-Performance](https://github.com/zhaiyanqi/vclogg2/wiki/Search-Performance) |
| `doc/ui-layout.md` | [UI-Layout](https://github.com/zhaiyanqi/vclogg2/wiki/UI-Layout) |
| `doc/ui-polish-acceptance.md` | [UI-Acceptance](https://github.com/zhaiyanqi/vclogg2/wiki/UI-Acceptance) |
| `doc/usage.md` | [User-Guide](https://github.com/zhaiyanqi/vclogg2/wiki/User-Guide) |
| `doc/PERFORMANCE_DELIVERY_2026-08-31.md` | [Archive-UI-Performance-2026-08-31](https://github.com/zhaiyanqi/vclogg2/wiki/Archive-UI-Performance-2026-08-31) |
| `doc/README.md` | [Home](https://github.com/zhaiyanqi/vclogg2/wiki/Home) |
| `doc/README.zh-CN.md` | 主仓库根目录 `README.zh-CN.md`（保留 README） |

原详细状态表另保存在 [Archive-Implementation-Status](https://github.com/zhaiyanqi/vclogg2/wiki/Archive-Implementation-Status)。新增专题已纳入[首页](https://github.com/zhaiyanqi/vclogg2/wiki/Home)与侧栏。旧 Markdown 删除前先发布 Wiki；源码 Git 历史保留迁移前版本。
