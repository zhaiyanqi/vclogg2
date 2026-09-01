/// Application version resolved by `build.rs` from the current Git tag.
pub const VERSION: &str = env!("VCLOGG2_VERSION");

/// Application version with the conventional release-tag prefix.
pub const DISPLAY_VERSION: &str = concat!("v", env!("VCLOGG2_VERSION"));

/// HTTP user agent containing the same version shown by the application.
pub const USER_AGENT: &str = concat!("VCLogg2/", env!("VCLOGG2_VERSION"));
