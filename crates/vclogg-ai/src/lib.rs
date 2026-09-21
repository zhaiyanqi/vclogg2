//! Provider-independent, bounded log analysis. The host alone implements log access.
mod builtin_skills;
mod compaction;
mod context_usage;
mod defaults;
mod discovery;
mod mcp;
mod model;
mod model_limits;
mod prompts;
mod provider;
mod ripgrep;
mod runner;
mod skills;
mod source_index;
mod source_lsp;
mod source_workspace;
mod tools;

pub use compaction::{compact_conversation, conversation_tokens};
pub use discovery::*;
pub use mcp::{McpConnection, McpServer, probe_mcp};
pub use model::*;
pub use model_limits::{ModelLimits, discover_model_limits, parse_model_limits};
pub use prompts::*;
pub use provider::stream_completion;
pub use runner::{
    RunExtensions, RunHandle, log_reference_metadata, start_run, start_run_with_extensions,
};
pub use skills::{Skill, import_skill, read_skill_file, refresh_skill};
pub use tools::{tool_definitions, validate_call};

pub const PAGE_LINES: usize = 100;
pub const RESULT_BYTES: usize = 64 * 1024;
pub const MAX_REQUESTS: usize = 20;
