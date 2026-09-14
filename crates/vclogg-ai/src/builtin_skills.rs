//! Ten discoverable entries backed by four shared workflow guides.
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
        id: "search-logs",
        name: "搜索日志 · Search logs",
        description: "搜索与分析：模糊症状、关联请求、验证原因 / Investigate symptoms and correlate evidence.",
        workflow: workflows::ANALYSIS,
    },
    BuiltinSkill {
        id: "execute-search",
        name: "执行搜索 · Execute search",
        description: "搜索与分析：区分后台取证、展示搜索和编辑草稿 / Choose analysis, displayed search or draft editing.",
        workflow: workflows::ANALYSIS,
    },
    BuiltinSkill {
        id: "open-file",
        name: "打开文件 · Open file",
        description: "文件操作：处理同名文件、未打开目标和后续操作身份 / Resolve files and opening dependencies.",
        workflow: workflows::FILES,
    },
    BuiltinSkill {
        id: "close-file",
        name: "关闭文件 · Close file",
        description: "文件操作：处理关闭确认、部分完成和失效引用 / Handle close confirmation and stale references.",
        workflow: workflows::FILES,
    },
    BuiltinSkill {
        id: "switch-file",
        name: "切换文件 · Switch file",
        description: "文件操作：切换并保留视口、确定后续搜索目标 / Preserve view state across file operations.",
        workflow: workflows::FILES,
    },
    BuiltinSkill {
        id: "locate-file",
        name: "定位日志文件 · Locate files",
        description: "文件操作：区分查找路径、侧栏显露和跳转日志行 / Disambiguate file discovery and navigation.",
        workflow: workflows::FILES,
    },
    BuiltinSkill {
        id: "text-marks",
        name: "添加文字标记 · Text annotations",
        description: "标记与高亮：确定语义目标、避免重复标注 / Resolve annotation targets and avoid duplicate edits.",
        workflow: workflows::MARKS,
    },
    BuiltinSkill {
        id: "color-labels",
        name: "关键词颜色标签 · Keyword color labels",
        description: "标记与高亮：使用已有颜色标签高亮关键词，不创建标签 / Apply existing keyword colors.",
        workflow: workflows::MARKS,
    },
    BuiltinSkill {
        id: "line-marks",
        name: "添加行标记 · Line bookmarks",
        description: "标记与高亮：区分行标记、文字注释和关键词着色 / Choose bookmark, annotation or keyword highlighting.",
        workflow: workflows::MARKS,
    },
    BuiltinSkill {
        id: "navigate-log",
        name: "定位日志行 · Navigate logs",
        description: "导航定位：区分源行与结果序号、处理历史引用 / Navigate source rows and search projections.",
        workflow: workflows::NAVIGATION,
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
        let old = legacy::markdown(builtin.id).context("Missing legacy skill default")?;
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
            &[&old],
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
