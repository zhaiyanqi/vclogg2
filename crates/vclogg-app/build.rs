use std::{
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const WINDOWS_MAIN_STACK_BYTES: usize = 8 * 1024 * 1024;

fn main() {
    emit_build_metadata();

    if cfg!(target_os = "windows") {
        println!("cargo:rustc-link-arg-bin=vclogg2=/STACK:{WINDOWS_MAIN_STACK_BYTES}");
        embed_windows_resources();
    }
}

fn emit_build_metadata() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
    for path in ["../../.git/HEAD", "../../.git/index", "../../.git/refs"] {
        if Path::new(path).exists() {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-env-changed=VCLOGG2_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=VCLOGG2_BUILD_VERSION");

    let timestamp = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_secs())
        })
        .unwrap_or_default();
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    let version = build_version();
    let commit = std::env::var("VCLOGG2_BUILD_COMMIT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(git_commit)
        .unwrap_or_else(|| "unknown".to_string());
    let profile = match std::env::var("PROFILE").as_deref() {
        Ok("debug") => "Debug".to_string(),
        Ok("release") => "Release".to_string(),
        Ok(profile) => profile.to_string(),
        Err(_) => "Unknown".to_string(),
    };

    println!("cargo:rustc-env=VCLOGG2_VERSION={version}");
    println!("cargo:rustc-env=VCLOGG2_BUILD_UNIX_TIMESTAMP={timestamp}");
    println!("cargo:rustc-env=VCLOGG2_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=VCLOGG2_BUILD_TARGET={target}");
    println!("cargo:rustc-env=VCLOGG2_BUILD_PROFILE={profile}");
}

fn build_version() -> String {
    if let Some(version) = std::env::var("VCLOGG2_BUILD_VERSION")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return normalize_version(&version).unwrap_or_else(|error| {
            panic!("VCLOGG2_BUILD_VERSION must be a semantic version: {error}")
        });
    }

    if let Some(tag) = git_output(&[
        "describe",
        "--tags",
        "--exact-match",
        "--match",
        "v[0-9]*",
        "HEAD",
    ]) {
        return normalize_version(&tag)
            .unwrap_or_else(|error| panic!("release tag must be v<semver>: {error}"));
    }

    git_output(&["rev-parse", "--short=12", "HEAD"])
        .map(|commit| format!("0.0.0-dev+g{commit}"))
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

fn normalize_version(value: &str) -> Result<String, semver::Error> {
    let value = value.trim().strip_prefix('v').unwrap_or(value.trim());
    semver::Version::parse(value).map(|version| version.to_string())
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn git_commit() -> Option<String> {
    let mut commit = git_output(&["rev-parse", "--short=12", "HEAD"])?;

    let dirty = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .output()
        .ok()
        .is_some_and(|output| output.status.success() && !output.stdout.is_empty());
    if dirty {
        commit.push_str("-dirty");
    }

    Some(commit)
}

#[cfg(target_os = "windows")]
fn embed_windows_resources() {
    const RESOURCE_SCRIPT: &str = "resources/windows/vclogg2.rc";

    println!("cargo:rerun-if-changed={RESOURCE_SCRIPT}");
    println!("cargo:rerun-if-changed=resources/windows/vclogg2.ico");
    embed_resource::compile(RESOURCE_SCRIPT, embed_resource::NONE)
        .manifest_required()
        .expect("compile VCLogg2 Windows resources");
}

#[cfg(not(target_os = "windows"))]
fn embed_windows_resources() {}
