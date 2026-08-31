//! File indexing and search services used by the VCLogg2 desktop application.

mod document;
mod index_cache;
mod search;

pub use document::{
    DocumentMetadata, DocumentRefreshKind, LinePreview, LogDocument, PendingIndexCacheWrite,
};
pub use index_cache::{
    IndexCacheCleanupResult, IndexCacheClearResult, IndexCacheInfo, cleanup_index_cache,
    clear_index_cache, index_cache_info,
};
pub use search::{
    CompressedRows, SearchCancellation, SearchMatcher, SearchProgress, SearchProgressSnapshot,
    SearchQuery, SearchResult, SearchRun, search, search_cancellable, search_with_compiled_matcher,
    search_with_progress,
};

/// Core crate version exposed for delivery diagnostics.
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");
