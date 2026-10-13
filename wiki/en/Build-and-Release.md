# Build and release

[简体中文](../zh-CN/Build-and-Release.md) · [Home / 首页](Home.md)

Build prerequisites are in [Building](Building.md). All desktop packages use the same Cargo.lock and native platform runners. Package scripts currently bundle ripgrep and its license even though AI features have been removed.

| Platform | Root-level command | Output |
| --- | --- | --- |
| Windows x64 | `powershell -ExecutionPolicy Bypass -File scripts/package-release.ps1` | `dist/windows-x86_64/`, portable ZIP and Inno Setup EXE |
| macOS ARM64 | `./scripts/package-release-macos.sh` | `dist/macos-aarch64/`, DMG |
| Linux x86_64 | `./scripts/package-release-linux.sh` | `dist/linux-x86_64/`, tar.gz |
| Server Windows/Linux amd64 | `./scripts/package-server-release.sh` or PowerShell equivalent | `dist/server/`, ZIP, tar.gz and SHA256SUMS |

Windows user packages are `vclogg-<version>-windows-x86_64-portable.zip` and `vclogg-<version>-windows-x86_64-setup.exe`; macOS/Linux use the `vclogg2-` prefix. The optional Windows PDB symbols package is for developer diagnosis, not user installation; actual symbol availability depends on the build profile and packaging output.

## Installation and signing

Windows per-user installation separates program and data locations, writes `vclogg2-data-dir.txt`, and preserves data on uninstall. Silent installation can specify `/DIR="D:\Apps\VCLogg2" /DATADIR="D:\VCLogg2Data"`. Moving the data location does not copy existing data. `-InnoSetupCompiler` selects a nonstandard ISCC path.

Windows signing modes: `None` produces unsigned packages; `Pfx` signs the program then installer using configured certificate/password/timestamp inputs; `PreSigned -SkipBuild` requires an already signed application and the newly generated installer still needs signing. Actions selects unsigned, PFX or Microsoft Artifact Signing via `WINDOWS_SIGNING_PROVIDER`. Signed branches verify the application extracted from the ZIP and the installer Authenticode chain and RFC 3161 timestamp. Package names do not encode signing status; the generated uninstaller is not separately signed.

macOS Actions packages are ad-hoc signed without Developer ID/notarization. Linux packages include a per-user installer, desktop integration and MIME registration. See [Installation](Installation.md) for upgrade/data behavior.

## Version and publishing

The Cargo package version remains a placeholder. Release builds resolve `v<SemVer>` tags or `VCLOGG2_BUILD_VERSION`; untagged builds follow the repository's development-version rules.

For an intentional release, commit/push a clean main first, create a tag at that commit, then use the release helper (replace the example version):

```sh
git tag -a v2.0.8 -m 'VCLogg2 v2.0.8'
./scripts/publish-github-release.sh v2.0.8
```

The helper requires clean main equal to origin/main, checks the tag, runs `scripts/check.sh`, and pushes the tag. `--yes` skips its interactive confirmation; `--remote` selects another remote. Pushing `v*` triggers the multiplatform workflow and creates a Release with desktop and server assets. Manual workflow dispatch selects all/desktop/server and uploads Actions artifacts without creating a Release. Ordinary main pushes do not run the manual-only application CI; documentation paths trigger Pages.

Release notes derive English/Chinese sections from complete commit messages, with historical translations in `scripts/commit-translations.json`. Categories, ordering, commit links and a comparison link are preserved across languages. See [Contributing](Contributing.md).

## Update checks

**Help → Check for updates** reads `https://zhaiyanqi.github.io/vclogg2/latest.json` and compares stable releases using SemVer. Only a source version with strictly higher SemVer precedence opens release notes with current/latest versions and a download action; build metadata does not affect precedence. Equal or older versions show “No updates available” with current/source versions and no release notes or download action. Cached results use the same comparison; network, HTTP or manifest-validation failure opens the Releases index; failures retain the existing browser fallback. Successful checks are cached for 24 hours. Failures back off for 1, 2, 4, 8, 16, 32, then 60 minutes, capped at 60 minutes thereafter; clicks during backoff open Releases without another website request. Cache and backoff state persist in local SQLite across restarts, with an in-process fallback if storage is unavailable. Windows share an in-flight check. Connection timeout is 5 seconds and total request timeout is 10 seconds, with no immediate retry. Checks only run on user request, never at startup, and do not download or install packages.

Pages generates `_site/latest.json` using the workflow token to read the latest published stable GitHub Release. It validates the stable tag, publication time, official URL and all four desktop packages; main’s placeholder version and prereleases are excluded. Generation failure prevents deployment and preserves the previous website. Release publication, edits and deletion trigger deployment, as does a successful tag release build via `workflow_run` (covering releases created by `GITHUB_TOKEN`). Only deployment calls the Releases API; clients need no token.

## Server packaging

Use `--version v2.2.14` / `-Version v2.2.14` to override version and `--client-key-file /path/to/client-public-key.json` / `-ClientKeyFile` for an optional **public** key descriptor. `VCLOGG_CLIENT_KEY_ID` and `VCLOGG_CLIENT_PUBLIC_KEY` are equivalent paired inputs. Packages contain executable, example config, README, LICENSE and VERSION, not runtime databases or private keys. The admin UI and migrations are embedded; Node.js is a build-time dependency.

Server support for update feeds does not mean this desktop client uses them. Desktop checks use the static website manifest described above. The server's signed-registration default also needs reconciliation with the current unsigned desktop registration; see [Cloud filters](Cloud-Filters.md).

Source: [release workflow](https://github.com/zhaiyanqi/vclogg2/blob/main/.github/workflows/release-build.yml), [scripts](https://github.com/zhaiyanqi/vclogg2/tree/main/scripts).

Clicking Check for updates immediately shows a pending dialog regardless of notification settings; completion replaces its content with release notes or “No updates available”. If dismissed while pending, background work only completes caching and does not show results or open the browser.

## Cross-version release history

Deployment paginates through GitHub releases, excludes drafts, prereleases and non-stable SemVer tags, and retains the highest 30 stable versions in descending SemVer order. The latest release still requires all four desktop packages. The manifest preserves legacy top-level fields and adds `releases` and `history_complete`. Clients validate ordering, duplicate versions, URLs and the 30-entry limit, with an 8 MiB response limit. Oversized or invalid manifests stop deployment rather than truncating notes.

One client request supplies all available releases newer than the installed version through the latest version. The newest three are expanded initially; every version can be expanded or collapsed independently. Up-to-date or newer installed builds show only the latest release. Versions older than the retained history get an explicit partial-history notice and a View full release history action; legacy single-version feeds also receive conservative coverage notices.

The manifest retains extracted breaking-change and migration fields for older clients. Authors must use Breaking Changes / 不兼容变更 and Migration / 迁移 headings. Generation preserves these sections and conservatively collects complete paragraphs or list blocks containing related keywords, without rewriting or summarizing them. Requirements absent from the published notes cannot be inferred; release authors must record them explicitly.

## Release-note language

The manifest stores per-release body and important notes in `localized_notes`, keyed by `zh-CN` and `en`. Generation splits the Release’s `## 简体中文` / `## English` sections without automatic translation and retains the mixed-language fields for older clients. The current application language selects the release body. Both languages share one request and cache, so switching languages needs no new network request. Legacy feeds and caches are parsed by language headings; missing or unrecognized translations produce an explicit notice rather than mixing in another language. Missing translations show a notice within that version directing users to its original release page.

When a newer version is available, the update result shows current/latest versions followed by per-version release notes, without an extra Version changes heading or a separate important-change summary. Breaking changes and migration details remain in the original per-version body. Important-note fields remain in the manifest for older clients.
