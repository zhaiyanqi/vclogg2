# 技术栈

## 工作区与依赖

| 层 / 技术 | 在项目中的作用 |
| --- | --- |
| Rust stable、Edition 2024、Cargo workspace | 原生程序、三层 crate、统一依赖与构建 |
| GPUI Kit 的 GPUI / 平台层 | 窗口、事件、实体、布局、文本与 GPU 呈现 |
| `gpui_kit::component`、`gpui_kit::base` 与资源 | 菜单、输入、对话框、设置和主题等桌面组件 |
| Rayon | CPU 搜索并行和文件准备工作 |
| Aho-Corasick、regex、memchr | 多字面量匹配、正则与字节扫描 |
| chardetng、encoding_rs | 自动编码检测与传统编码转码 |
| roaring / RoaringTreemap | 压缩源行集合、命中和选择集合 |
| rusqlite（bundled SQLite） | SQLite/WAL、会话和设置事务、schema 迁移 |
| notify | 当前活动日志的原生目录变化通知 |
| SHA-256 | 内容/缓存校验及稳定摘要 |
| serde、serde_json、UUID | 配置与过滤器交换、稳定身份 |
| reqwest、Rustls、url | 可选云端过滤器 HTTP 接口 |
| async-channel、GPUI Task | 后台工作与前台发布协调 |
| log、chrono | 有界应用日志、日期和构建信息展示 |
| windows-sys、objc2、x11rb、raw-window-handle | Windows/macOS/Linux 原生集成 |
| Bash、PowerShell、Inno Setup、GitHub Actions | 本地构建、打包、签名和发布 |

## 三层组织

```text
crates/vclogg-app     Cargo 包名和可执行文件名：vclogg2
    ├── vclogg-core  文件、索引、解码、搜索、压缩结果集
    └── vclogg-data  SQLite、缓存生命周期、路径编码、状态记录
```

core 不依赖 app/data，data 不依赖 app，core/data 不依赖 GPUI。`vclogg-app` 是目录名称，执行 Cargo 包选择时应使用 `-p vclogg2`。

## 版本策略

`rust-toolchain.toml` 选择 stable，并安装 rustfmt 与 Clippy。应用通过精确依赖 `gpui-kit = "=0.7.1"` 使用统一入口，GPUI、组件、基础控件与资源从 `gpui_kit` 导入；实际传递依赖以提交的 `Cargo.lock` 为准。

## 本地兼容层

- `vendor/block`：现有 block 0.1.6 未来兼容性修复，通过 `[patch.crates-io]` 使用。
- `compat/xim-ctext`：应用自有桥接包，保留旧链路要求的 0.3.0 包身份，重新导出官方 0.4.1 解码器并转发 `std` feature，用于 X11 中文 Compound Text 的 GB2312 解码。不是外部解码器源码副本，`zed-xim` 保持 registry 版本。
- 当前没有 `vendor/gpui-base` 或 `vendor/gpui-component` 补丁；历史说明不代表当前依赖状态。

构建环境和命令见[源码构建](https://github.com/zhaiyanqi/vclogg2/wiki/Building)。

源码依据：[Cargo.toml](https://github.com/zhaiyanqi/vclogg2/blob/b14c8db4862c2495816d63bde34c80210a793e05/Cargo.toml) · [Cargo.lock](https://github.com/zhaiyanqi/vclogg2/blob/b14c8db4862c2495816d63bde34c80210a793e05/Cargo.lock) · [Cargo.toml](https://github.com/zhaiyanqi/vclogg2/blob/b14c8db4862c2495816d63bde34c80210a793e05/crates/vclogg-app/Cargo.toml) · [README.md](https://github.com/zhaiyanqi/vclogg2/blob/b14c8db4862c2495816d63bde34c80210a793e05/compat/xim-ctext/README.md)。
