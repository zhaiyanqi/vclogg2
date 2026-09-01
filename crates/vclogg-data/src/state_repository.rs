//! SQLite repository for file history and workspace recovery records.

use std::{
    collections::{BTreeSet, HashSet},
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result};
use roaring::RoaringTreemap;
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::{
    DatabaseInfo, HistorySession, LastWorkspaceFile, RecentFile, decode_persisted_path,
    encode_persisted_path,
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
    use super::count_marked_rows;

    #[test]
    fn legacy_marked_rows_count_only_valid_rows() {
        assert_eq!(count_marked_rows("1,2,nope,2,9"), 3);
    }
}
