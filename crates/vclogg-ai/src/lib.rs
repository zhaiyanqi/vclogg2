//! Provider-independent, bounded log analysis. The host alone implements log access.
mod model;
mod provider;
mod runner;
mod skills;
mod tools;

pub use model::*;
pub use provider::stream_completion;
pub use runner::{RunHandle, start_run};
pub use skills::{Skill, import_skill, read_skill_file, refresh_skill};
pub use tools::{tool_definitions, validate_call};

pub const PAGE_LINES: usize = 100;
pub const RESULT_BYTES: usize = 64 * 1024;
pub const MAX_REQUESTS: usize = 20;
