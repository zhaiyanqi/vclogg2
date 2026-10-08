//! Host-owned durable grants; command strings and resolved working directories match exactly.
use anyhow::Result;
use std::path::Path;

pub trait CommandApprovalStore: Send + Sync {
    fn is_allowed(&self, directory: &Path, command: &str) -> Result<bool>;
    fn allow(&self, directory: &Path, command: &str) -> Result<()>;
}
