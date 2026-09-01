//! SQLite repository for file history and workspace recovery records.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result};
use roaring::RoaringTreemap;
use rusqlite::{Connection, OptionalExtension as _, TransactionBehavior, params, params_from_iter};

use crate::{
    DatabaseInfo, FileSessionRecord, FileSessionRecords, HistorySession, LastWorkspaceFile,
    RecentFile, SessionRecordSaveResult, decode_persisted_path, encode_persisted_path,
};

const COMPRESSED_MARKED_ROWS_PREFIX: &str = "rb1:";

/// Owns SQLite access for durable file-history and workspace records.
pub struct StateRepository {
    connection: Mutex<Connection>,
    database_path: PathBuf,
}

impl StateRepository {
    /// Opens an already initialized state database.
    pub fn open(database_path: PathBuf) -> Result<Self> {
        let connection = Connection::open(&database_path)
            .with_context(|| format!("无法打开状态库：{}", database_path.display()))?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .context("无法设置状态库忙等待")?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .context("无法启用状态库外键")?;
        Ok(Self {
            connection: Mutex::new(connection),
            database_path,
        })
    }

    pub fn record_opened(&self, paths: &[PathBuf]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction().context("无法开始最近文件事务")?;
        let opened_at = unix_timestamp();
        for path in paths {
            transaction
                .execute(
                    "INSERT INTO file_sessions(path, last_opened_at)
                     VALUES (?1, ?2)
                     ON CONFLICT(path) DO UPDATE SET
                         last_opened_at = excluded.last_opened_at,
                         revision = file_sessions.revision + 1",
                    params![encode_persisted_path(path), opened_at],
                )
                .with_context(|| format!("无法记录最近文件：{}", path.display()))?;
        }
        transaction.commit().context("无法提交最近文件事务")
    }

    pub fn recent_files(&self, limit: usize) -> Result<Vec<RecentFile>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, path, last_opened_at
                 FROM file_sessions
                 ORDER BY pinned DESC, last_opened_at DESC, id DESC
                 LIMIT ?1",
            )
            .context("无法读取最近文件查询")?;
        let rows = statement
            .query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
                Ok(RecentFile {
                    id: row.get(0)?,
                    path: decode_persisted_path(&row.get::<_, String>(1)?),
                    last_opened_at: row.get(2)?,
                })
            })
            .context("无法查询最近文件")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("无法解析最近文件")
    }

    pub fn pinned_files(&self) -> Result<Vec<RecentFile>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, path, last_opened_at
                 FROM file_sessions
                 WHERE pinned = 1
                 ORDER BY last_opened_at DESC, id DESC",
            )
            .context("无法读取收藏文件查询")?;
        let rows = statement
            .query_map([], |row| {
                Ok(RecentFile {
                    id: row.get(0)?,
                    path: decode_persisted_path(&row.get::<_, String>(1)?),
                    last_opened_at: row.get(2)?,
                })
            })
            .context("无法查询收藏文件")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("无法解析收藏文件")
    }

    pub fn session_history(&self) -> Result<Vec<HistorySession>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, path, last_opened_at, revision, selected_row,
                        query_text, marked_rows, pinned
                 FROM file_sessions
                 ORDER BY pinned DESC, last_opened_at DESC, id DESC",
            )
            .context("无法读取文件会话历史查询")?;
        let rows = statement
            .query_map([], |row| {
                let selected_row = row
                    .get::<_, Option<i64>>(4)?
                    .and_then(|value| usize::try_from(value).ok());
                Ok(HistorySession {
                    id: row.get(0)?,
                    path: decode_persisted_path(&row.get::<_, String>(1)?),
                    last_opened_at: row.get(2)?,
                    revision: row.get(3)?,
                    selected_row,
                    query_text: row.get(5)?,
                    marked_rows_count: count_marked_rows(&row.get::<_, String>(6)?),
                    pinned: row.get::<_, i64>(7)? != 0,
                })
            })
            .context("无法查询文件会话历史")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("无法解析文件会话历史")
    }

    pub fn set_pinned(&self, path: &Path, pinned: bool) -> Result<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO file_sessions(path, last_opened_at, pinned)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(path) DO UPDATE SET
                     pinned = excluded.pinned,
                     revision = file_sessions.revision + 1",
                params![encode_persisted_path(path), unix_timestamp(), pinned],
            )
            .with_context(|| format!("无法更新文件收藏状态：{}", path.display()))?;
        Ok(())
    }

    pub fn clear_pinned(&self) -> Result<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "UPDATE file_sessions
                 SET pinned = 0, revision = revision + 1
                 WHERE pinned = 1",
                [],
            )
            .context("无法清空收藏文件")?;
        Ok(())
    }

    pub fn clear_history(&self, open_paths: &[PathBuf]) -> Result<usize> {
        let protected_paths = encoded_path_set(open_paths);
        let mut connection = self.lock()?;
        let transaction = connection.transaction().context("无法开始历史清理事务")?;
        let removable_ids = {
            let mut statement = transaction
                .prepare(
                    "SELECT id, path
                     FROM file_sessions
                     WHERE pinned = 0 AND marked_rows = ''",
                )
                .context("无法准备历史清理查询")?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .context("无法查询可清理历史")?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .context("无法解析可清理历史")?
                .into_iter()
                .filter_map(|(id, path)| (!protected_paths.contains(&path)).then_some(id))
                .collect::<Vec<_>>()
        };
        for id in &removable_ids {
            transaction
                .execute("DELETE FROM file_sessions WHERE id = ?1", [id])
                .context("无法删除历史记录")?;
        }
        transaction.commit().context("无法提交历史清理事务")?;
        Ok(removable_ids.len())
    }

    pub fn delete_history_session(&self, id: i64, open_paths: &[PathBuf]) -> Result<bool> {
        let protected_paths = encoded_path_set(open_paths);
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .context("无法开始历史记录删除事务")?;
        let session = transaction
            .query_row(
                "SELECT path, pinned, marked_rows FROM file_sessions WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)? != 0,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .context("无法解析历史记录保护状态")?;
        let Some((path, pinned, marked_rows)) = session else {
            return Ok(false);
        };
        if pinned || !marked_rows.is_empty() || protected_paths.contains(&path) {
            return Ok(false);
        }
        let removed = transaction
            .execute("DELETE FROM file_sessions WHERE id = ?1", [id])
            .context("无法删除文件会话历史")?;
        transaction.commit().context("无法提交历史记录删除事务")?;
        Ok(removed > 0)
    }

    pub fn last_workspace(&self) -> Result<Vec<LastWorkspaceFile>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT file_sessions.id, file_sessions.path,
                        file_sessions.last_opened_at, last_workspace_files.was_active
                 FROM last_workspace_files
                 JOIN file_sessions
                   ON file_sessions.id = last_workspace_files.file_session_id
                 ORDER BY last_workspace_files.position ASC",
            )
            .context("无法准备上一次工作区查询")?;
        let rows = statement
            .query_map([], |row| {
                Ok(LastWorkspaceFile {
                    id: row.get(0)?,
                    path: decode_persisted_path(&row.get::<_, String>(1)?),
                    last_opened_at: row.get(2)?,
                    was_active: row.get::<_, i64>(3)? != 0,
                })
            })
            .context("无法查询上一次工作区")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("无法解析上一次工作区")
    }

    pub fn load_session(&self, path: &Path) -> Result<Option<FileSessionRecord>> {
        let connection = self.lock()?;
        load_session_row(&connection, path)
    }

    pub fn load_sessions(&self, paths: &[PathBuf]) -> Result<FileSessionRecords> {
        const BATCH_SIZE: usize = 500;

        if paths.is_empty() {
            return Ok(BTreeMap::new());
        }
        let database_paths = paths
            .iter()
            .map(|path| encode_persisted_path(path))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let connection = self.lock()?;
        let mut sessions = BTreeMap::new();
        for batch in database_paths.chunks(BATCH_SIZE) {
            let placeholders = std::iter::repeat_n("?", batch.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT path, revision, custom_title, selected_row, query_text,
                        case_sensitive, regex, result_mode, marked_rows,
                        show_line_numbers, show_row_separators, keyword_color_rules, word_wrap,
                        resume_state
                 FROM file_sessions
                 WHERE path IN ({placeholders})"
            );
            let mut statement = connection
                .prepare(&sql)
                .context("无法准备批量文件会话查询")?;
            let rows = statement
                .query_map(params_from_iter(batch), |row| {
                    Ok((
                        decode_persisted_path(&row.get::<_, String>(0)?),
                        file_session_from_row(row, 1)?,
                    ))
                })
                .context("无法批量查询文件会话")?;
            for row in rows {
                let (path, session) = row.context("无法解析批量文件会话")?;
                sessions.insert(path, session);
            }
        }
        Ok(sessions)
    }

    pub fn save_session(
        &self,
        path: &Path,
        base: &FileSessionRecord,
        desired: &FileSessionRecord,
    ) -> Result<SessionRecordSaveResult> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("无法开始文件会话保存事务")?;
        let current = load_session_row(&transaction, path)?;
        let current_revision = current.as_ref().map_or(0, |current| current.revision);
        let conflict_resolved = current
            .as_ref()
            .is_some_and(|current| current.revision != base.revision);
        let mut candidate = match current {
            Some(current) if conflict_resolved => merge_session_changes(base, desired, current),
            Some(_) | None => desired.clone(),
        };
        candidate.revision = current_revision.saturating_add(1);
        save_session_row(&transaction, path, &candidate)?;
        transaction.commit().context("无法提交文件会话保存事务")?;
        Ok(SessionRecordSaveResult {
            record: candidate,
            conflict_resolved,
        })
    }

    pub fn save_sessions(&self, sessions: &[(PathBuf, FileSessionRecord)]) -> Result<()> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().context("无法开始窗口会话事务")?;
        for (path, state) in sessions {
            save_session_row(&transaction, path, state)?;
        }
        transaction.commit().context("无法提交窗口会话事务")
    }

    pub fn save_workspace(
        &self,
        sessions: &[(PathBuf, FileSessionRecord)],
        open_paths: &[PathBuf],
        active_path: Option<&Path>,
    ) -> Result<()> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().context("无法开始退出状态事务")?;
        for (path, state) in sessions {
            save_session_row(&transaction, path, state)?;
        }
        transaction
            .execute("DELETE FROM last_workspace_files", [])
            .context("无法重置上一次工作区")?;
        for (position, path) in open_paths.iter().enumerate() {
            let session_id = transaction
                .query_row(
                    "SELECT id FROM file_sessions WHERE path = ?1",
                    [encode_persisted_path(path)],
                    |row| row.get::<_, i64>(0),
                )
                .with_context(|| format!("无法定位工作区文件会话：{}", path.display()))?;
            transaction
                .execute(
                    "INSERT INTO last_workspace_files(position, file_session_id, was_active)
                     VALUES (?1, ?2, ?3)",
                    params![
                        i64::try_from(position).unwrap_or(i64::MAX),
                        session_id,
                        active_path.is_some_and(|active| active == path)
                    ],
                )
                .with_context(|| format!("无法保存工作区文件：{}", path.display()))?;
        }
        transaction.commit().context("无法提交退出状态事务")
    }

    pub fn database_info(&self) -> Result<DatabaseInfo> {
        let connection = self.lock()?;
        _ = connection.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
        let session_count = connection
            .query_row("SELECT COUNT(*) FROM file_sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .context("无法统计文件会话数量")?;
        drop(connection);
        let sidecar = |suffix: &str| {
            let mut path = self.database_path.as_os_str().to_os_string();
            path.push(suffix);
            PathBuf::from(path)
        };
        let byte_size = [self.database_path.clone(), sidecar("-wal"), sidecar("-shm")]
            .into_iter()
            .filter_map(|path| fs::metadata(path).ok().map(|metadata| metadata.len()))
            .fold(0_u64, u64::saturating_add);
        Ok(DatabaseInfo {
            byte_size,
            session_count: usize::try_from(session_count).unwrap_or(usize::MAX),
        })
    }

    pub fn delete_session_for_path(&self, path: &Path) -> Result<bool> {
        let connection = self.lock()?;
        connection
            .execute(
                "DELETE FROM file_sessions WHERE path = ?1",
                [encode_persisted_path(path)],
            )
            .with_context(|| format!("无法删除文件会话：{}", path.display()))
            .map(|removed| removed > 0)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("状态库锁已损坏"))
    }
}

fn load_session_row(connection: &Connection, path: &Path) -> Result<Option<FileSessionRecord>> {
    connection
        .query_row(
            "SELECT revision, custom_title, selected_row, query_text, case_sensitive, regex,
                    result_mode, marked_rows, show_line_numbers, show_row_separators,
                    keyword_color_rules, word_wrap, resume_state
             FROM file_sessions
             WHERE path = ?1",
            [encode_persisted_path(path)],
            |row| file_session_from_row(row, 0),
        )
        .optional()
        .with_context(|| format!("无法读取文件会话：{}", path.display()))
}

fn file_session_from_row(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<FileSessionRecord> {
    Ok(FileSessionRecord {
        revision: row.get(offset)?,
        custom_title: row.get(offset + 1)?,
        selected_row: row
            .get::<_, Option<i64>>(offset + 2)?
            .and_then(|row| usize::try_from(row).ok()),
        query_text: row.get(offset + 3)?,
        case_sensitive: row.get::<_, i64>(offset + 4)? != 0,
        regex: row.get::<_, i64>(offset + 5)? != 0,
        result_mode: row.get(offset + 6)?,
        marked_rows: row.get(offset + 7)?,
        show_line_numbers: row.get::<_, i64>(offset + 8)? != 0,
        show_row_separators: row.get::<_, i64>(offset + 9)? != 0,
        keyword_color_rules: row.get(offset + 10)?,
        word_wrap: row.get::<_, i64>(offset + 11)? != 0,
        resume_state: row.get(offset + 12)?,
    })
}

fn merge_session_changes(
    base: &FileSessionRecord,
    desired: &FileSessionRecord,
    mut latest: FileSessionRecord,
) -> FileSessionRecord {
    if base.custom_title != desired.custom_title {
        latest.custom_title.clone_from(&desired.custom_title);
    }
    if base.selected_row != desired.selected_row {
        latest.selected_row = desired.selected_row;
    }
    if base.query_text != desired.query_text {
        latest.query_text.clone_from(&desired.query_text);
    }
    if base.case_sensitive != desired.case_sensitive {
        latest.case_sensitive = desired.case_sensitive;
    }
    if base.regex != desired.regex {
        latest.regex = desired.regex;
    }
    if base.result_mode != desired.result_mode {
        latest.result_mode = desired.result_mode;
    }
    if base.marked_rows != desired.marked_rows {
        latest.marked_rows.clone_from(&desired.marked_rows);
    }
    if base.show_line_numbers != desired.show_line_numbers {
        latest.show_line_numbers = desired.show_line_numbers;
    }
    if base.show_row_separators != desired.show_row_separators {
        latest.show_row_separators = desired.show_row_separators;
    }
    if base.word_wrap != desired.word_wrap {
        latest.word_wrap = desired.word_wrap;
    }
    if base.keyword_color_rules != desired.keyword_color_rules {
        latest
            .keyword_color_rules
            .clone_from(&desired.keyword_color_rules);
    }
    if base.resume_state != desired.resume_state {
        latest.resume_state.clone_from(&desired.resume_state);
    }
    latest
}

fn save_session_row(connection: &Connection, path: &Path, state: &FileSessionRecord) -> Result<()> {
    let selected_row = state.selected_row.and_then(|row| i64::try_from(row).ok());
    connection
        .execute(
            "INSERT INTO file_sessions(
                 path, custom_title, last_opened_at, selected_row, query_text,
                 case_sensitive, regex, result_mode, marked_rows,
                 show_line_numbers, show_row_separators, keyword_color_rules, word_wrap,
                 resume_state
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(path) DO UPDATE SET
                 custom_title = excluded.custom_title,
                 selected_row = excluded.selected_row,
                 query_text = excluded.query_text,
                 case_sensitive = excluded.case_sensitive,
                 regex = excluded.regex,
                 result_mode = excluded.result_mode,
                 marked_rows = excluded.marked_rows,
                 show_line_numbers = excluded.show_line_numbers,
                 show_row_separators = excluded.show_row_separators,
                 keyword_color_rules = excluded.keyword_color_rules,
                 word_wrap = excluded.word_wrap,
                 resume_state = excluded.resume_state,
                 revision = file_sessions.revision + 1",
            params![
                encode_persisted_path(path),
                state.custom_title,
                unix_timestamp(),
                selected_row,
                state.query_text,
                state.case_sensitive,
                state.regex,
                state.result_mode,
                state.marked_rows,
                state.show_line_numbers,
                state.show_row_separators,
                state.keyword_color_rules,
                state.word_wrap,
                state.resume_state,
            ],
        )
        .with_context(|| format!("无法保存文件会话：{}", path.display()))?;
    Ok(())
}

fn encoded_path_set(paths: &[PathBuf]) -> HashSet<String> {
    paths
        .iter()
        .map(|path| encode_persisted_path(path))
        .collect()
}

fn count_marked_rows(stored: &str) -> usize {
    if let Some(encoded) = stored.strip_prefix(COMPRESSED_MARKED_ROWS_PREFIX) {
        return decode_hex(encoded)
            .and_then(|bytes| {
                let mut reader = Cursor::new(&bytes);
                let rows = RoaringTreemap::deserialize_from(&mut reader).ok()?;
                (reader.position() == u64::try_from(bytes.len()).ok()?
                    && rows.max().is_none_or(|row| usize::try_from(row).is_ok()))
                .then_some(usize::try_from(rows.len()).unwrap_or(usize::MAX))
            })
            .unwrap_or_default();
    }
    stored
        .split(',')
        .filter_map(|value| value.parse::<usize>().ok())
        .collect::<BTreeSet<_>>()
        .len()
}

fn decode_hex(encoded: &str) -> Option<Vec<u8>> {
    if !encoded.len().is_multiple_of(2) {
        return None;
    }
    let (pairs, remainder) = encoded.as_bytes().as_chunks::<2>();
    debug_assert!(remainder.is_empty());
    pairs
        .iter()
        .map(|digits| {
            let high = (digits[0] as char).to_digit(16)?;
            let low = (digits[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{FileSessionRecord, count_marked_rows, merge_session_changes};

    #[test]
    fn legacy_marked_rows_count_only_valid_rows() {
        assert_eq!(count_marked_rows("1,2,nope,2,9"), 3);
    }

    #[test]
    fn session_conflicts_merge_only_locally_changed_fields() {
        let base = FileSessionRecord {
            revision: 1,
            query_text: "old".into(),
            word_wrap: false,
            ..FileSessionRecord::default()
        };
        let desired = FileSessionRecord {
            revision: 1,
            query_text: "local".into(),
            word_wrap: false,
            ..FileSessionRecord::default()
        };
        let latest = FileSessionRecord {
            revision: 2,
            query_text: "old".into(),
            word_wrap: true,
            ..FileSessionRecord::default()
        };

        let merged = merge_session_changes(&base, &desired, latest);

        assert_eq!(merged.query_text, "local");
        assert!(merged.word_wrap);
    }
}
