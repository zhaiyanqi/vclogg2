# VCLogg2 Code signing policy

[简体中文](../zh-CN/Code-Signing-Policy.md) · [Home / 首页](Home.md)

This policy covers Windows desktop releases distributed through GitHub Releases, including the `setup.exe` installer and `vclogg2.exe` inside the portable ZIP. Signing status can vary by release. For signed files, the signer is shown on the **Digital Signatures** tab in Windows file properties. [Official releases](https://github.com/zhaiyanqi/vclogg2/releases)

VCLogg2 plans to use SignPath’s code signing service for open-source projects. Once enabled, signing requests will use artifacts built by GitHub Actions from the project’s public source code. A designated approver will review each release signing request. Maintainers involved in this process must enable two-factor authentication on GitHub and SignPath. A designated reviewer reviews external contributions before they are merged.

| Role | Maintainer |
| --- | --- |
| Committer | [zhaiyanqi](https://github.com/zhaiyanqi) |
| Reviewer | [zhaiyanqi](https://github.com/zhaiyanqi) |
| Signing approver | [zhaiyanqi](https://github.com/zhaiyanqi) |

See the [VCLogg2 Privacy Notice](Privacy-Notice.md) for details on how the desktop application handles data.
