use super::*;
use crate::AiConversationRecord;

impl StateRepository {
    pub fn ai_conversations(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<AiConversationRecord>> {
        self.search_ai_conversations("", false, offset, limit)
    }
    pub fn search_ai_conversations(
        &self,
        query: &str,
        archived: bool,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<AiConversationRecord>> {
        let connection = self.lock()?;
        let mut statement = connection.prepare("SELECT c.id, c.title, '', c.revision FROM ai_conversations c LEFT JOIN ai_conversation_flags f ON f.id=c.id WHERE coalesce(f.archived,0)=?1 AND instr(lower(c.title),lower(?2))>0 ORDER BY coalesce(f.pinned,0) DESC, c.updated_at DESC, c.id LIMIT ?3 OFFSET ?4")?;
        statement
            .query_map(
                params![archived, query, limit.min(100) as i64, offset as i64],
                |row| {
                    Ok(AiConversationRecord {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        payload: row.get(2)?,
                        revision: row.get(3)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Could not search conversations")
    }
    pub fn ai_conversation_flags(&self) -> Result<BTreeMap<String, (bool, bool)>> {
        self.lock()?
            .prepare("SELECT id,pinned,archived FROM ai_conversation_flags")?
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
            .collect::<rusqlite::Result<_>>()
            .context("Could not load conversation flags")
    }
    pub fn set_ai_conversation_flags(&self, id: &str, pinned: bool, archived: bool) -> Result<()> {
        // Flags are independent of the transcript revision: archiving a running
        // conversation must not race its streamed transcript writes.
        if !pinned && !archived {
            self.lock()?
                .execute("DELETE FROM ai_conversation_flags WHERE id=?1", [id])?;
        } else {
            self.lock()?.execute("INSERT INTO ai_conversation_flags(id,pinned,archived) VALUES (?1,?2,?3) ON CONFLICT(id) DO UPDATE SET pinned=?2,archived=?3", params![id,pinned,archived])?;
        }
        Ok(())
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
        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction()?;
        let count = transaction.execute(
            "DELETE FROM ai_conversations WHERE id=?1 AND revision=?2",
            params![id, revision],
        )?;
        if count == 0 {
            anyhow::bail!("Conversation changed in another window; reload it before deleting");
        }
        transaction.execute("DELETE FROM ai_conversation_flags WHERE id=?1", [id])?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flags_survive_migration_and_do_not_conflict_with_transcript_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let defaults = StateMigrationDefaults {
            app_log_level: "error".into(),
            color_labels: Vec::new(),
        };
        let store = StateRepository::open(path.clone(), &defaults).unwrap();
        for (id, title) in [("a", "Normal"), ("z", "Timeout 100%_分析")] {
            store
                .save_ai_conversation(&AiConversationRecord {
                    id: id.into(),
                    title: title.into(),
                    payload: "{}".into(),
                    revision: 0,
                })
                .unwrap();
        }
        drop(store);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("DROP TABLE ai_conversation_flags; PRAGMA user_version=20;")
            .unwrap();
        drop(connection);
        let store = StateRepository::open(path.clone(), &defaults).unwrap();
        store.set_ai_conversation_flags("z", true, false).unwrap();
        assert_eq!(store.ai_conversations(0, 100).unwrap()[0].id, "z");
        assert_eq!(
            store
                .search_ai_conversations("100%_", false, 0, 100)
                .unwrap()
                .len(),
            1
        );
        let mut record = store.load_ai_conversation("z").unwrap().unwrap();
        store.set_ai_conversation_flags("z", true, true).unwrap();
        record.revision = store.save_ai_conversation(&record).unwrap();
        assert_eq!(store.ai_conversations(0, 100).unwrap().len(), 1);
        assert_eq!(
            store
                .search_ai_conversations("timeout", true, 0, 100)
                .unwrap()[0]
                .id,
            "z"
        );
        drop(store);
        let store = StateRepository::open(path, &defaults).unwrap();
        assert_eq!(store.ai_conversation_flags().unwrap()["z"], (true, true));
        assert!(store.delete_ai_conversation("z", 1).is_err());
        assert!(store.ai_conversation_flags().unwrap().contains_key("z"));
        store.delete_ai_conversation("z", record.revision).unwrap();
        assert!(!store.ai_conversation_flags().unwrap().contains_key("z"));
    }
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
            assert_eq!(store.schema_version().unwrap(), STATE_SCHEMA_VERSION);
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
