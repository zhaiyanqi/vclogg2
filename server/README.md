# VCLogg Server

VCLogg Server 位于 VCLogg2 仓库的 `server/`，保留独立 Go 模块和管理端依赖。它是一个自包含的 Go HTTP 服务，使用单个 SQLite 数据库存储数据；管理页面、数据库迁移和静态资源都嵌入可执行文件，可在 Windows 与 Linux 之间迁移同一份数据库。

项目不会读取或编译桌面端源码。需要让正式桌面客户端和服务器使用同一套首次授权密钥时，两边只交换不含私钥的 `client-public-key.json`。

## 工程结构

```text
admin-web/             Vue 管理中心源码
cmd/vclogg-server/     命令行与服务入口
internal/              服务逻辑、配置、SQLite 存储和内嵌管理资源
scripts/               独立构建与校验脚本
config.example.yaml    完整配置示例
```

## 构建与发布

从仓库根目录运行统一打包入口：

```bash
./scripts/package-server-release.sh
# 可选：指定版本，默认与桌面端采用相同的标签/提交版本规则
./scripts/package-server-release.sh --version v2.2.14
```

Windows PowerShell：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-server-release.ps1
# 可选参数：-Version v2.2.14 -OutputDirectory D:\releases\server
```

需要 Go 1.24 或更高版本、Node.js 22.12 或更高版本、`pnpm@11.9.0`、tar，以及 macOS/Linux 上的 zip；命令应在 PATH 上。Windows 入口也能从 pnpm 启动器旁找到 Node.js。脚本安装锁定的 pnpm 依赖、检查管理端类型、重建内嵌管理资源，并以 `CGO_ENABLED=0` 构建 Windows/Linux amd64。

默认输出到仓库根 `dist/server/`：

- `vclogg-server-<version>-windows-amd64.zip`
- `vclogg-server-<version>-linux-amd64.tar.gz`
- `vclogg-server-<version>-SHA256SUMS.txt`

每个包只包含可执行文件、配置示例、README、LICENSE 和 VERSION；不包含运行数据、账户配置、密钥或依赖目录。Node.js、Vue 和 Element Plus 只参与构建，不是服务器运行时依赖。Linux 解压后先执行 `chmod +x vclogg-server`，以兼容 Windows 本机构建包的文件权限。

推送 `v*` 标签时，仓库的 `Multiplatform Release Build` 会将这些包与桌面包发布至同一个 GitHub Release。手动运行可选择 `all`、`desktop` 或 `server`，只上传 Actions Artifacts。详细依赖安装、命令及发布规则见 [Building](https://github.com/zhaiyanqi/vclogg2/wiki/Building) 和 [Build-and-Release](https://github.com/zhaiyanqi/vclogg2/wiki/Build-and-Release)。本地脚本只打包，不上传或部署。

原入口仍可使用：在 `server/` 中执行 `pnpm run build`，或运行 `server/scripts/build-server.bat`，都会转到统一发布打包逻辑。完整 Windows 校验入口为 `server/scripts/verify.ps1`。

仅开发管理中心时，在 `server/` 执行：

```bash
pnpm install --frozen-lockfile
pnpm run dev:admin
pnpm run build:admin
```

生产资源写入 `server/internal/app/admin/` 并由 Go 嵌入。修改管理端源码后，应重建并一起提交资源；发布工作流会检查两者一致。

## 可选客户端公钥

服务端保留原有签名鉴权协议。默认配置启用了 `client_auth`；使用默认配置启动前，必须提供受信任客户端的公钥。发布包不会自行生成或嵌入私钥，也不会关闭鉴权。

构建时可提供只含 `keyId` 与 `publicKey` 的公开 JSON 描述文件：

```bash
./scripts/package-server-release.sh --client-key-file /path/to/client-public-key.json
```

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-server-release.ps1 `
  -ClientKeyFile D:\path\to\client-public-key.json
```

也可同时设置 `VCLOGG_CLIENT_KEY_ID` 与 `VCLOGG_CLIENT_PUBLIC_KEY`；GitHub Actions 从同名 Repository Variables 读取。未配置时仍可打包，但部署前须在 `config.yaml` 的 `client_auth.public_keys` 配置公钥，或按实际客户端协议明确调整鉴权配置。现有 VCLogg2 客户端不会因同仓库构建而自动获得旧签名协议支持。

## 首次启动

```powershell
Copy-Item config.example.yaml config.yaml
./vclogg-server admin create -config ./config.yaml -username admin
./vclogg-server serve -config ./config.yaml
```

使用 `-ClientKeyFile` 构建的标准发行 server 已内置与桌面安装包匹配的公钥，无需修改 `public_keys`。自定义 server 构建若没有内置公钥，则必须手工配置；启用客户端鉴权但没有任何内置或自定义公钥时，服务器会拒绝启动。

管理页面默认地址为 `http://127.0.0.1:8787/admin/`。在不受信任的网络中，应使用 Caddy、nginx 或其他 TLS 反向代理。

## 桌面端自动更新

同一个服务端通过两套互不覆盖的目录和下载地址提供更新：

| 客户端 | 配置目录 | 默认文件目录 | 下载地址 |
| --- | --- | --- | --- |
| VCLogg | `updates.directory` | `./data/updates/win-x64` | `/updates/win-x64/` |
| VCLogg2 | `updates.vclogg2_directory` | `./data/updates-vclogg2/win-x64` | `/updates-vclogg2/win-x64/` |

两个更新根目录默认位于同一级。VCLogg 继续使用 `latest.yml`、NSIS 安装包和 `.blockmap`，现有客户端和发布流程无需修改。桌面项目仍提供原子发布脚本，例如：

```powershell
powershell -ExecutionPolicy Bypass -File D:\path\to\vclogg\scripts\publish-update.ps1 `
  -TargetDirectory D:\vclogg-server\data\updates\win-x64
```

发布脚本先复制带版本号的安装包和 blockmap，最后替换 `latest.yml`，并保留历史 blockmap 供差分更新使用。不要在发布过程中先覆盖 `latest.yml`。更新文件通过公开只读的 `/updates/win-x64/` 路由下载，不使用客户端 Cookie；生产网络应使用 HTTPS 并为安装包配置 Windows 代码签名。

也可以直接登录管理页面，在“版本与备份”中分别上传两个客户端的发行 ZIP，不再登录服务器复制文件：

- “VCLogg 软件发布”的 ZIP 根目录必须且只能包含 `latest.yml`、清单中对应的 `VCLogg-<version>-Setup-x64.exe` 和同名 `.blockmap`。
- “VCLogg2 软件发布”的 ZIP 根目录必须且只能包含 `latest.json`、`vclogg2-<version>-win-x64.zip` 和 `vclogg2-<version>-win-x64.blockmap.json`。

服务端会分别校验清单、SemVer、文件名、大小、完整文件摘要和分块摘要，在对应目录内完成暂存；版本文件和 blockmap 就绪后才替换各自的最新清单。两种产品会独立保留历史文件，发布 VCLogg2 不会修改 VCLogg 的版本标记或更新文件。同一版本再次上传相同文件是幂等的，不同内容则拒绝覆盖，避免客户端的长期缓存命中错误文件。

例如桌面项目的发行目录已有三个发行文件时，可以先将它们压缩为一个 ZIP 再通过管理页面上传：

```powershell
$version = '0.8.9'
$releaseDirectory = 'D:\path\to\vclogg\release'
$installer = Join-Path $releaseDirectory "VCLogg-$version-Setup-x64.exe"
Compress-Archive -LiteralPath (Join-Path $releaseDirectory 'latest.yml'),$installer,"$installer.blockmap" -DestinationPath (Join-Path $releaseDirectory "$version.zip") -Force
```

反向代理还需要允许至少 513 MiB 的请求体（其中发行 ZIP 最大 512 MiB，另有 multipart 边界开销），并为上传接口配置足够长的请求超时。发布接口继续使用管理员会话、CSRF 和 `settings.version.update` 权限，成功操作会写入审计日志。

## 管理中心与 RBAC

管理中心使用内嵌的 Vue 3、Element Plus 和 Pinia，提供仪表盘、关键词、访问身份、管理员、角色权限、版本备份、操作审计和账户安全页面。页面资源与 Go 服务打入同一个可执行文件，`/admin/roles` 等 History 路由可直接打开和刷新。

管理员继续使用 12 小时的 `HttpOnly`、`SameSite=Strict` Cookie；所有修改请求必须携带会话返回的 `X-CSRF-Token`。权限由 SQLite 中的角色、成员和权限关联表管理，并在每个服务端 API 上强制校验。角色变化立即生效，前端隐藏菜单或按钮不构成安全边界。

内置 `super_admin` 角色拥有全部权限且不可修改或删除。升级旧数据库时，已有管理员会自动加入该角色；系统禁止停用或降级最后一个有效超级管理员。命令行创建的管理员默认也是超级管理员，其他管理员应在管理中心创建并分配最小必要权限。

管理操作、登录结果和权限拒绝写入审计日志。日志包含动作、资源、结果、可信来源地址和脱敏元数据，不保存密码、Cookie、CSRF 或访问令牌。

关键词管理把查看、编辑、状态变更和导出拆分为独立权限。管理员编辑必须携带列表中的基础修订，过期修订返回 `409 revision_conflict`。过滤器以 UUID 区分身份，同名同内容但 UUID 不同的记录可以并存；管理列表显示短 UUID，编辑页显示完整 UUID。具有 `filters.export` 权限的管理员可按当前搜索和状态筛选导出 JSON 或带 UTF-8 BOM 的 CSV，导出操作会记录格式、筛选条件和条目数量审计。

仅当服务器只能通过可信反向代理访问时，才可设置 `trusted_proxy: true`。启用后，服务器会接受 `X-Forwarded-Proto: https`，并为管理与客户端 Cookie 设置 `Secure`。

## 客户端授权与 Cookie

`POST /api/v1/identities/register` 必须携带 Ed25519 签名。签名覆盖请求方法、完整路径与查询、时间戳、随机数和请求体哈希；服务器会拒绝未知密钥、过期时间、内容篡改和随机数重放。

授权成功后，服务器发放 `vclogg_client_session` Cookie 和 CSRF 令牌：

注册响应和 `GET /api/v1/me` 同时发布 `filter-uuid-branches-v1` capability。支持该能力的客户端把本地 UUID 放入现有 `clientFilterId`：UUID 不存在时以该 UUID 创建修订 1，同一所有者的同 UUID 后续按 `baseRevision` 更新，其他所有者占用该 UUID 时返回 `409 filter_identity_conflict`。非 UUID `clientFilterId` 继续走旧客户端兼容流程，由服务端生成远程 UUID。已删除的 UUID 可由原分享者再次发布：删除会解除客户端基线，因此恢复时允许省略 `baseRevision`；若显式提供版本仍须匹配。恢复保留历史修订，内容变化时追加修订，内容未变时只恢复可见性。其他所有者仍不能接管 UUID，管理员禁用的记录不能通过再次分享恢复。

- Cookie 为 `HttpOnly`、`SameSite=Strict`、`Path=/api/v1`，默认绝对有效期为 720 小时。
- 后续身份 API 只接受 Cookie，不接受 Bearer 令牌。
- 所有非 GET/HEAD 请求必须同时携带 `X-VCLogg-CSRF`。
- Cookie 过期后，Rust 客户端使用保存在系统凭据库中的用户令牌重新签名授权，并只重试原请求一次。
- 每个身份默认最多保留 5 个会话；超出后自动淘汰最旧会话。
- `POST /api/v1/session/logout` 会撤销当前会话并清除 Cookie。

用户令牌、Cookie 和 CSRF 令牌不会返回给渲染界面。Rust 客户端将它们保存到 Windows Credential Manager、macOS Keychain 或 Linux Secret Service；凭据库不可用时不会降级为明文文件。

客户端按规范化服务器地址隔离凭据。切换服务器只改变当前活动连接，不注销其他已保存服务器；切回时优先复用原 Cookie，过期后自动重新授权。只有“移除服务器”或“断开当前服务器”才会注销并删除该服务器的本地凭据。

## 过滤器共创与版本接口

云端过滤器默认只有上传者可编辑和删除。上传者开启 `collaborative` 后，同一服务器上的其他有效身份可以编辑名称、匹配值、正则状态和备注，但不能切换共创或删除过滤器；管理员仍可通过管理接口审核和修改。

- `GET /api/v1/filters/{id}`：读取当前内容、共创状态以及当前身份的 `canEdit/canDelete` 权限。
- `PATCH /api/v1/filters/{id}`：编辑当前版本；新版客户端携带 `baseRevision`，版本不一致时返回 `409 revision_conflict`。
- `DELETE /api/v1/filters/{id}`：仅上传者可删除。
- `GET /api/v1/filters/{id}/revisions`：分页读取编辑记录，包含编辑者、角色、时间、共创状态和当前版本标记。
- `GET /api/v1/filters/{id}/revisions/{revision}`：读取指定版本的完整只读快照。

每次实际内容变化或共创状态变化都会写入新的 `filter_revisions` 记录；名称、匹配值、正则状态、备注和共创状态全部未变化时，编辑及再次分享接口返回 `409 no_changes`，不会产生版本或成功假象。历史快照不随普通编辑清理，并随 SQLite 备份保留。为兼容旧客户端，缺失的首次分享 `collaborative` 按关闭处理，缺失 `baseRevision` 的旧编辑请求仍可执行。

## 配置

`config.example.yaml` 包含全部配置项：

- `listen`：HTTP 监听地址。
- `data_dir`：SQLite 数据与备份目录。
- `trusted_proxy`：是否信任反向代理的 HTTPS 标记。
- `updates.enabled`：是否提供桌面端更新文件。
- `updates.directory`：更新文件根目录；其下必须包含 `win-x64/latest.yml`、安装包和 blockmap。
- `updates.vclogg2_directory`：VCLogg2 更新文件根目录；其下包含 `win-x64/latest.json`、便携更新包和 JSON blockmap。未配置时会在 VCLogg 更新目录同级派生一个 `-vclogg2` 目录。
- `client_auth.enabled`：是否要求客户端签名。
- `client_auth.accept_bundled_public_keys`：是否接受标准发行 server 内置的客户端公钥，默认启用；严格环境可关闭。
- `client_auth.max_clock_skew_seconds`：签名时间允许的最大偏差。
- `client_auth.nonce_ttl_seconds`：防重放随机数保留时间。
- `client_auth.session_ttl_hours`：客户端 Cookie 的绝对有效期。
- `client_auth.max_sessions_per_identity`：每个身份允许的有效会话数量。
- `client_auth.public_keys`：允许访问的客户端 Ed25519 公钥列表。
- `uploads`：每个身份的上传频率、过滤器数量、存储字节数、批量数量和请求体大小限制。

`client_auth.enabled: false` 仅用于隔离的本地开发环境。

## 备份与恢复

数据库使用 WAL 模式，运行时不要直接复制数据库文件。生成一致性备份：

```powershell
./vclogg-server backup -config ./config.yaml
```

恢复前停止目标服务器，然后执行：

```powershell
./vclogg-server restore -config ./config.yaml -input ./vclogg-backup.db
```

访问令牌哈希、管理员、客户端会话、过滤器、点赞、下载计数、上传频率记录和防重放随机数都包含在 SQLite 备份中。

## 密钥轮换

长期发行密钥不随普通版本更新。需要轮换时，先把新公钥加入现有服务器的 `client_auth.public_keys` 并重启服务，再发布使用新私钥构建的客户端，最后删除旧公钥。整个过程不需要重新编译 server 代码。

通用发行私钥会存在于原生客户端二进制中，具备逆向能力的攻击者可能提取它，并影响所有仍接受对应内置公钥的服务器。高安全环境应设置 `accept_bundled_public_keys: false`，并使用独立的自定义发行密钥。
