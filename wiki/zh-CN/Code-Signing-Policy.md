# VCLogg2 代码签名政策（Code signing policy）

[English](../en/Code-Signing-Policy.md) · [Home / 首页](Home.md)

本政策适用于 VCLogg2 通过 GitHub Releases 分发的 Windows 桌面版，包括安装程序 `setup.exe` 和便携包内的 `vclogg2.exe`。签名状态以各版本的实际文件为准。已签名的文件可在 Windows 文件属性的“数字签名”选项卡中查看签名者。 [官方发布页](https://github.com/zhaiyanqi/vclogg2/releases)

VCLogg2 计划采用 SignPath 为开源项目提供的代码签名服务。接入后，签名请求将使用 GitHub Actions 从项目公开源码构建的产物，每次正式发布的签名请求均由指定批准人审核。参与该流程的维护者须在 GitHub 和 SignPath 启用双因素认证。外部贡献在合并前由指定评审者审阅。

| 角色 | 维护者 |
| --- | --- |
| 提交者（Committer） | [zhaiyanqi](https://github.com/zhaiyanqi) |
| 评审者（Reviewer） | [zhaiyanqi](https://github.com/zhaiyanqi) |
| 签名批准人（Signing approver） | [zhaiyanqi](https://github.com/zhaiyanqi) |

桌面应用的数据处理方式见 [VCLogg2 隐私说明](Privacy-Notice.md)。
