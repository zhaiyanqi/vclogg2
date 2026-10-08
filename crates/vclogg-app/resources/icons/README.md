# 内置应用图标

四款图标均来自本次 ImageGen 设计，母版为 1024 × 1024 RGBA PNG，保留透明背景：

| 设置项 | 母版 | Windows 资源 ID |
| --- | --- | --- |
| A · 软萌立体（默认，原始站姿） | `soft.png` | 1 |
| A · 圆润坐姿 | `compact.png` | 2 |
| B · 清爽插画 | `illustration.png` | 3 |
| C · 可爱贴纸 | `sticker.png` | 4 |

`app_icon.rs` 通过编译期 `cfg` 按目标平台选择 PNG，未选中的尺寸不嵌入可执行文件。设置预览与关于页共享缓存的 `gpui::Image`；透明背景与四款选择不变。

| 平台 | 嵌入 PNG | 原生系统/安装资源 |
| --- | --- | --- |
| Windows | `runtime/128/`，用于约 64px 设置预览、48px 关于页，覆盖 2× 显示 | 资源表内四款 ICO，包含 16、20、24、32、40、48、64、128、256px；不打包外置 PNG |
| macOS | `runtime/512/`，预览与可放大的 Dock 图标共用 | 默认安装 ICNS 仍从 1024px 母版生成 |
| Linux | `runtime/128/`，预览与 128px X11 窗口图标共用 | 外置 `runtime/512/` PNG，供桌面/Wayland 使用 |

Windows 原生窗口从资源表加载共享图标句柄，不使用预览 PNG。预览在高于 2× 的界面缩放下由渲染器放大；原生系统图标保留最高 256px。

修改母版后，在仓库根目录运行 `python3 scripts/generate-app-icons.py`（需要 Pillow）。脚本从母版以 Lanczos 缩放生成 `runtime/` 资源并优化 PNG 编码，同时同步三款备用 ICO、默认 Windows ICO/PNG 和官网 PNG。派生文件随源码提交，正常编译无需运行生成脚本。不要从缩小后的资源反向覆盖母版。

macOS 打包脚本继续从 1024px 默认 PNG 生成完整 ICNS。Linux 安装包使用 512px 派生资源，与安装器的 `hicolor/512x512/apps` 目录匹配；默认款仅放在包根目录 `vclogg2.png`，`icons/` 只携带三款备用图标，避免重复打包默认款。安装器为备用图标创建 `NoDisplay=true` 的桌面入口，供 Wayland 根据应用 ID 识别。

运行时选择只改变应用窗口、macOS Dock 及支持的 Linux 桌面显示；不修改签名后的 macOS 应用包、Windows 可执行文件、文件关联或已有固定快捷方式。默认安装资源与官网始终使用原版 A。
