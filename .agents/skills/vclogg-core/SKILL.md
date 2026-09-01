---
name: vclogg-core
description: Navigate and modify VCLogg2 core file reading, indexing, search execution, compressed result sets, and cancellation. Use for algorithmic work in crates/vclogg-core; do not use for SQLite persistence or GPUI presentation behavior.
---

# VCLogg2 Core

Keep core deterministic, GPUI-free, persistence-free, and usable from background workers. Read [doc/architecture.md](../../../doc/architecture.md) before changing a public boundary.

## Route the change

- `document.rs`: verified source snapshots, bounded previews, decoding, line readers, line-index construction, cached index format, append refresh.
- `search.rs`: query compilation, parallel/cancellable scans, progress and completed search results.
- `result_set.rs`: `CompressedRows`, union/subtraction, stable source-row lookup, positional ranges, portable compressed encoding.
- `cancellation.rs`: cooperative cancellation shared by long-running core operations.

File bytes and index construction belong here. Cache retention, cleanup policy, SQLite, and recovery records belong in `vclogg-data`. Selection, highlighting, line height, wrapping, marks, focus and rendering belong in `vclogg-app`.

## Preserve these invariants

- Poll cancellation before the next expensive source read or scan batch; cancellation prevents stale I/O while the caller's identity/revision check rejects late installation.
- Search and result composition operate in stable source-row coordinates and keep large sets compressed.
- Rendering callers receive snapshots/readers; core never reaches into GPUI entities or performs UI notification.
- A source changed during a verified operation is an explicit outcome, never a partial successful result.

## Verify

Run the narrow test for the changed module, then `cargo test -p vclogg-core`. For search concurrency also run `cargo test -p vclogg-core --test search_parallel`. Before a complete commit run the workspace checks documented in the root workflow.
