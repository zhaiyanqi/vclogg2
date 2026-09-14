# VCLogg2 TextView selection patch

Source: https://github.com/longbridge/gpui-component
Revision: `38b2f652874fec3c31e5a876443ee594c0fd9d04`, package `gpui-component` 0.5.2.

Source, locales, build script, tests and Apache license are retained from `crates/ui`.
The standalone manifest expands upstream workspace dependencies and lints without
upgrading their versions or features. Two upstream test fixture paths are adapted
to this vendor layout, and the Aurora theme fixture is retained. Cargo selects this copy together with the
existing `gpui-base` patch; no framework migration is required.

`TextViewStyle::selection_colors(background, foreground)` optionally overrides
selection colors for one TextView. The default behavior is unchanged. All inline
content, including code, uses the same renderer. It repaints the original shaped
glyphs inside the selected rectangles, retaining wrapping, font fallback and
ligatures. Color emoji retain their native rendering. Copying and selection state
are unchanged. Application colors stay in the product theme.

The source changes are recorded in `selection-colors.patch` (apply with
`git apply --unidiff-zero` inside the upstream `crates/ui` directory). On dependency
upgrades, retain this capability until upstream exposes equivalent selection
foreground/background styling. Remove this patch when that API is available.
