//! File indexing and search services used by the VCLogg2 desktop application.

mod cancellation;
mod document;
mod search;

pub use cancellation::CancellationToken;
pub use document::{
    DocumentMetadata, DocumentRefreshKind, LinePreview, LinePreviewReader, LineReader, LogDocument,
    PendingIndexCacheWrite,
};
pub use search::{
    CompressedRows, SearchCancellation, SearchMatcher, SearchProgress, SearchProgressSnapshot,
    SearchQuery, SearchResult, SearchRun, search, search_cancellable, search_with_compiled_matcher,
    search_with_progress,
};

/// Core crate version exposed for delivery diagnostics.
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");
