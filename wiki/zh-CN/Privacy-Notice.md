# VCLogg2 隐私说明

[English](../en/Privacy-Notice.md) · [Home / 首页](Home.md)

**本地日志与应用数据。** VCLogg2 在本机读取、索引和搜索你打开的日志。普通浏览和搜索以只读方式访问源文件。应用会在本地保存设置、最近打开的文件路径、搜索历史、标记、会话恢复信息、索引和临时结果。编辑并保存时会写入源文件，导出时会写入你选定的位置。

**诊断与清理。** 应用在本地保存崩溃报告，并在应用数据目录的 `logs/open` 下记录启动和文件打开诊断。这些诊断可能包含命令行参数、文件路径和运行错误。应用不会自动上传这些报告。你可以在设置中清理可重建索引和应用诊断。Windows 安装版卸载时会保留应用数据目录。

**检查更新。** 当你选择“帮助 → 检查更新”时，应用通过 HTTPS 从项目官网请求版本清单。请求标头包含当前 VCLogg2 版本，打开的日志和搜索历史保留在本机。新版由你自行下载和安装。官网由 GitHub Pages 托管，GitHub 会出于安全目的记录访问者的 IP 地址。 [GitHub Pages 数据收集说明](https://docs.github.com/en/pages/getting-started-with-github-pages/what-is-github-pages#data-collection) · [GitHub 隐私声明](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement)

**可选云端过滤器。** 本地搜索和过滤器可以离线使用。使用云端功能时，应用会连接已配置的服务器。注册会发送你填写的工号或昵称及本机生成的客户端标识。根据你选择的操作，服务器还会收到目录查询、过滤器下载记录，或你发布的名称、匹配规则和备注。发布的内容可能对该服务的其他用户可见。服务器端的数据保留和访问权限由其运营方管理。会话 Cookie 和 CSRF 令牌保存在本地会话文件中，应用未对这些文件额外加密。

**隐私问题与诊断分享。** 一般问题可通过项目的 GitHub Issues 提出。GitHub Issues 是公开的。附上诊断材料前，请检查并脱敏文件路径、日志片段和搜索词，避免公开会话文件或凭据。 [项目 Issues](https://github.com/zhaiyanqi/vclogg2/issues)
