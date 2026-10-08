use super::*;

impl StateRepository {
    pub fn ai_command_allowed(&self, directory: &Path, command: &str) -> Result<bool> {
        Ok(self.lock()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM ai_command_approvals WHERE directory=?1 AND command=?2)",
            params![encode_persisted_path(directory), command],
            |row| row.get(0),
        )?)
    }
    pub fn allow_ai_command(&self, directory: &Path, command: &str) -> Result<()> {
        anyhow::ensure!(
            directory.is_absolute() && !command.is_empty(),
            "Invalid command approval"
        );
        self.lock()?.execute(
            "INSERT OR IGNORE INTO ai_command_approvals(directory,command) VALUES (?1,?2)",
            params![encode_persisted_path(directory), command],
        )?;
        Ok(())
    }
    pub fn ai_command_approvals(&self) -> Result<Vec<(PathBuf, String)>> {
        self.lock()?
            .prepare(
                "SELECT directory,command FROM ai_command_approvals ORDER BY directory,command",
            )?
            .query_map([], |row| {
                let directory: String = row.get(0)?;
                Ok((decode_persisted_path(&directory), row.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()
            .context("Could not load command approvals")
    }
    pub fn revoke_ai_command(&self, directory: &Path, command: &str) -> Result<()> {
        self.lock()?.execute(
            "DELETE FROM ai_command_approvals WHERE directory=?1 AND command=?2",
            params![encode_persisted_path(directory), command],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_reopen_exact_matching_and_revoke() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let defaults = StateMigrationDefaults {
            app_log_level: "error".into(),
            color_labels: vec![],
        };
        let store = StateRepository::open(path.clone(), &defaults).unwrap();
        store.save_ui_value("existing", "retained").unwrap();
        drop(store);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE ai_command_approvals; PRAGMA user_version=21;")
            .unwrap();
        drop(conn);
        let root = dir.path().canonicalize().unwrap();
        let store = StateRepository::open(path.clone(), &defaults).unwrap();
        store.allow_ai_command(&root, "cargo test").unwrap();
        store.allow_ai_command(&root, "cargo test").unwrap();
        drop(store);
        for _ in 0..2 {
            let store = StateRepository::open(path.clone(), &defaults).unwrap();
            assert_eq!(
                store.load_ui_value("existing").unwrap().as_deref(),
                Some("retained")
            );
            assert!(store.ai_command_allowed(&root, "cargo test").unwrap());
            assert!(!store.ai_command_allowed(&root, "cargo test --all").unwrap());
            assert!(
                !store
                    .ai_command_allowed(&root.join("other"), "cargo test")
                    .unwrap()
            );
            assert_eq!(
                store.ai_command_approvals().unwrap(),
                vec![(root.clone(), "cargo test".into())]
            );
        }
        let store = StateRepository::open(path, &defaults).unwrap();
        store.revoke_ai_command(&root, "cargo test").unwrap();
        assert!(!store.ai_command_allowed(&root, "cargo test").unwrap());
    }
}
