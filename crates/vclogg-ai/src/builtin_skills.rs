//! Discoverable product workflows, including one shell guide for the current platform.
use crate::{AiSettings, SkillDirectory};
use anyhow::{Context as _, Result};
use std::{fs, path::Path};
mod legacy;
mod workflows;

struct BuiltinSkill {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    workflow: &'static str,
}
const BUILTINS: &[BuiltinSkill] = &[
    BuiltinSkill {
        id: "vclogg-docs",
        name: "VC log DOCS",
        description: "VCLogg 日志、文件、搜索、导航、标记与高亮工具的完整使用手册。",
        workflow: workflows::VCLOGG_DOCS,
    },
    BuiltinSkill {
        id: "search-logs",
        name: "搜索日志",
        description: "调查原因、标记关键证据并高亮关键词。",
        workflow: workflows::ANALYSIS,
    },
    BuiltinSkill {
        id: "execute-search",
        name: "执行搜索",
        description: "区分搜索操作并保留已验证证据。",
        workflow: workflows::ANALYSIS,
    },
    BuiltinSkill {
        id: "open-file",
        name: "打开文件",
        description: "处理同名文件、未打开目标和后续操作身份。",
        workflow: workflows::FILES,
    },
    BuiltinSkill {
        id: "close-file",
        name: "关闭文件",
        description: "处理关闭确认、部分完成和失效引用。",
        workflow: workflows::FILES,
    },
    BuiltinSkill {
        id: "switch-file",
        name: "切换文件",
        description: "切换并保留视口，确定后续搜索目标。",
        workflow: workflows::FILES,
    },
    BuiltinSkill {
        id: "locate-file",
        name: "定位日志文件",
        description: "区分查找路径、侧栏定位和跳转日志行。",
        workflow: workflows::FILES,
    },
    BuiltinSkill {
        id: "text-marks",
        name: "添加文字标记",
        description: "确定语义目标并避免重复标注。",
        workflow: workflows::MARKS,
    },
    BuiltinSkill {
        id: "color-labels",
        name: "关键词颜色标签",
        description: "使用已有颜色标签高亮关键词，不创建标签。",
        workflow: workflows::MARKS,
    },
    BuiltinSkill {
        id: "line-marks",
        name: "添加行标记",
        description: "区分行书签、文字注释和关键词着色。",
        workflow: workflows::MARKS,
    },
    BuiltinSkill {
        id: "navigate-log",
        name: "定位日志行",
        description: "区分源行与结果序号，处理历史引用。",
        workflow: workflows::NAVIGATION,
    },
    BuiltinSkill {
        id: if cfg!(target_os = "windows") {
            "shell-windows"
        } else if cfg!(target_os = "macos") {
            "shell-macos"
        } else {
            "shell-linux"
        },
        name: if cfg!(target_os = "windows") {
            "Windows 命令行"
        } else if cfg!(target_os = "macos") {
            "macOS 命令行"
        } else {
            "Linux 命令行"
        },
        description: if cfg!(target_os = "windows") {
            "在隐藏窗口的 cmd.exe 中安全使用工作区命令。"
        } else if cfg!(target_os = "macos") {
            "在无 Terminal 窗口的 POSIX shell 中安全使用工作区命令。"
        } else {
            "在非交互 POSIX shell 中安全使用工作区命令。"
        },
        workflow: if cfg!(target_os = "windows") {
            workflows::SHELL_WINDOWS
        } else if cfg!(target_os = "macos") {
            workflows::SHELL_MACOS
        } else {
            workflows::SHELL_LINUX
        },
    },
];

pub(crate) fn initialize_builtin_skills(
    settings: &mut AiSettings,
    settings_path: &Path,
) -> Result<bool> {
    let directory = settings_path
        .parent()
        .context("Missing AI configuration directory")?
        .join("skills")
        .join("builtin");
    let mut changed = false;
    for builtin in BUILTINS {
        let id = format!("vclogg:{}", builtin.id);
        let initialized = settings.initialized_builtin_skills.contains(&id);
        let existing = settings.skills.iter().position(|s| s.id == id);
        // Removing an entry is durable. Updating a release must not restore it.
        if initialized && existing.is_none() {
            continue;
        }
        let leaf = existing
            .map(|ix| settings.skills[ix].directory.clone())
            .unwrap_or_else(|| directory.join(builtin.id));
        if initialized && !leaf.join("SKILL.md").is_file() {
            continue;
        }
        let markdown = format!(
            "---\nname: {}\ndescription: {}\n---\n\n{}\n",
            builtin.name, builtin.description, builtin.workflow
        );
        let old = legacy::markdown(builtin.id);
        let previous = old.as_deref().into_iter().collect::<Vec<_>>();
        if let Some(ix) = existing {
            // Invalid or temporarily unreadable user edits must not prevent the
            // remaining AI configuration from loading. Refresh reports their errors.
            let mut readable = settings.skills[ix].clone();
            readable.enabled = true;
            if crate::read_skill_file(&readable, "SKILL.md").is_err() {
                continue;
            }
        }
        let updated = crate::defaults::update_default(
            &leaf.join("SKILL.md"),
            &markdown,
            &previous,
            !initialized,
        )?;
        if let Some(ix) = existing {
            let mut readable = settings.skills[ix].clone();
            readable.enabled = true;
            let managed = crate::read_skill_file(&readable, "SKILL.md")
                .ok()
                .as_deref()
                == Some(markdown.as_str());
            // Also repair stale metadata if a previous launch upgraded the file
            // but did not finish saving settings. Preserve the enable switch.
            if managed {
                let skill = &mut settings.skills[ix];
                if skill.name != builtin.name || skill.description != builtin.description {
                    skill.name = builtin.name.into();
                    skill.description = builtin.description.into();
                    changed = true;
                }
            }
            changed |= updated;
        } else {
            fs::create_dir_all(&directory)?;
            let root = directory.canonicalize()?;
            if !settings
                .skill_directories
                .iter()
                .any(|r| r.directory == root)
            {
                settings.skill_directories.push(SkillDirectory {
                    id: "vclogg:builtins".into(),
                    name: "VCLogg 内置 / Built-in".into(),
                    directory: root.clone(),
                    enabled: true,
                });
            }
            let mut skill = crate::import_skill(&leaf)?;
            skill.id = id.clone();
            skill.source_directory = Some(root);
            if !settings
                .skills
                .iter()
                .any(|s| s.directory == skill.directory)
            {
                settings.skills.push(skill);
                changed = true;
            }
        }
        if !initialized {
            settings.initialized_builtin_skills.push(id);
            changed = true;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_analysis_skills_preserve_verified_evidence_visually() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config");
        fs::create_dir_all(&config).unwrap();
        let settings_path = config.join("ai.json");
        let mut settings = AiSettings::default();

        assert!(initialize_builtin_skills(&mut settings, &settings_path).unwrap());
        let read = |id: &str| {
            let skill = settings.skills.iter().find(|skill| skill.id == id).unwrap();
            fs::read_to_string(skill.directory.join("SKILL.md")).unwrap()
        };
        let analysis = read("vclogg:search-logs");
        for instruction in [
            "少量决定性行加书签",
            "调用一次 `list_colors`",
            "各相关文件高亮 1–3 个实际观察到的精确高信号词",
            "用户要求只读",
        ] {
            assert!(analysis.contains(instruction), "missing {instruction}");
        }
        let marking = read("vclogg:color-labels");
        assert!(marking.contains("不应在书签足够时自动添加说明"));
        assert!(marking.contains("每文件选择 1–3 个实际观察到的高信号词"));
        let docs = read("vclogg:vclogg-docs");
        for instruction in [
            "{\"group\":\"vclogg\"}",
            "## 当前状态与文件发现",
            "## 日志读取与后台搜索",
            "## 应用搜索视图",
            "## 书签、注释与高亮",
            "`confirmation_pending`",
        ] {
            assert!(docs.contains(instruction), "missing {instruction}");
        }
        let shell = settings
            .skills
            .iter()
            .find(|skill| skill.id.starts_with("vclogg:shell-"))
            .expect("current platform shell skill");
        let shell = fs::read_to_string(shell.directory.join("SKILL.md")).unwrap();
        assert!(shell.contains("Skill 只说明语法，不授予额外权限"));
        assert!(shell.contains("工作目录固定为所选工作区"));
        assert!(workflows::SHELL_WINDOWS.contains("cmd.exe /D /S /C"));
        assert!(workflows::SHELL_LINUX.contains("/bin/sh -c"));
        assert!(workflows::SHELL_MACOS.contains("BSD 参数"));
    }
}
