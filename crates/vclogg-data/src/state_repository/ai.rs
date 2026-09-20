use super::*;
use crate::AiConversationRecord;

impl StateRepository {
    pub fn ai_conversations(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<AiConversationRecord>> {
        let connection = self.lock()?;
        let mut statement = connection.prepare("SELECT id, title, '', revision FROM ai_conversations ORDER BY updated_at DESC, id LIMIT ?1 OFFSET ?2")?;
        statement
            .query_map(params![limit.min(100) as i64, offset as i64], |row| {
                Ok(AiConversationRecord {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    payload: row.get(2)?,
                    revision: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Could not list AI conversations")
    }
    pub fn load_ai_conversation(&self, id: &str) -> Result<Option<AiConversationRecord>> {
        self.lock()?
            .query_row(
                "SELECT id, title, payload, revision FROM ai_conversations WHERE id = ?1",
                [id],
                |row| {
                    Ok(AiConversationRecord {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        payload: row.get(2)?,
                        revision: row.get(3)?,
                    })
                },
            )
            .optional()
            .context("Could not load AI conversation")
    }
    /// Compare-and-swap prevents a second window from overwriting a newer conversation.
    pub fn save_ai_conversation(&self, record: &AiConversationRecord) -> Result<u64> {
        let connection = self.lock()?;
        let changed = if record.revision == 0 {
            connection.execute("INSERT OR IGNORE INTO ai_conversations(id,title,payload,revision,updated_at) VALUES (?1,?2,?3,1,?4)", params![record.id, record.title, record.payload, unix_timestamp()])?
        } else {
            connection.execute("UPDATE ai_conversations SET title=?2,payload=?3,revision=revision+1,updated_at=?4 WHERE id=?1 AND revision=?5", params![record.id, record.title, record.payload, unix_timestamp(), record.revision])?
        };
        if changed != 1 {
            anyhow::bail!("Conversation changed in another window; reload it before continuing");
        }
        Ok(record.revision + 1)
    }
    pub fn delete_ai_conversation(&self, id: &str, revision: u64) -> Result<()> {
        let count = self.lock()?.execute(
            "DELETE FROM ai_conversations WHERE id=?1 AND revision=?2",
            params![id, revision],
        )?;
        if count == 0 {
            anyhow::bail!("Conversation changed in another window; reload it before deleting");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_seventeen_migration_is_additive_and_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let defaults = StateMigrationDefaults {
            app_log_level: "error".into(),
            color_labels: Vec::new(),
        };
        let store = StateRepository::open(path.clone(), &defaults).unwrap();
        store.save_ui_value("existing", "kept").unwrap();
        drop(store);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("DROP TABLE ai_conversations; PRAGMA user_version=17;")
            .unwrap();
        drop(connection);
        for _ in 0..2 {
            let store = StateRepository::open(path.clone(), &defaults).unwrap();
            assert_eq!(store.schema_version().unwrap(), 19);
            assert_eq!(
                store.load_ui_value("existing").unwrap().as_deref(),
                Some("kept")
            );
            assert!(store.ai_conversations(0, 100).unwrap().is_empty());
        }
    }
    #[test]
    fn ai_revision_conflicts_and_reopen_preserve_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let defaults = StateMigrationDefaults {
            app_log_level: "error".into(),
            color_labels: Vec::new(),
        };
        let store = StateRepository::open(path.clone(), &defaults).unwrap();
        let mut record = AiConversationRecord {
            id: "one".into(),
            title: "分析".into(),
            payload: "{\"messages\":[]}".into(),
            revision: 0,
        };
        record.revision = store.save_ai_conversation(&record).unwrap();
        let revision = store.save_ai_conversation(&record).unwrap();
        assert!(store.save_ai_conversation(&record).is_err());
        assert!(
            store
                .delete_ai_conversation(&record.id, record.revision)
                .is_err()
        );
        drop(store);
        let store = StateRepository::open(path, &defaults).unwrap();
        assert_eq!(
            store.load_ai_conversation("one").unwrap().unwrap().payload,
            record.payload
        );
        store.delete_ai_conversation("one", revision).unwrap();
        assert!(store.ai_conversations(0, 100).unwrap().is_empty());
    }
}
