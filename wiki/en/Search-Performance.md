# Search performance and validation

[简体中文](../zh-CN/Search-Performance.md) · [Home / 首页](Home.md)

Core uses positional file reads, line indices, compressed source-row sets and parallel matching. Sparse directory results retain matching offsets; dense projections can share the full source index. Work scheduling is bounded and cancellation prevents new work from being acquired. Cache writing does not block result publication.

Repeated-query caching uses a process-local LRU bounded to 16 entries / 8 MiB of query and compressed-result data. Keys include source identity/content, encoding and query options. Reuse still validates the source; a cache hit is not a guarantee of zero disk I/O. Canceled or invalidated snapshots cannot become successful cached results.

Non-UTF-8 search uses bounded decoding batches; UTF-8 can borrow byte slices. Result limits preserve source order, with an extra match used to detect truncation. Parallel completion order must not choose which rows count as the first N. Ordinary UI searches publish complete results rather than partial progress previews.

Performance depends on file size, line count/length, encoding, storage, result density and parallelism. The fast-refresh middle-rewrite limitation is documented in [Live logs](Live-Logs.md).

Existing coverage lives in `crates/vclogg-core/tests/`, core document tests and workspace capability tests. Historical timings and test counts remain in the [archive](Archive.md). No application benchmark or test was rerun for this migration; do not quote old timings as current measurements.
