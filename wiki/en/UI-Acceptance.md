# UI acceptance checklist

[简体中文](../zh-CN/UI-Acceptance.md) · [Home / 首页](Home.md)

These are manual review scenarios, **not results executed for this documentation migration**.

- Open empty, small, very large and long-line logs; check visible-row reading, wrapping, font changes and source-line navigation.
- Exercise current/global/directory search tabs, queued/canceled searches, reordering, limits and result-source navigation.
- Copy text/rows, multi-select, mark, color and add/edit/drag text tags; verify no decoration leaks into export.
- Append, truncate and rotate a foreground file; switch tabs/windows during refresh; verify follow and F5 recovery.
- Preview settings across windows, cancel, save and reopen; check singleton window, size recovery and category focus.
- Edit a supported small file, reject a changed source, cancel a dirty draft and save a new file without overwriting another.
- Test platform shortcuts, input focus, window controls, high-DPI layouts and both UI languages.
- Verify tab copies/moves, session recovery, protected history cleanup and temporary-result ownership.

Record source commit, platform, scenario and actual outcome separately. Historical acceptance boards and results remain in [Archive](Archive.md).
