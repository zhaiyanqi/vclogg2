# MessageScroller compatibility port

Source: `crates/component/src/message_scroller.rs` from
[GPUI Kit](https://github.com/longbridge/gpui-kit), snapshot
`c33bfebf03f5f7a1751d0395b40c786b869a9f50`, Apache-2.0.

This source port makes the upstream MessageScroller and MessageScrollerState
available to VCLogg's pinned GPUI/component versions. It does not depend on a
machine-local checkout or introduce the incompatible gpui-pre runtime.

Changes from upstream: imports target the existing gpui-component crate;
the newer generic ScrollableMask is unavailable, so this standalone transcript
uses GPUI List's native wheel handling (do not nest it inside another scroller);
optional bottom-fade and jump-button transitions are omitted. Jump visibility
changes immediately, independent of OS motion settings. The virtual list,
row remeasurement, tail following, styling slots and
jump-to-latest action retain the upstream implementation.

The application owns its messages, Markdown entities and composer. When the
main component dependency supplies MessageScroller, replace this compatibility
crate with that module.
