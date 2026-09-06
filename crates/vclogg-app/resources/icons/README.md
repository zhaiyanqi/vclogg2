# 内置应用图标

四款图标均来自本次 ImageGen 设计，母版为 1024 × 1024 RGBA PNG，保留透明背景：

| 设置项 | 母版 | Windows 资源 ID |
| --- | --- | --- |
| A · 软萌立体（默认，原始站姿） | `soft.png` | 1 |
| A · 圆润坐姿 | `compact.png` | 2 |
| B · 清爽插画 | `illustration.png` | 3 |
| C · 可爱贴纸 | `sticker.png` | 4 |

`app_icon.rs` 在编译期嵌入四张 PNG，设置预览与关于页共享缓存的 `gpui::Image`。
Windows 原生窗口从资源表加载共享图标句柄，四款 ICO 均包含 16、20、24、32、40、48、64、128、256 像素版本。

修改母版后，在仓库根目录运行 `python3 scripts/generate-app-icons.py`（需要 Pillow）。脚本同步三款备用 ICO、默认 Windows ICO/PNG 和官网 PNG。macOS 打包脚本继续从默认 PNG 生成 ICNS；Linux 安装包携带全部 PNG，安装器为备用图标创建 `NoDisplay=true` 的桌面入口，供 Wayland 根据应用 ID 识别。

运行时选择只改变应用窗口、macOS Dock 及支持的 Linux 桌面显示；不修改签名后的 macOS 应用包、Windows 可执行文件、文件关联或已有固定快捷方式。默认安装资源与官网始终使用原版 A。
