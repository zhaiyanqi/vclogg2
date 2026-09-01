//! Persistence and cache lifecycle adapters for VCLogg2.
//!
//! This crate owns durable and cached state. It deliberately has no dependency
//! on GPUI so storage work can run off the UI thread and be tested in isolation.

mod index_cache;
mod path_codec;

pub use index_cache::{
    IndexCacheCleanupResult, IndexCacheClearResult, IndexCacheInfo, cleanup_index_cache,
    clear_index_cache, index_cache_info,
};
pub use path_codec::{decode_persisted_path, encode_persisted_path};
