# Cloud filters

[简体中文](../zh-CN/Cloud-Filters.md) · [Home / 首页](Home.md)

Cloud sharing is optional; local search and filters work without a server. Configure a server and username in Settings → Network, save/test, then open the cloud filter view. Connections use on-demand `/api/v1` requests, not a permanent socket.

## Authentication compatibility

The current desktop client registers without a built-in signing key. The Go server in `server/` defaults to `client_auth.enabled: true` and retains a signed-registration protocol. **Building both in one repository does not make those authentication modes compatible.** A default server deployment may reject this desktop client's registration. Align the protocol before production use; `client_auth.enabled: false` is only an isolated-development option, not a production recommendation.

Public connection settings go to SQLite. Cookie/CSRF data is stored in local private `cloud/sessions/<server-URL-SHA-256>.json` files without additional encryption; the current desktop client does not use the OS keychain. Do not publish these files. HTTPS is recommended; HTTP remains available for controlled local networks.

## UUIDs, revisions and merging

| Situation | Behavior |
| --- | --- |
| UUID absent locally | Download while preserving remote UUID |
| Only remote changed | Update to newer revision |
| Different fields changed on each side | Three-way merge |
| Same field changed differently | Explicit local/remote or field-by-field resolution |
| Resolve later | Retain local content and common baseline |
| Save as new filter | Create a new UUID branch |

Merging locally is not a remote submission. Publishing uses a common revision baseline. A concurrent-revision conflict fetches/merges the latest revision without automatically resubmitting. Restoring history creates local pending edits; deleting a remote share retains the local filter.

Sharing/submitting requires the server capability `filter-uuid-branches-v1`. Older servers can still support browsing and downloads. On failure, cached public pages can be searched/sorted locally; they are only pages previously viewed, not a complete offline mirror. Offline merge accepts only safely newer cached revisions. Likes, publishing, remote deletion and remote revision operations require connectivity.

See [filter acceptance](Filter-Branch-Acceptance.md) and [server source](https://github.com/zhaiyanqi/vclogg2/tree/main/server).
