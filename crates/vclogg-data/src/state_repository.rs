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
    AppSettingsRecord, CloudSettings, ColorLabelRecord, DatabaseInfo, FileSessionRecord,
    FileSessionRecords, HistorySession, LastWorkspaceFile, PredefinedFilterRecord, RecentFile,
    SessionRecordSaveResult, StateMigrationDefaults, decode_persisted_path, encode_persisted_path,
};

const COMPRESSED_MARKED_ROWS_PREFIX: &str = "rb1:";
pub const STATE_SCHEMA_VERSION: u32 = 6;

/// Owns SQLite access for durable file-history and workspace records.
pub struct StateRepository {
    connection: Mutex<Connection>,
    database_path: PathBuf,
}

impl StateRepository {
    /// Opens and idempotently migrates the state database.
    pub fn open(database_path: PathBuf, defaults: &StateMigrationDefaults) -> Result<Self> {
        let sessions_dir = database_path.parent().context("状态库路径没有父目录")?;
        fs::create_dir_all(sessions_dir)
            .with_context(|| format!("无法创建状态目录：{}", sessions_dir.display()))?;
        let connection = Connection::open(&database_path)
            .with_context(|| format!("无法打开状态库：{}", database_path.display()))?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .context("无法设置状态库忙等待")?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .context("无法启用状态库外键")?;
        initialize_schema(&connection, defaults)?;
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

    pub fn schema_version(&self) -> Result<u32> {
        self.lock()?
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .context("无法读取状态库版本")
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
                        result_mode, marked_rows,
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

    pub fn load_ui_value(&self, key: &str) -> Result<Option<String>> {
        let connection = self.lock()?;
        connection
            .query_row("SELECT value FROM ui_state WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .with_context(|| format!("无法读取界面状态：{key}"))
    }

    pub fn load_app_settings(&self) -> Result<Option<AppSettingsRecord>> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT default_show_line_numbers, default_show_row_separators,
                        highlight_log_levels, log_font_size, log_line_spacing, log_font_family,
                        shortcut_open_file, shortcut_focus_search, shortcut_quick_find,
                        shortcut_close_tab, shortcut_open_settings,
                        shortcut_toggle_case_sensitive, shortcut_jump_to_bottom,
                        shortcut_cycle_color_label, shortcut_toggle_word_wrap,
                        mouse_wheel_scroll_percent, scroll_by_line,
                        mouse_wheel_scroll_lines, scroll_by_line_when_word_wrap,
                        reduce_motion, confirm_close_tab, show_full_path,
                        max_search_results, highlight_matches, word_boundary_characters,
                        default_case_sensitive, default_use_regex,
                        show_line_number_row_separators, line_number_width,
                        line_number_text_color, line_number_background_color,
                        theme_preference, open_directory_command, viewer_overscan, language,
                        app_log_level, light_log_text_color, light_log_background_color,
                        dark_log_text_color, dark_log_background_color, log_level_color_rules
                 FROM app_settings WHERE id = 1",
                [],
                |row| {
                    Ok(AppSettingsRecord {
                        default_show_line_numbers: row.get::<_, i64>(0)? != 0,
                        default_show_row_separators: row.get::<_, i64>(1)? != 0,
                        highlight_log_levels: row.get::<_, i64>(2)? != 0,
                        log_font_size: row.get(3)?,
                        log_line_spacing: row.get(4)?,
                        log_font_family: row.get(5)?,
                        shortcut_open_file: row.get(6)?,
                        shortcut_focus_search: row.get(7)?,
                        shortcut_quick_find: row.get(8)?,
                        shortcut_close_tab: row.get(9)?,
                        shortcut_open_settings: row.get(10)?,
                        shortcut_toggle_case_sensitive: row.get(11)?,
                        shortcut_jump_to_bottom: row.get(12)?,
                        shortcut_cycle_color_label: row.get(13)?,
                        shortcut_toggle_word_wrap: row.get(14)?,
                        mouse_wheel_scroll_percent: row.get(15)?,
                        scroll_by_line: row.get::<_, i64>(16)? != 0,
                        mouse_wheel_scroll_lines: row.get(17)?,
                        scroll_by_line_when_word_wrap: row.get::<_, i64>(18)? != 0,
                        reduce_motion: row.get::<_, i64>(19)? != 0,
                        confirm_close_tab: row.get::<_, i64>(20)? != 0,
                        show_full_path: row.get::<_, i64>(21)? != 0,
                        max_search_results: row.get(22)?,
                        highlight_matches: row.get::<_, i64>(23)? != 0,
                        word_boundary_characters: row.get(24)?,
                        default_case_sensitive: row.get::<_, i64>(25)? != 0,
                        default_use_regex: row.get::<_, i64>(26)? != 0,
                        show_line_number_row_separators: row.get::<_, i64>(27)? != 0,
                        line_number_width: row.get(28)?,
                        line_number_text_color: row.get(29)?,
                        line_number_background_color: row.get(30)?,
                        theme_preference: row.get(31)?,
                        open_directory_command: row.get(32)?,
                        viewer_overscan: row.get(33)?,
                        language: row.get(34)?,
                        app_log_level: row.get(35)?,
                        light_log_text_color: row.get(36)?,
                        light_log_background_color: row.get(37)?,
                        dark_log_text_color: row.get(38)?,
                        dark_log_background_color: row.get(39)?,
                        log_level_color_rules: row.get(40)?,
                    })
                },
            )
            .optional()
            .context("无法读取应用设置")
    }

    pub fn save_app_settings(&self, settings: &AppSettingsRecord) -> Result<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO app_settings(
                     id, default_show_line_numbers, default_show_row_separators,
                     highlight_log_levels, log_font_size, log_line_spacing, log_font_family,
                     shortcut_open_file, shortcut_focus_search, shortcut_quick_find,
                     shortcut_close_tab, shortcut_open_settings,
                     shortcut_toggle_case_sensitive, shortcut_jump_to_bottom,
                     shortcut_cycle_color_label, shortcut_toggle_word_wrap,
                     mouse_wheel_scroll_percent, scroll_by_line,
                     mouse_wheel_scroll_lines, scroll_by_line_when_word_wrap,
                     reduce_motion, confirm_close_tab, show_full_path,
                     max_search_results, highlight_matches, word_boundary_characters,
                     default_case_sensitive, default_use_regex,
                     show_line_number_row_separators, line_number_width,
                     line_number_text_color, line_number_background_color,
                     theme_preference, open_directory_command, viewer_overscan, language,
                     app_log_level, light_log_text_color, light_log_background_color,
                     dark_log_text_color, dark_log_background_color, log_level_color_rules
                 ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37, ?38, ?39, ?40, ?41)
                 ON CONFLICT(id) DO UPDATE SET
                     default_show_line_numbers = excluded.default_show_line_numbers,
                     default_show_row_separators = excluded.default_show_row_separators,
                     highlight_log_levels = excluded.highlight_log_levels,
                     log_font_size = excluded.log_font_size,
                     log_line_spacing = excluded.log_line_spacing,
                     log_font_family = excluded.log_font_family,
                     shortcut_open_file = excluded.shortcut_open_file,
                     shortcut_focus_search = excluded.shortcut_focus_search,
                     shortcut_quick_find = excluded.shortcut_quick_find,
                     shortcut_close_tab = excluded.shortcut_close_tab,
                     shortcut_open_settings = excluded.shortcut_open_settings,
                     shortcut_toggle_case_sensitive = excluded.shortcut_toggle_case_sensitive,
                     shortcut_jump_to_bottom = excluded.shortcut_jump_to_bottom,
                     shortcut_cycle_color_label = excluded.shortcut_cycle_color_label,
                     shortcut_toggle_word_wrap = excluded.shortcut_toggle_word_wrap,
                     mouse_wheel_scroll_percent = excluded.mouse_wheel_scroll_percent,
                     scroll_by_line = excluded.scroll_by_line,
                     mouse_wheel_scroll_lines = excluded.mouse_wheel_scroll_lines,
                     scroll_by_line_when_word_wrap = excluded.scroll_by_line_when_word_wrap,
                     reduce_motion = excluded.reduce_motion,
                     confirm_close_tab = excluded.confirm_close_tab,
                     show_full_path = excluded.show_full_path,
                     max_search_results = excluded.max_search_results,
                     highlight_matches = excluded.highlight_matches,
                     word_boundary_characters = excluded.word_boundary_characters,
                     default_case_sensitive = excluded.default_case_sensitive,
                     default_use_regex = excluded.default_use_regex,
                     show_line_number_row_separators = excluded.show_line_number_row_separators,
                     line_number_width = excluded.line_number_width,
                     line_number_text_color = excluded.line_number_text_color,
                     line_number_background_color = excluded.line_number_background_color,
                     theme_preference = excluded.theme_preference,
                     open_directory_command = excluded.open_directory_command,
                     viewer_overscan = excluded.viewer_overscan,
                     language = excluded.language,
                     app_log_level = excluded.app_log_level,
                     light_log_text_color = excluded.light_log_text_color,
                     light_log_background_color = excluded.light_log_background_color,
                     dark_log_text_color = excluded.dark_log_text_color,
                     dark_log_background_color = excluded.dark_log_background_color,
                     log_level_color_rules = excluded.log_level_color_rules",
                params![
                    settings.default_show_line_numbers,
                    settings.default_show_row_separators,
                    settings.highlight_log_levels,
                    settings.log_font_size,
                    settings.log_line_spacing,
                    settings.log_font_family,
                    settings.shortcut_open_file,
                    settings.shortcut_focus_search,
                    settings.shortcut_quick_find,
                    settings.shortcut_close_tab,
                    settings.shortcut_open_settings,
                    settings.shortcut_toggle_case_sensitive,
                    settings.shortcut_jump_to_bottom,
                    settings.shortcut_cycle_color_label,
                    settings.shortcut_toggle_word_wrap,
                    settings.mouse_wheel_scroll_percent,
                    settings.scroll_by_line,
                    settings.mouse_wheel_scroll_lines,
                    settings.scroll_by_line_when_word_wrap,
                    settings.reduce_motion,
                    settings.confirm_close_tab,
                    settings.show_full_path,
                    settings.max_search_results,
                    settings.highlight_matches,
                    settings.word_boundary_characters,
                    settings.default_case_sensitive,
                    settings.default_use_regex,
                    settings.show_line_number_row_separators,
                    settings.line_number_width,
                    settings.line_number_text_color,
                    settings.line_number_background_color,
                    settings.theme_preference,
                    settings.open_directory_command,
                    settings.viewer_overscan,
                    settings.language,
                    settings.app_log_level,
                    settings.light_log_text_color,
                    settings.light_log_background_color,
                    settings.dark_log_text_color,
                    settings.dark_log_background_color,
                    settings.log_level_color_rules,
                ],
            )
            .context("无法保存应用设置")?;
        Ok(())
    }

    pub fn save_ui_value(&self, key: &str, value: &str) -> Result<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO ui_state(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .with_context(|| format!("无法保存界面状态：{key}"))?;
        Ok(())
    }

    pub fn global_search_preferences(&self) -> Result<BTreeMap<PathBuf, bool>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT path, selected FROM global_search_preferences ORDER BY path")
            .context("无法读取全局搜索参与偏好")?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    decode_persisted_path(&row.get::<_, String>(0)?),
                    row.get::<_, i64>(1)? != 0,
                ))
            })
            .context("无法查询全局搜索参与偏好")?;
        rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()
            .context("无法解析全局搜索参与偏好")
    }

    pub fn save_global_search_preferences(&self, preferences: &[(PathBuf, bool)]) -> Result<()> {
        if preferences.is_empty() {
            return Ok(());
        }
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .context("无法开始全局搜索参与偏好事务")?;
        for (path, selected) in preferences {
            transaction
                .execute(
                    "INSERT INTO global_search_preferences(path, selected)
                     VALUES (?1, ?2)
                     ON CONFLICT(path) DO UPDATE SET selected = excluded.selected",
                    params![encode_persisted_path(path), selected],
                )
                .with_context(|| format!("无法保存全局搜索参与偏好：{}", path.display()))?;
        }
        transaction.commit().context("无法提交全局搜索参与偏好事务")
    }

    pub fn load_search_history(&self) -> Result<Vec<String>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT query_text FROM search_history ORDER BY position ASC")
            .context("无法读取搜索历史")?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .context("无法查询搜索历史")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("无法解析搜索历史")
    }

    pub fn save_search_history(&self, history: &[String]) -> Result<()> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().context("无法开始搜索历史事务")?;
        transaction
            .execute("DELETE FROM search_history", [])
            .context("无法重置搜索历史")?;
        for (position, query) in history.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO search_history(position, query_text) VALUES (?1, ?2)",
                    params![i64::try_from(position).unwrap_or(i64::MAX), query],
                )
                .context("无法写入搜索历史")?;
        }
        transaction.commit().context("无法提交搜索历史事务")
    }

    pub fn load_predefined_filters(&self) -> Result<Vec<PredefinedFilterRecord>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT filter_id, item_json FROM predefined_filters ORDER BY position ASC")
            .context("无法读取预定义过滤器")?;
        let rows = statement
            .query_map([], |row| {
                Ok(PredefinedFilterRecord {
                    id: row.get(0)?,
                    json: row.get(1)?,
                })
            })
            .context("无法查询预定义过滤器")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("无法解析预定义过滤器记录")
    }

    pub fn save_predefined_filters(&self, filters: &[PredefinedFilterRecord]) -> Result<()> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .context("无法开始预定义过滤器事务")?;
        transaction
            .execute("DELETE FROM predefined_filters", [])
            .context("无法重置预定义过滤器")?;
        for (position, filter) in filters.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO predefined_filters(position, filter_id, item_json)
                     VALUES (?1, ?2, ?3)",
                    params![
                        i64::try_from(position).unwrap_or(i64::MAX),
                        filter.id,
                        filter.json,
                    ],
                )
                .with_context(|| format!("无法保存预定义过滤器：{}", filter.id))?;
        }
        transaction.commit().context("无法提交预定义过滤器事务")
    }

    pub fn load_cloud_settings(&self) -> Result<CloudSettings> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT server_url, display_name FROM cloud_settings WHERE id = 1",
                [],
                |row| {
                    Ok(CloudSettings {
                        server_url: row.get(0)?,
                        display_name: row.get(1)?,
                    })
                },
            )
            .optional()
            .context("无法读取云端过滤器设置")
            .map(|settings| settings.unwrap_or_default())
    }

    pub fn save_cloud_settings(&self, settings: &CloudSettings) -> Result<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO cloud_settings(id, server_url, display_name)
                 VALUES (1, ?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET
                     server_url = excluded.server_url,
                     display_name = excluded.display_name",
                params![settings.server_url.trim(), settings.display_name.trim()],
            )
            .context("无法保存云端过滤器设置")?;
        Ok(())
    }

    pub fn load_color_labels(&self) -> Result<Option<Vec<ColorLabelRecord>>> {
        let connection = self.lock()?;
        let initialized = connection
            .query_row(
                "SELECT initialized FROM color_label_settings WHERE id = 1",
                [],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .context("无法读取颜色标签初始化状态")?
            .unwrap_or(false);
        if !initialized {
            return Ok(None);
        }
        let mut statement = connection
            .prepare(
                "SELECT label_id, name, text_color, text_alpha, color, alpha
                 FROM color_labels ORDER BY position ASC",
            )
            .context("无法读取颜色标签")?;
        let rows = statement
            .query_map([], |row| {
                Ok(ColorLabelRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    text_color: row.get(2)?,
                    text_alpha: u8::try_from(row.get::<_, i64>(3)?).unwrap_or(u8::MAX),
                    background_color: row.get(4)?,
                    background_alpha: u8::try_from(row.get::<_, i64>(5)?).unwrap_or(u8::MAX),
                })
            })
            .context("无法查询颜色标签")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("无法解析颜色标签")
            .map(Some)
    }

    pub fn save_color_labels(&self, labels: &[ColorLabelRecord]) -> Result<()> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().context("无法开始颜色标签事务")?;
        transaction
            .execute("DELETE FROM color_labels", [])
            .context("无法重置颜色标签")?;
        for (position, label) in labels.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO color_labels(
                         position, label_id, name, text_color, text_alpha, color, alpha
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        i64::try_from(position).unwrap_or(i64::MAX),
                        label.id,
                        label.name,
                        label.text_color,
                        label.text_alpha,
                        label.background_color,
                        label.background_alpha,
                    ],
                )
                .with_context(|| format!("无法保存颜色标签：{}", label.name))?;
        }
        transaction
            .execute(
                "INSERT INTO color_label_settings(id, initialized) VALUES (1, 1)
                 ON CONFLICT(id) DO UPDATE SET initialized = 1",
                [],
            )
            .context("无法保存颜色标签初始化状态")?;
        transaction.commit().context("无法提交颜色标签事务")
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

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("状态库锁已损坏"))
    }
}

fn load_session_row(connection: &Connection, path: &Path) -> Result<Option<FileSessionRecord>> {
    connection
        .query_row(
            "SELECT revision, custom_title, selected_row, query_text, result_mode,
                    marked_rows, show_line_numbers, show_row_separators,
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
        result_mode: row.get(offset + 4)?,
        marked_rows: row.get(offset + 5)?,
        show_line_numbers: row.get::<_, i64>(offset + 6)? != 0,
        show_row_separators: row.get::<_, i64>(offset + 7)? != 0,
        keyword_color_rules: row.get(offset + 8)?,
        word_wrap: row.get::<_, i64>(offset + 9)? != 0,
        resume_state: row.get(offset + 10)?,
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
                 result_mode, marked_rows,
                 show_line_numbers, show_row_separators, keyword_color_rules, word_wrap,
                 resume_state
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(path) DO UPDATE SET
                 custom_title = excluded.custom_title,
                 selected_row = excluded.selected_row,
                 query_text = excluded.query_text,
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

fn initialize_schema(connection: &Connection, defaults: &StateMigrationDefaults) -> Result<()> {
    let schema_version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
        .context("无法读取状态库版本")?;
    if schema_version > STATE_SCHEMA_VERSION {
        anyhow::bail!("状态库版本 {schema_version} 高于当前支持的 {STATE_SCHEMA_VERSION}");
    }
    if schema_version == STATE_SCHEMA_VERSION {
        return Ok(());
    }
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS file_sessions (
                 id INTEGER PRIMARY KEY,
                 path TEXT NOT NULL UNIQUE,
                 custom_title TEXT,
                 last_opened_at INTEGER NOT NULL,
                 revision INTEGER NOT NULL DEFAULT 1,
                 selected_row INTEGER,
                 query_text TEXT NOT NULL DEFAULT '',
                 case_sensitive INTEGER NOT NULL DEFAULT 0,
                 regex INTEGER NOT NULL DEFAULT 0,
                 result_mode INTEGER NOT NULL DEFAULT 0,
                 marked_rows TEXT NOT NULL DEFAULT '',
                 show_line_numbers INTEGER NOT NULL DEFAULT 1,
                 show_row_separators INTEGER NOT NULL DEFAULT 0,
                 word_wrap INTEGER NOT NULL DEFAULT 0,
                 keyword_color_rules TEXT NOT NULL DEFAULT '[]',
                 resume_state TEXT NOT NULL DEFAULT '',
                 pinned INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_file_sessions_recent
                 ON file_sessions(last_opened_at DESC);
             CREATE TABLE IF NOT EXISTS last_workspace_files (
                 position INTEGER PRIMARY KEY,
                 file_session_id INTEGER NOT NULL UNIQUE
                     REFERENCES file_sessions(id) ON DELETE CASCADE,
                 was_active INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS app_settings (
                 id INTEGER PRIMARY KEY CHECK(id = 1),
                 default_show_line_numbers INTEGER NOT NULL DEFAULT 1,
                 default_show_row_separators INTEGER NOT NULL DEFAULT 0,
                 highlight_log_levels INTEGER NOT NULL DEFAULT 0,
                 log_level_color_rules TEXT NOT NULL DEFAULT '',
                 log_font_size INTEGER NOT NULL DEFAULT 13,
                 log_line_spacing INTEGER NOT NULL DEFAULT 6,
                 log_font_family TEXT NOT NULL DEFAULT 'consolas',
                 shortcut_open_file TEXT NOT NULL DEFAULT 'Ctrl+O',
                 shortcut_focus_search TEXT NOT NULL DEFAULT 'Ctrl+F',
                 shortcut_quick_find TEXT NOT NULL DEFAULT 'Ctrl+Shift+F',
                 shortcut_close_tab TEXT NOT NULL DEFAULT 'Ctrl+W',
                 shortcut_open_settings TEXT NOT NULL DEFAULT 'Ctrl+,',
                 shortcut_toggle_case_sensitive TEXT NOT NULL DEFAULT 'Alt+C',
                 shortcut_jump_to_bottom TEXT NOT NULL DEFAULT 'Ctrl+End',
                 shortcut_cycle_color_label TEXT NOT NULL DEFAULT 'Ctrl+D',
                 shortcut_toggle_word_wrap TEXT NOT NULL DEFAULT 'W',
                 mouse_wheel_scroll_percent INTEGER NOT NULL DEFAULT 100,
                 scroll_by_line INTEGER NOT NULL DEFAULT 0,
                 mouse_wheel_scroll_lines INTEGER NOT NULL DEFAULT 1,
                 scroll_by_line_when_word_wrap INTEGER NOT NULL DEFAULT 0,
                 reduce_motion INTEGER NOT NULL DEFAULT 0,
                 confirm_close_tab INTEGER NOT NULL DEFAULT 0,
                 show_full_path INTEGER NOT NULL DEFAULT 1,
                 max_search_results INTEGER NOT NULL DEFAULT 0,
                 highlight_matches INTEGER NOT NULL DEFAULT 1,
                 word_boundary_characters TEXT NOT NULL DEFAULT '.,;:!?()[]{}<>/\\|\"''`~@#$%^&*+-=，。！？；：、（）【】《》“”‘’…—',
                 default_case_sensitive INTEGER NOT NULL DEFAULT 0,
                 default_use_regex INTEGER NOT NULL DEFAULT 0,
                 show_line_number_row_separators INTEGER NOT NULL DEFAULT 0,
                 line_number_width INTEGER NOT NULL DEFAULT 60,
                 line_number_text_color TEXT,
                 line_number_background_color TEXT,
                 theme_preference TEXT NOT NULL DEFAULT 'light',
                 open_directory_command TEXT NOT NULL DEFAULT '',
                 viewer_overscan INTEGER NOT NULL DEFAULT 12,
                 language TEXT NOT NULL DEFAULT 'zh-CN',
                 app_log_level TEXT NOT NULL DEFAULT 'error',
                 light_log_text_color TEXT,
                 light_log_background_color TEXT,
                 dark_log_text_color TEXT,
                 dark_log_background_color TEXT
             );
             CREATE TABLE IF NOT EXISTS ui_state (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS global_search_preferences (
                 path TEXT PRIMARY KEY,
                 selected INTEGER NOT NULL DEFAULT 1
             );
             CREATE TABLE IF NOT EXISTS search_history (
                 position INTEGER PRIMARY KEY,
                 query_text TEXT NOT NULL UNIQUE
             );
             CREATE TABLE IF NOT EXISTS color_labels (
                 position INTEGER PRIMARY KEY,
                 label_id TEXT NOT NULL UNIQUE,
                 name TEXT NOT NULL,
                 color INTEGER NOT NULL,
                 alpha INTEGER NOT NULL DEFAULT 255,
                 text_color INTEGER NOT NULL DEFAULT 1315860,
                 text_alpha INTEGER NOT NULL DEFAULT 255
             );
             CREATE TABLE IF NOT EXISTS color_label_settings (
                 id INTEGER PRIMARY KEY CHECK(id = 1),
                 initialized INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS predefined_filters (
                 position INTEGER PRIMARY KEY,
                 filter_id TEXT NOT NULL UNIQUE,
                 item_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS cloud_settings (
                 id INTEGER PRIMARY KEY CHECK(id = 1),
                 server_url TEXT NOT NULL DEFAULT '',
                 display_name TEXT NOT NULL DEFAULT ''
             );",
        )
        .context("无法初始化状态库结构")?;
    ensure_session_columns(connection)?;
    ensure_app_settings_columns(connection, &defaults.app_log_level)?;
    ensure_color_label_columns(connection, &defaults.color_labels)?;
    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_file_sessions_pinned_recent
             ON file_sessions(pinned DESC, last_opened_at DESC)",
            [],
        )
        .context("无法创建收藏文件索引")?;
    connection
        .pragma_update(None, "user_version", STATE_SCHEMA_VERSION)
        .context("无法更新状态库版本")?;
    Ok(())
}

fn ensure_session_columns(connection: &Connection) -> Result<()> {
    // `case_sensitive` and `regex` are retained only so older databases and binaries keep a
    // compatible table shape. Current code stores the canonical pair once in `app_settings`.
    const COLUMNS: [(&str, &str); 13] = [
        ("custom_title", "TEXT"),
        ("selected_row", "INTEGER"),
        ("query_text", "TEXT NOT NULL DEFAULT ''"),
        ("case_sensitive", "INTEGER NOT NULL DEFAULT 0"),
        ("regex", "INTEGER NOT NULL DEFAULT 0"),
        ("result_mode", "INTEGER NOT NULL DEFAULT 0"),
        ("marked_rows", "TEXT NOT NULL DEFAULT ''"),
        ("show_line_numbers", "INTEGER NOT NULL DEFAULT 1"),
        ("show_row_separators", "INTEGER NOT NULL DEFAULT 0"),
        ("keyword_color_rules", "TEXT NOT NULL DEFAULT '[]'"),
        ("word_wrap", "INTEGER NOT NULL DEFAULT 0"),
        ("resume_state", "TEXT NOT NULL DEFAULT ''"),
        ("pinned", "INTEGER NOT NULL DEFAULT 0"),
    ];
    ensure_columns(connection, "file_sessions", &COLUMNS)
}

fn ensure_color_label_columns(
    connection: &Connection,
    default_labels: &[ColorLabelRecord],
) -> Result<()> {
    let columns = table_columns(connection, "color_labels")?;
    if !columns.contains("alpha") {
        connection
            .execute(
                "ALTER TABLE color_labels ADD COLUMN alpha INTEGER NOT NULL DEFAULT 255",
                [],
            )
            .context("无法迁移颜色标签透明度字段")?;
        for label in default_labels {
            connection
                .execute(
                    "UPDATE color_labels SET alpha = ?1 WHERE label_id = ?2 AND color = ?3",
                    params![label.background_alpha, label.id, label.background_color],
                )
                .with_context(|| format!("无法迁移颜色标签透明度：{}", label.name))?;
        }
    }
    ensure_columns(
        connection,
        "color_labels",
        &[
            ("text_color", "INTEGER NOT NULL DEFAULT 1315860"),
            ("text_alpha", "INTEGER NOT NULL DEFAULT 255"),
        ],
    )?;
    Ok(())
}

fn ensure_app_settings_columns(connection: &Connection, default_log_level: &str) -> Result<()> {
    const COLUMNS: [(&str, &str); 38] = [
        ("highlight_log_levels", "INTEGER NOT NULL DEFAULT 0"),
        ("log_level_color_rules", "TEXT NOT NULL DEFAULT ''"),
        ("log_font_size", "INTEGER NOT NULL DEFAULT 13"),
        ("log_line_spacing", "INTEGER NOT NULL DEFAULT 6"),
        ("log_font_family", "TEXT NOT NULL DEFAULT 'consolas'"),
        ("shortcut_open_file", "TEXT NOT NULL DEFAULT 'Ctrl+O'"),
        ("shortcut_focus_search", "TEXT NOT NULL DEFAULT 'Ctrl+F'"),
        (
            "shortcut_quick_find",
            "TEXT NOT NULL DEFAULT 'Ctrl+Shift+F'",
        ),
        ("shortcut_close_tab", "TEXT NOT NULL DEFAULT 'Ctrl+W'"),
        ("shortcut_open_settings", "TEXT NOT NULL DEFAULT 'Ctrl+,'"),
        (
            "shortcut_toggle_case_sensitive",
            "TEXT NOT NULL DEFAULT 'Alt+C'",
        ),
        (
            "shortcut_jump_to_bottom",
            "TEXT NOT NULL DEFAULT 'Ctrl+End'",
        ),
        (
            "shortcut_cycle_color_label",
            "TEXT NOT NULL DEFAULT 'Ctrl+D'",
        ),
        ("shortcut_toggle_word_wrap", "TEXT NOT NULL DEFAULT 'W'"),
        ("mouse_wheel_scroll_percent", "INTEGER NOT NULL DEFAULT 100"),
        ("scroll_by_line", "INTEGER NOT NULL DEFAULT 0"),
        ("mouse_wheel_scroll_lines", "INTEGER NOT NULL DEFAULT 1"),
        (
            "scroll_by_line_when_word_wrap",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("reduce_motion", "INTEGER NOT NULL DEFAULT 0"),
        ("confirm_close_tab", "INTEGER NOT NULL DEFAULT 0"),
        ("show_full_path", "INTEGER NOT NULL DEFAULT 1"),
        ("max_search_results", "INTEGER NOT NULL DEFAULT 0"),
        ("highlight_matches", "INTEGER NOT NULL DEFAULT 1"),
        (
            "word_boundary_characters",
            r#"TEXT NOT NULL DEFAULT '.,;:!?()[]{}<>/\|"''`~@#$%^&*+-=，。！？；：、（）【】《》“”‘’…—'"#,
        ),
        ("default_case_sensitive", "INTEGER NOT NULL DEFAULT 0"),
        ("default_use_regex", "INTEGER NOT NULL DEFAULT 0"),
        (
            "show_line_number_row_separators",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("line_number_width", "INTEGER NOT NULL DEFAULT 60"),
        ("line_number_text_color", "TEXT"),
        ("line_number_background_color", "TEXT"),
        ("theme_preference", "TEXT NOT NULL DEFAULT 'light'"),
        ("open_directory_command", "TEXT NOT NULL DEFAULT ''"),
        ("viewer_overscan", "INTEGER NOT NULL DEFAULT 12"),
        ("language", "TEXT NOT NULL DEFAULT 'zh-CN'"),
        ("light_log_text_color", "TEXT"),
        ("light_log_background_color", "TEXT"),
        ("dark_log_text_color", "TEXT"),
        ("dark_log_background_color", "TEXT"),
    ];
    ensure_columns(connection, "app_settings", &COLUMNS)?;
    let existing = table_columns(connection, "app_settings")?;
    if !existing.contains("app_log_level") {
        connection
            .execute(
                "ALTER TABLE app_settings ADD COLUMN app_log_level TEXT NOT NULL DEFAULT 'error'",
                [],
            )
            .context("无法迁移应用日志等级字段")?;
        connection
            .execute(
                "UPDATE app_settings SET app_log_level = ?1",
                [default_log_level],
            )
            .context("无法初始化应用日志等级")?;
    }
    Ok(())
}

fn ensure_columns(connection: &Connection, table: &str, columns: &[(&str, &str)]) -> Result<()> {
    let existing = table_columns(connection, table)?;
    for (name, declaration) in columns {
        if !existing.contains(*name) {
            connection
                .execute(
                    &format!("ALTER TABLE {table} ADD COLUMN {name} {declaration}"),
                    [],
                )
                .with_context(|| format!("无法迁移状态库字段：{name}"))?;
        }
    }
    Ok(())
}

fn table_columns(connection: &Connection, table: &str) -> Result<HashSet<String>> {
    let mut statement = connection
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .with_context(|| format!("无法准备状态库字段检查：{table}"))?;
    let columns = statement
        .query_map([table], |row| row.get::<_, String>(0))
        .with_context(|| format!("无法查询状态库字段：{table}"))?;
    columns
        .collect::<rusqlite::Result<HashSet<_>>>()
        .with_context(|| format!("无法解析状态库字段：{table}"))
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{
        FileSessionRecord, StateMigrationDefaults, count_marked_rows, initialize_schema,
        merge_session_changes, table_columns,
    };

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

    #[test]
    fn version_four_database_adds_theme_and_log_coloring_fields() {
        let connection = Connection::open_in_memory().expect("应能创建内存数据库");
        connection
            .execute_batch(
                "CREATE TABLE app_settings (id INTEGER PRIMARY KEY CHECK(id = 1));
                 PRAGMA user_version = 4;",
            )
            .expect("应能创建旧版应用设置表");

        initialize_schema(
            &connection,
            &StateMigrationDefaults {
                app_log_level: "error".into(),
                color_labels: Vec::new(),
            },
        )
        .expect("应能迁移旧版数据库");

        let columns = table_columns(&connection, "app_settings").expect("应能读取迁移后的字段");
        for column in [
            "light_log_text_color",
            "light_log_background_color",
            "dark_log_text_color",
            "dark_log_background_color",
            "log_level_color_rules",
        ] {
            assert!(columns.contains(column), "缺少迁移字段 {column}");
        }
        let label_columns =
            table_columns(&connection, "color_labels").expect("应能读取迁移后的颜色标签字段");
        for column in ["text_color", "text_alpha", "color", "alpha"] {
            assert!(label_columns.contains(column), "缺少颜色标签字段 {column}");
        }
    }
}
