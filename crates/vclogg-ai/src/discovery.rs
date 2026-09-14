use crate::Skill;
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillDirectory {
    pub id: String,
    pub name: String,
    pub directory: PathBuf,
    pub enabled: bool,
}
impl SkillDirectory {
    pub fn contains(&self, skill: &Skill) -> bool {
        skill.directory.starts_with(&self.directory)
            || skill.source_directory.as_ref() == Some(&self.directory)
    }
}
#[derive(Default)]
pub struct SkillDiscovery {
    pub directories: Vec<SkillDirectory>,
    pub skills: Vec<Skill>,
    pub warnings: Vec<String>,
}
pub fn system_skill_directories() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let mut roots = Vec::new();
    if let Some(home) = home {
        roots.extend(
            [
                ".codex/skills",
                ".claude/skills",
                ".cursor/skills",
                ".agents/skills",
            ]
            .map(|p| home.join(p)),
        );
    }
    for (key, subdir) in [("CODEX_HOME", "skills"), ("CLAUDE_CONFIG_DIR", "skills")] {
        if let Some(path) = std::env::var_os(key) {
            roots.push(PathBuf::from(path).join(subdir));
        }
    }
    roots.retain(|p| p.is_dir());
    roots
}
pub fn discover_skills(paths: Vec<PathBuf>) -> SkillDiscovery {
    let mut result = SkillDiscovery::default();
    let mut visited = BTreeSet::new();
    for path in paths {
        match path.canonicalize() {
            Ok(root) => {
                if result.directories.iter().any(|r| r.directory == root) {
                    continue;
                }
                result.directories.push(SkillDirectory {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: if path.file_name().is_some_and(|name| name == "skills") {
                        path.parent().and_then(Path::file_name)
                    } else {
                        path.file_name()
                    }
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                    directory: root.clone(),
                    enabled: true,
                });
                let start = result.skills.len();
                if let Err(error) = scan(&root, 0, &mut visited, &mut result) {
                    result.warnings.push(error.to_string());
                }
                for skill in &mut result.skills[start..] {
                    skill.source_directory = Some(root.clone());
                }
            }
            Err(error) => result.warnings.push(format!("{}: {error}", path.display())),
        }
    }
    result
}
fn scan(
    path: &Path,
    depth: usize,
    visited: &mut BTreeSet<PathBuf>,
    result: &mut SkillDiscovery,
) -> Result<()> {
    let canonical = path.canonicalize()?;
    if visited.contains(&canonical) {
        return Ok(());
    }
    if visited.len() >= 4096 || result.skills.len() >= 512 {
        anyhow::bail!("Skill scan limit reached; select a smaller folder");
    }
    visited.insert(canonical.clone());
    if canonical.join("SKILL.md").is_file() {
        match crate::import_skill(&canonical) {
            Ok(mut skill) => {
                skill.enabled = false;
                result.skills.push(skill);
            }
            Err(error) => result
                .warnings
                .push(format!("{}: {error}", canonical.display())),
        }
        return Ok(());
    }
    if depth >= 6 {
        return Ok(());
    }
    let mut children = fs::read_dir(&canonical)
        .with_context(|| format!("Cannot scan {}", canonical.display()))?
        .take(4097)
        .collect::<std::io::Result<Vec<_>>>()?;
    if children.len() > 4096 {
        anyhow::bail!("Folder has too many entries; select a smaller skills folder");
    }
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        // Follow symlinked skill leaves used by shared installations, but not arbitrary trees.
        let kind = entry.file_type()?;
        if (kind.is_dir() || (kind.is_symlink() && entry.path().join("SKILL.md").is_file()))
            && !matches!(
                entry.file_name().to_str(),
                Some(".git" | "node_modules" | "target")
            )
            && let Err(error) = scan(&entry.path(), depth + 1, visited, result)
        {
            result.warnings.push(error.to_string());
            if visited.len() >= 4096 || result.skills.len() >= 512 {
                break;
            }
        }
    }
    Ok(())
}
