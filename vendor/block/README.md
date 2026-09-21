# Patched `block` 0.1.6

This directory vendors [`SSheldon/rust-block`](https://github.com/SSheldon/rust-block)
version 0.1.6 under its MIT license. GPUI Kit's macOS dependency chain still
requires this legacy API.

Local compatibility changes:

- represent the opaque Objective-C `Class` symbol with an inhabited zero-sized
  C-layout struct instead of an uninhabited enum;
- spell the existing C ABI explicitly on foreign declarations and callbacks.

These changes preserve the crate's public API and runtime layout while removing
Rust future-incompatibility and missing-ABI warnings. Remove the workspace patch
when GPUI Kit no longer depends on `block` 0.1.x.
