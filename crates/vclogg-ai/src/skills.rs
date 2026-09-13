use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub directory: PathBuf,
    pub enabled: bool,
}

pub fn import_skill(directory: &Path) -> Result<Skill> {
    let directory = directory
        .canonicalize()
        .context("Skill directory is unavailable")?;
    let mut skill = Skill {
        id: uuid::Uuid::new_v4().to_string(),
        name: String::new(),
        description: String::new(),
        directory,
        enabled: true,
    };
    refresh_skill(&mut skill)?;
    Ok(skill)
}
pub fn refresh_skill(skill: &mut Skill) -> Result<()> {
    let text = read_skill_file(skill, "SKILL.md")?;
    let lines = text.lines().collect::<Vec<_>>();
    skill.name = skill
        .directory
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into();
    skill.description.clear();
    if lines.first().is_some_and(|l| l.trim() == "---") {
        let mut description_block = false;
        for line in lines.iter().skip(1).take_while(|l| l.trim() != "---") {
            if let Some(name) = line.strip_prefix("name:") {
                skill.name = scalar(name);
                description_block = false;
            } else if let Some(description) = line.strip_prefix("description:") {
                description_block = matches!(description.trim(), ">" | "|" | ">-" | "|-");
                if !description_block {
                    skill.description = scalar(description);
                }
            } else if description_block && line.starts_with(char::is_whitespace) {
                if !skill.description.is_empty() {
                    skill.description.push(' ');
                }
                skill.description.push_str(line.trim());
            } else {
                description_block = false;
            }
        }
    }
    if skill.description.is_empty() {
        skill.description = text
            .lines()
            .find(|l| !l.trim().is_empty() && !l.starts_with('#') && *l != "---")
            .unwrap_or("Log analysis skill")
            .chars()
            .take(500)
            .collect();
    }
    skill.name = skill.name.chars().take(100).collect();
    skill.description = skill.description.chars().take(2000).collect();
    Ok(())
}
fn scalar(value: &str) -> String {
    let value = value.trim();
    if value.starts_with('"') {
        serde_json::from_str(value).unwrap_or_else(|_| value.trim_matches('"').into())
    } else {
        value.trim_matches('\'').replace("''", "'")
    }
}
pub fn read_skill_file(skill: &Skill, relative: &str) -> Result<String> {
    if !skill.enabled {
        bail!("Skill is disabled");
    }
    let relative = Path::new(relative);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        bail!("Only relative skill paths are allowed");
    }
    let root = skill.directory.canonicalize()?;
    if root != skill.directory {
        bail!("Skill directory changed; import it again");
    }
    let file = root
        .join(relative)
        .canonicalize()
        .context("Skill reference is unavailable")?;
    if !file.starts_with(&root) || !file.is_file() {
        bail!("Reference escapes the skill directory");
    }
    // Limit before allocation as well as after read (the file can grow concurrently).
    use std::io::Read as _;
    let mut bytes = Vec::new();
    fs::File::open(file)?
        .take((crate::RESULT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > crate::RESULT_BYTES {
        bail!("Skill reference exceeds 64 KiB");
    }
    let text = String::from_utf8(bytes).context("Skill reference must be UTF-8 text")?;
    if text.contains('\0') {
        bail!("Binary skill references are not supported");
    }
    Ok(text)
}
