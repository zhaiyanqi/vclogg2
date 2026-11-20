# VCLogg2 Privacy Notice

[简体中文](../zh-CN/Privacy-Notice.md) · [Home / 首页](Home.md)

**Local logs and application data.** VCLogg2 reads, indexes, and searches the logs you open on your device. Browsing and searching use read-only access to source files. The app stores settings, recent file paths, search history, marks, session recovery data, indexes, and temporary results locally. Saving an edit writes to the source file. Exporting writes to the location you choose.

**Diagnostics and cleanup.** The app saves crash reports locally and records startup and file-opening diagnostics under `logs/open` in its application data directory. These diagnostics may contain command-line arguments, file paths, and runtime errors. The app does not upload these reports automatically. You can clear rebuildable indexes and application diagnostics in Settings. Uninstalling the Windows installer edition leaves the application data directory in place.

**Update checks.** When you choose **Help → Check for updates**, the app requests a version manifest from the project website over HTTPS. The request header includes the current VCLogg2 version. Open logs and search history stay on your device. You choose whether to download and install a new version. GitHub Pages hosts the project website, and GitHub logs visitors’ IP addresses for security purposes. [GitHub Pages data collection](https://docs.github.com/en/pages/getting-started-with-github-pages/what-is-github-pages#data-collection) · [GitHub Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement)

**Optional cloud filters.** Local search and filters work offline. When you use cloud features, the app connects to the configured server. Registration sends the employee ID or nickname you enter and a client identifier generated on your device. Depending on the action you take, the server also receives catalog queries, filter download records, or the names, matching rules, and notes you publish. Published content may be visible to other users of that service. The server operator manages server-side retention and access. The app stores session cookies and CSRF tokens in local session files without additional encryption.

**Privacy questions and diagnostic sharing.** You can raise general questions in the project’s public GitHub Issues. Before attaching diagnostics, review and redact file paths, log excerpts, and search terms. Keep session files and credentials private. [Project issues](https://github.com/zhaiyanqi/vclogg2/issues)
