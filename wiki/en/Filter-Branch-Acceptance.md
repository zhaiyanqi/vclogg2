# Filter branch acceptance checklist

[简体中文](../zh-CN/Filter-Branch-Acceptance.md) · [Home / 首页](Home.md)

Review against the intended desktop/server pair; these are scenarios, not a newly completed test report.

1. Check unsigned desktop registration against the chosen server authentication policy and `filter-uuid-branches-v1` capability.
2. Download a new UUID and retain identity. Import identical content with a different UUID and retain both entries.
3. Test remote-only updates, disjoint-field merges and conflicting edits to the same field; “resolve later” must preserve the baseline.
4. Save as a new filter to create an independent UUID; local merging must not silently publish.
5. Submit with a stale revision: fetch/merge without automatic retry publishing. Restore history as local pending changes.
6. Delete a remote share while retaining the local filter; check owner/collaborator permissions.
7. Disconnect and browse cached pages; prevent remote mutations and unsafe revision rollback.
8. Restart and switch servers; verify server-isolated sessions and no credentials in SQLite/public caches or exports.

See [Cloud filters](Cloud-Filters.md) and [Archive](Archive.md).
