# Local data and privacy

[简体中文](../zh-CN/Data-and-Privacy.md) · [Home / 首页](Home.md)

Normal reading, searches and annotations do not modify logs. Explicit [editing and saving](Editing.md) replaces source text, and export writes a chosen destination. Closing tabs or deleting ordinary history does not delete source logs.

| Platform | Data root |
| --- | --- |
| Windows portable / default Debug | `VCLogg2` next to the executable |
| Windows installed | Chosen during installation, default `%LOCALAPPDATA%\VCLogg2` |
| macOS | System data/cache/temp directories; data normally `~/Library/Application Support/VCLogg2` |
| Linux | System/XDG data/cache/temp directories |

Windows `vclogg2-data-dir.txt` specifies an absolute data root. Missing means portable mode; invalid/unwritable means startup failure, not fallback. Windows instance routing is scoped to user and data root.

Stored records include `sessions/vclogg2-state.db` (SQLite settings, paths, searches and recovery), rebuildable indices, public cloud page caches, private cloud session JSON, crash reports and temporary documents. Current schema is v22. Unix session directory/file permissions are 0700/0600; Windows follows data-root permissions. Session JSON has no extra encryption and is not stored in a system keychain.

Panic reports include build/platform/thread/source location/backtrace; normal startup retains the latest 20. Application logs use an in-memory buffer limited to 20,000 entries and 8 MiB, exported only on request rather than continuously written to a permanent log file.

Exit all processes sharing the root before backup/migration, then copy the full data directory. Do not copy only a live SQLite main file and omit WAL. Storage settings can clear rebuildable indices; history cleanup protects open, pinned or annotated/marked records. Temporary-file cleanup validates application ownership and current use.

Before sharing diagnostics, inspect paths, searches, filters and log snippets for private content. Never attach raw cloud session files, Cookie/CSRF values, release credentials or an unreviewed state database.

Update checks contact the public website only when requested. Version metadata, success time and failure backoff are stored in SQLite; no log content is sent. Successful checks show release notes and a download-page button; failures open GitHub Releases in the browser. See [update checks](Build-and-Release.md#update-checks).
