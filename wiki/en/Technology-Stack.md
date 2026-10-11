# Technology stack

[简体中文](../zh-CN/Technology-Stack.md) · [Home / 首页](Home.md)

| Technology | Role |
| --- | --- |
| Rust stable / Edition 2024 | Desktop workspace and native executable |
| GPUI Kit **0.7.1** | Unified GPUI, component, base and asset APIs |
| Rayon, Aho-Corasick, regex, memchr | Parallel search, literal/regex matching, byte scanning |
| chardetng, encoding_rs | Detection and legacy decoding |
| roaring / RoaringTreemap | Compressed source-row sets |
| rusqlite with bundled SQLite | WAL, transactions, settings and schema migration |
| notify 7 | Active-document filesystem notifications |
| reqwest / Rustls / url | Optional cloud HTTP client |
| serde / JSON / UUID / SHA-256 | Stable identities, interchange and validation |
| windows-sys / objc2 / x11rb | Native platform integration |
| Go + SQLite, Vue 3 / Element Plus / Pinia | Independent optional server and embedded admin UI |
| mdBook + GitHub Actions Pages | Documentation generation and hosting |

`rust-toolchain.toml` selects stable with rustfmt/Clippy. `Cargo.lock` pins resolved dependencies; do not remove it for ordinary builds. The app directory is `crates/vclogg-app`, but the package selector is `-p vclogg2`.

Existing compatibility entries in root Cargo.toml are `vendor/block` (block 0.1.6 compatibility) and `compat/xim-ctext` (legacy 0.3.0 interface re-exporting the official 0.4.1 decoder). There is no current vendored GPUI component/base patch. The Go module and pnpm lockfile are independent of Cargo.

See [architecture](Architecture.md) and [building](Building.md).
