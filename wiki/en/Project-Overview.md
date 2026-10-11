# Project overview

[简体中文](../zh-CN/Project-Overview.md) · [Home / 首页](Home.md)

VCLogg2 is an Apache-2.0 desktop application for reading and searching large local logs. The current workspace uses Rust 2024 and GPUI Kit 0.7.1, with separate core, data and app crates. The optional `server/` Go module provides cloud filter administration and distribution independently of the desktop build.

The application supports indexed browsing, current/global/directory searches, multiple file and search tabs, live refresh, annotations, result export, settings and session recovery. Explicit editing is available for supported files up to 100 MiB. AI/MCP/memory modules and the old workspace sidebars are no longer present.

Use [implementation status](Implementation-Status.md) for boundaries, [architecture](Architecture.md) to navigate code, and [building](Building.md) for platform prerequisites. Release packages target Windows x64, macOS Apple Silicon and Linux x86_64; this is not a promise that every older operating system or graphics driver is supported.
