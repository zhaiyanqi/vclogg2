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

    println!("cargo:rustc-env=VCLOGG2_BUILD_UNIX_TIMESTAMP={timestamp}");
    println!("cargo:rustc-env=VCLOGG2_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=VCLOGG2_BUILD_TARGET={target}");
    println!("cargo:rustc-env=VCLOGG2_BUILD_PROFILE={profile}");
}

fn git_commit() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let mut commit = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if commit.is_empty() {
        return None;
    }

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
