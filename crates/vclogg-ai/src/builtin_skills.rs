//! Discoverable workflows and mandatory, non-optional platform shell guidance.
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
const OLD_BUILTINS: &[BuiltinSkill] = &[
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

const BUILTINS: &[BuiltinSkill] = &[
    BuiltinSkill {
        id: "log-investigation",
        name: "日志调查",
        description: "以搜索、证据、因果和反例调查日志问题。",
        workflow: workflows::ANALYSIS,
    },
    BuiltinSkill {
        id: "workspace-operations",
        name: "文件与视图操作",
        description: "解析文件身份，操作文件标签、搜索视图并导航源行。",
        workflow: workflows::WORKSPACE,
    },
    BuiltinSkill {
        id: "annotations",
        name: "标注与高亮",
        description: "用书签、文字注释和已有颜色标签保存已验证发现。",
        workflow: workflows::MARKS,
    },
    BuiltinSkill {
        id: "source-correlation",
        name: "源码关联",
        description: "通过源码符号、定义、引用和命令行验证日志来源。",
        workflow: workflows::SOURCE,
    },
];

pub(crate) fn shell_instructions() -> String {
    let legacy = if cfg!(target_os = "windows") {
        workflows::SHELL_WINDOWS
    } else if cfg!(target_os = "macos") {
        workflows::SHELL_MACOS
    } else {
        workflows::SHELL_LINUX
    };
    // Keep shipped legacy text unchanged for conservative migration of old skills.
    legacy.replace("工作目录固定为所选工作区", "root 可省略，默认在唯一工作区目录中执行，用于临时副本、输出和转储；显式 root 只选择项目目录作为相对路径基准")
        .replace("先用 `list_source_workspaces` 取得数字 `root`，路径尽量相对于该根目录。", "已知绝对路径时直接读取或搜索，不需要查询 root、添加项目目录或打开文件标签。只有需要项目范围或相对路径且上下文未提供 root 时，才调用 list_source_workspaces；不要重复查询已有 root。")
}

const MIGRATIONS: &[(&str, &[&str])] = &[
    ("log-investigation", &["search-logs", "execute-search"]),
    (
        "workspace-operations",
        &[
            "open-file",
            "close-file",
            "switch-file",
            "locate-file",
            "navigate-log",
        ],
    ),
    ("annotations", &["text-marks", "color-labels", "line-marks"]),
];

fn migrate_catalog(settings: &mut AiSettings) -> bool {
    if settings.builtin_skill_catalog_version >= 1 {
        return false;
    }
    for (new, old) in MIGRATIONS {
        let initialized = old.iter().any(|id| {
            settings
                .initialized_builtin_skills
                .contains(&format!("vclogg:{id}"))
        });
        let present = settings
            .skills
            .iter()
            .any(|s| old.iter().any(|id| s.id == format!("vclogg:{id}")));
        // A removed workflow must not silently regain guidance after an upgrade.
        if initialized && !present {
            settings
                .initialized_builtin_skills
                .push(format!("vclogg:{new}"));
        }
    }
    settings.skills.retain(|skill| {
        let Some(builtin) = OLD_BUILTINS
            .iter()
            .find(|b| skill.id == format!("vclogg:{}", b.id))
        else {
            return true;
        };
        let mut readable = skill.clone();
        readable.enabled = true;
        let Ok(content) = crate::read_skill_file(&readable, "SKILL.md") else {
            return true;
        };
        let expected = format!(
            "---\nname: {}\ndescription: {}\n---\n\n{}\n",
            builtin.name, builtin.description, builtin.workflow
        );
        let previous_shell = expected
            .replace("只读命令可直接读取与任务相关的工作区外文件，父目录和绝对路径本身无需再次询问用户。", "只读命令可直接运行。")
            .replace("环境变量展开、PowerShell/WSL", "环境变量展开、绝对路径、PowerShell/WSL")
            .replace("变量或命令替换、联网", "变量或命令替换、父目录/绝对路径、联网");
        content != expected
            && !(builtin.id.starts_with("shell-") && content == previous_shell)
            && legacy::markdown(builtin.id).as_deref() != Some(content.as_str())
    });
    settings.builtin_skill_catalog_version = 1;
    true
}

pub(crate) fn initialize_builtin_skills(
    settings: &mut AiSettings,
    settings_path: &Path,
) -> Result<bool> {
    let directory = settings_path
        .parent()
        .context("Missing AI configuration directory")?
        .join("skills")
        .join("builtin");
    let disabled = MIGRATIONS
        .iter()
        .filter_map(|(new, old)| {
            let previous = settings
                .skills
                .iter()
                .filter(|s| old.iter().any(|id| s.id == format!("vclogg:{id}")))
                .collect::<Vec<_>>();
            (settings.builtin_skill_catalog_version == 0
                && !previous.is_empty()
                && previous.iter().all(|s| !settings.skill_enabled(s)))
            .then(|| format!("vclogg:{new}"))
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut changed = migrate_catalog(settings);
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
            builtin.name,
            builtin.description,
            builtin
                .workflow
                .replace(
                    "`search-logs` 与 `execute-search` 共用本指南，每轮只读一次。",
                    "读取证据加载 logs，保存标注或显示搜索加载 vclogg_actions。"
                )
                .replace(
                    "三个标记技能共用本指南，每轮只读一次。",
                    "加载 vclogg_actions 使用标注工具。"
                )
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
            if disabled.contains(&id) {
                skill.enabled = false;
            }
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
    fn migration_preserves_edits_disabled_and_removed_workflows() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = AiSettings::default();
        for builtin in OLD_BUILTINS {
            let id = format!("vclogg:{}", builtin.id);
            settings.initialized_builtin_skills.push(id.clone());
            // All old file/navigation entries were explicitly removed.
            if [
                "open-file",
                "close-file",
                "switch-file",
                "locate-file",
                "navigate-log",
            ]
            .contains(&builtin.id)
            {
                continue;
            }
            let leaf = directory.path().join(builtin.id);
            fs::create_dir_all(&leaf).unwrap();
            let markdown = format!(
                "---\nname: {}\ndescription: {}\n---\n\n{}\n",
                builtin.name, builtin.description, builtin.workflow
            );
            fs::write(
                leaf.join("SKILL.md"),
                if builtin.id == "vclogg-docs" {
                    format!("{markdown}\nUser instructions\n")
                } else {
                    markdown
                },
            )
            .unwrap();
            let mut skill = crate::import_skill(&leaf).unwrap();
            skill.id = id;
            skill.enabled = !["search-logs", "execute-search"].contains(&builtin.id);
            settings.skills.push(skill);
        }
        let path = directory.path().join("ai.json");
        assert!(initialize_builtin_skills(&mut settings, &path).unwrap());
        assert_eq!(settings.builtin_skill_catalog_version, 1);
        assert!(settings.skills.iter().any(|s| s.id == "vclogg:vclogg-docs"));
        assert!(
            !settings
                .skills
                .iter()
                .find(|s| s.id == "vclogg:log-investigation")
                .unwrap()
                .enabled
        );
        assert!(
            !settings
                .skills
                .iter()
                .any(|s| s.id == "vclogg:workspace-operations")
        );
        assert!(!settings.skills.iter().any(|s| s.id == "vclogg:search-logs"));
        assert!(!initialize_builtin_skills(&mut settings, &path).unwrap());
    }

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
        let analysis = read("vclogg:log-investigation");
        for instruction in [
            "少量决定性行加书签",
            "调用一次 `list_colors`",
            "各相关文件高亮 1–3 个实际观察到的精确高信号词",
            "用户要求只读",
        ] {
            assert!(analysis.contains(instruction), "missing {instruction}");
        }
        let marking = read("vclogg:annotations");
        assert!(marking.contains("不应在书签足够时自动添加说明"));
        assert!(marking.contains("每文件选择 1–3 个实际观察到的高信号词"));
        assert_eq!(settings.skills.len(), 4);
        assert!(read("vclogg:workspace-operations").contains("confirmation_pending"));
        assert!(read("vclogg:source-correlation").contains("source_symbols"));
        let shell = shell_instructions();
        assert!(shell.contains("Skill 只说明语法，不授予额外权限"));
        assert!(shell.contains("root 可省略"));
        assert!(!shell.contains("先用 `list_source_workspaces`"));
        assert!(workflows::SHELL_WINDOWS.contains("cmd.exe /D /S /C"));
        assert!(workflows::SHELL_LINUX.contains("/bin/sh -c"));
        assert!(workflows::SHELL_MACOS.contains("BSD 参数"));
    }
}
