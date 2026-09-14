use crate::StateRepository;
use anyhow::{Context as _, Result, bail};
use rusqlite::{OptionalExtension as _, params};
use serde::{Deserialize, Serialize};

/// Local, cross-conversation memory with optimistic concurrency control.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiMemoryRecord {
    id: String,
    title: String,
    content: String,
    revision: u64,
}
impl AiMemoryRecord {
    pub fn new(id: String, title: String, content: String, revision: u64) -> Self {
        Self {
            id,
            title,
            content,
            revision,
        }
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn content(&self) -> &str {
        &self.content
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    fn key(&self) -> Result<String> {
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            bail!("Invalid memory ID");
        }
        Ok(format!("ai.memory.{}", self.id))
    }
}
impl StateRepository {
    /// Uses a private key namespace in the existing durable key/value table.
    /// Old databases need no migration; an absent namespace is an empty memory.
    pub fn ai_memories(&self) -> Result<Vec<AiMemoryRecord>> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT value FROM ui_state WHERE key GLOB 'ai.memory.*' ORDER BY key LIMIT 200",
        )?;
        let values = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        values
            .into_iter()
            .map(|value| serde_json::from_str(&value).context("Could not read AI memory"))
            .collect()
    }
    pub fn save_ai_memory(&self, record: &AiMemoryRecord) -> Result<AiMemoryRecord> {
        let key = record.key()?;
        if record.title.trim().is_empty()
            || record.title.chars().count() > 120
            || record.content.trim().is_empty()
            || record.content.chars().count() > 4096
        {
            bail!("Memory requires a title (1–120 characters) and content (1–4096 characters)");
        }
        let mut connection = self.lock()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let previous: Option<String> = transaction
            .query_row("SELECT value FROM ui_state WHERE key=?1", [&key], |row| {
                row.get(0)
            })
            .optional()?;
        let revision = previous
            .as_deref()
            .map(serde_json::from_str::<AiMemoryRecord>)
            .transpose()?
            .map_or(0, |r| r.revision);
        if revision != record.revision || (previous.is_some() && record.revision == 0) {
            bail!("Memory changed in another window; refresh before saving");
        }
        if previous.is_none() {
            let count: usize = transaction.query_row(
                "SELECT COUNT(*) FROM ui_state WHERE key GLOB 'ai.memory.*'",
                [],
                |row| row.get(0),
            )?;
            if count >= 200 {
                bail!("Memory limit reached (200 entries); remove an entry first");
            }
        }
        let mut saved = record.clone();
        saved.title = saved.title.trim().into();
        saved.content = saved.content.trim().into();
        saved.revision = revision
            .checked_add(1)
            .context("Memory revision exhausted")?;
        transaction.execute("INSERT INTO ui_state(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, serde_json::to_string(&saved)?])?;
        transaction.commit()?;
        Ok(saved)
    }
    pub fn delete_ai_memory(&self, id: &str, revision: u64) -> Result<()> {
        let key = AiMemoryRecord::new(id.into(), String::new(), String::new(), revision).key()?;
        let mut connection = self.lock()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let previous: Option<String> = transaction
            .query_row("SELECT value FROM ui_state WHERE key=?1", [&key], |row| {
                row.get(0)
            })
            .optional()?;
        let previous: AiMemoryRecord = serde_json::from_str(
            previous
                .as_deref()
                .context("Memory no longer exists; refresh the list")?,
        )?;
        if previous.revision != revision {
            bail!("Memory changed in another window; refresh before deleting");
        }
        transaction.execute("DELETE FROM ui_state WHERE key=?1", [&key])?;
        transaction.commit()?;
        Ok(())
    }
}
