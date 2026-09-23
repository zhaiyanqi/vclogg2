# X11 Compound Text compatibility bridge

This is VCLogg2-owned adapter code, not a copy or fork of an external library.
The implementation only re-exports the official `xim-ctext` **0.4.1** crate.
It retains the **0.3.0** package identity required by `zed-xim` 0.4.0-zed and
forwards the `std` feature. The workspace Cargo patch routes that legacy
requirement through this bridge; Cargo.lock pins the registry source/checksum
of the actual 0.4.1 implementation. `zed-xim` itself is unchanged from crates.io.

## Why this exists

The GPUI Linux dependency currently uses `zed-xim` 0.4.0-zed, which depends on
`xim-ctext ^0.3.0`. That decoder rejects GB2312, and the XIM commit handler
panics on its decoding error. This caused the reported Ubuntu/X11/Fcitx/Sogou
crash when confirming Chinese input.

The official decoder 0.4.1 supports GB2312 and mixed Chinese/ASCII commits,
including Sogou date phrases. A normal Cargo update cannot cross the 0.3/0.4
compatibility boundary, so the bridge supplies the existing API without
maintaining a local decoder.

- [Official release](https://crates.io/crates/xim-ctext/0.4.1)
- [Upstream source](https://github.com/Riey/xim-rs/tree/master/xim-ctext)

## Verification and limits

`cargo test -p xim-ctext@0.3.0 --locked` covers GB2312, mixed Chinese/ASCII,
Sogou-style date phrases, long commits, UTF-8 and Japanese text. The original
`zed-xim` is a development dependency so these checks also compile it against
the bridge. These tests are included in `cargo test --workspace` and CI.

The bridge fixes the supported Chinese encoding path. It does not alter the
upstream XIM handler's `expect`/`unwrap` behavior on other decoding errors, nor
does it promise support for every Compound Text encoding. Real Sogou input
still requires Ubuntu/X11 acceptance testing.

Remove this package and its Cargo patch once the GPUI dependency chain uses
the corrected decoder directly, retaining the relevant input regressions.
