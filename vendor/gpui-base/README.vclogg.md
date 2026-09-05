# VCLogg2 gpui-base patch

Source: https://github.com/longbridge/gpui-component

Revision: `38b2f652874fec3c31e5a876443ee594c0fd9d04`, package `gpui-base` 0.5.2.
The `src` directory and Apache license come from `crates/base` at that revision.
Existing upstream tests and fixtures are retained; no new tests are added.

The workspace Cargo patch selects this copy for both VCLogg2 and the upstream
`gpui-component` dependency, so they share one set of base types. The manifest
expands upstream workspace dependencies and lint settings without changing their
versions or features. Examples are omitted, the example default-run is removed,
and an empty workspace keeps the dependency outside the application workspace.

## Single-line vertical scrolling

Single-line fields must always have a zero vertical scroll offset. Fractional
text line heights and device-pixel-snapped viewports can otherwise cause cursor
reveal to publish a small negative offset. The upstream post-paint reset to zero
then creates a one-frame upward jump in text and placeholders.

Local changes, captured in `single-line-scroll.patch`:

- `src/input/base/state.rs`: normalize the deferred cursor-reveal target to zero
  on the vertical axis for single-line inputs, before it reaches a frame.
- `src/input/base/element.rs`: skip vertical cursor-follow for single-line inputs
  and normalize retained/deferred offsets before applying them to paint geometry.

Horizontal scrolling and the existing multiline/editor paths are preserved.
This applies to every single-line input on every platform; it does not change
search toolbar dimensions, styling, focus, or keyboard dispatch.

When upgrading the dependency, compare these two paths with the new upstream
revision. Remove this vendor directory and the Cargo patch once upstream enforces
the invariant both when producing the target and before painting.
