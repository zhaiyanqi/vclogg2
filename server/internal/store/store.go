package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"modernc.org/sqlite"
)

const schemaVersion = 6

type Store struct {
	DB      *sql.DB
	Path    string
	DataDir string
}

func Open(ctx context.Context, dataDir string) (*Store, error) {
	if err := os.MkdirAll(dataDir, 0o750); err != nil {
		return nil, err
	}
	databasePath := filepath.Join(dataDir, "vclogg-server.db")
	db, err := sql.Open("sqlite", databasePath)
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
	db.SetMaxIdleConns(1)
	s := &Store{DB: db, Path: databasePath, DataDir: dataDir}
	if _, err := db.ExecContext(ctx, `
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA foreign_keys=ON;
PRAGMA busy_timeout=5000;
`); err != nil {
		db.Close()
		return nil, err
	}
	if err := s.migrate(ctx); err != nil {
		db.Close()
		return nil, err
	}
	return s, nil
}

func (s *Store) Close() error { return s.DB.Close() }

func (s *Store) migrate(ctx context.Context) error {
	if _, err := s.DB.ExecContext(ctx, `CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL)`); err != nil {
		return err
	}
	var current int
	if err := s.DB.QueryRowContext(ctx, `SELECT COALESCE(MAX(version), 0) FROM schema_migrations`).Scan(&current); err != nil {
		return err
	}
	if current >= schemaVersion {
		return nil
	}
	if current > 0 {
		if _, err := s.Backup(ctx, filepath.Join(s.DataDir, "backups"), "pre-migration"); err != nil {
			return fmt.Errorf("backup before migration: %w", err)
		}
	}
	tx, err := s.DB.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err := tx.ExecContext(ctx, schemaSQL); err != nil {
		return err
	}
	if current > 0 && current < 4 {
		if _, err := tx.ExecContext(ctx, migrationV4SQL); err != nil {
			return fmt.Errorf("migrate filter revision history to v4: %w", err)
		}
	}
	if current > 0 && current < 5 {
		hasAdminStatus, err := columnExists(tx, "admins", "status")
		if err != nil {
			return fmt.Errorf("inspect administrator schema: %w", err)
		}
		if !hasAdminStatus {
			if _, err := tx.ExecContext(ctx, `ALTER TABLE admins ADD COLUMN status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','disabled'))`); err != nil {
				return fmt.Errorf("add administrator status: %w", err)
			}
		}
		if _, err := tx.ExecContext(ctx, migrationV5SQL); err != nil {
			return fmt.Errorf("migrate administrator RBAC to v5: %w", err)
		}
	}
	if _, err := tx.ExecContext(ctx, `INSERT INTO schema_migrations(version, applied_at) VALUES(?, ?)`, schemaVersion, time.Now().UnixMilli()); err != nil {
		return err
	}
	return tx.Commit()
}

func columnExists(tx *sql.Tx, table, column string) (bool, error) {
	rows, err := tx.Query(`PRAGMA table_info(` + table + `)`)
	if err != nil {
		return false, err
	}
	defer rows.Close()
	for rows.Next() {
		var cid, notNull, primaryKey int
		var name, dataType string
		var defaultValue any
		if err := rows.Scan(&cid, &name, &dataType, &notNull, &defaultValue, &primaryKey); err != nil {
			return false, err
		}
		if name == column {
			return true, nil
		}
	}
	return false, rows.Err()
}

func (s *Store) Backup(ctx context.Context, directory, prefix string) (string, error) {
	if directory == "" {
		directory = filepath.Join(s.DataDir, "backups")
	}
	if err := os.MkdirAll(directory, 0o750); err != nil {
		return "", err
	}
	if prefix == "" {
		prefix = "vclogg"
	}
	name := fmt.Sprintf("%s-%s.db", prefix, time.Now().UTC().Format("20060102-150405.000"))
	target := filepath.Join(directory, name)
	connection, err := s.DB.Conn(ctx)
	if err != nil {
		return "", err
	}
	defer connection.Close()
	type onlineBackuper interface {
		NewBackup(string) (*sqlite.Backup, error)
	}
	err = connection.Raw(func(driverConnection any) error {
		backuper, ok := driverConnection.(onlineBackuper)
		if !ok {
			return errors.New("sqlite driver does not support the online backup API")
		}
		backup, err := backuper.NewBackup(target)
		if err != nil {
			return err
		}
		var stepErr error
		for more := true; more; {
			if err := ctx.Err(); err != nil {
				stepErr = err
				break
			}
			more, stepErr = backup.Step(256)
			if stepErr != nil {
				break
			}
		}
		return errors.Join(stepErr, backup.Finish())
	})
	if err != nil {
		_ = os.Remove(target)
		return "", err
	}
	return target, nil
}

func Restore(source, dataDir string) error {
	input, err := os.Open(source)
	if err != nil {
		return err
	}
	defer input.Close()
	if err := os.MkdirAll(dataDir, 0o750); err != nil {
		return err
	}
	target := filepath.Join(dataDir, "vclogg-server.db")
	if _, err := os.Stat(target + "-wal"); err == nil {
		return errors.New("database appears to be running; stop the server before restore")
	}
	temporary := target + ".restore.tmp"
	output, err := os.OpenFile(temporary, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0o640)
	if err != nil {
		return err
	}
	if _, err = output.ReadFrom(input); err != nil {
		output.Close()
		os.Remove(temporary)
		return err
	}
	if err := output.Close(); err != nil {
		os.Remove(temporary)
		return err
	}
	if err := validateRestoreCandidate(temporary); err != nil {
		_ = os.Remove(temporary)
		return err
	}
	previous := target + ".before-restore"
	_ = os.Remove(previous)
	if _, err := os.Stat(target); err == nil {
		if err := os.Rename(target, previous); err != nil {
			os.Remove(temporary)
			return err
		}
	}
	if err := os.Rename(temporary, target); err != nil {
		_ = os.Rename(previous, target)
		return err
	}
	_ = os.Remove(previous)
	return nil
}

func validateRestoreCandidate(path string) error {
	database, err := sql.Open("sqlite", path)
	if err != nil {
		return fmt.Errorf("open backup: %w", err)
	}
	database.SetMaxOpenConns(1)
	defer database.Close()
	var integrity string
	if err := database.QueryRow(`PRAGMA integrity_check`).Scan(&integrity); err != nil {
		return fmt.Errorf("check backup integrity: %w", err)
	}
	if integrity != "ok" {
		return fmt.Errorf("backup integrity check failed: %s", integrity)
	}
	var version int
	if err := database.QueryRow(`SELECT COALESCE(MAX(version), 0) FROM schema_migrations`).Scan(&version); err != nil {
		return fmt.Errorf("backup does not contain a VCLogg schema: %w", err)
	}
	if version > schemaVersion {
		return fmt.Errorf("backup schema version %d is newer than supported version %d", version, schemaVersion)
	}
	return nil
}

const schemaSQL = `
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
INSERT OR IGNORE INTO settings(key, value, updated_at) VALUES('latest_app_version', '0.0.0', 0);

CREATE TABLE IF NOT EXISTS admins (
  id TEXT PRIMARY KEY,
  username TEXT NOT NULL UNIQUE COLLATE NOCASE,
  password_hash TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','disabled')),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS admin_sessions (
  token_hash TEXT PRIMARY KEY,
  admin_id TEXT NOT NULL REFERENCES admins(id) ON DELETE CASCADE,
  csrf_token TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS admin_roles (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE COLLATE NOCASE,
  description TEXT NOT NULL DEFAULT '',
  built_in INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS admin_permissions (
  code TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  group_name TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS admin_role_members (
  admin_id TEXT NOT NULL REFERENCES admins(id) ON DELETE CASCADE,
  role_id TEXT NOT NULL REFERENCES admin_roles(id) ON DELETE RESTRICT,
  created_at INTEGER NOT NULL,
  PRIMARY KEY(admin_id, role_id)
);
CREATE INDEX IF NOT EXISTS idx_admin_role_members_role ON admin_role_members(role_id);
CREATE TABLE IF NOT EXISTS admin_role_permissions (
  role_id TEXT NOT NULL REFERENCES admin_roles(id) ON DELETE CASCADE,
  permission_code TEXT NOT NULL REFERENCES admin_permissions(code) ON DELETE CASCADE,
  PRIMARY KEY(role_id, permission_code)
);
CREATE TABLE IF NOT EXISTS admin_audit_logs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  admin_id TEXT,
  username TEXT NOT NULL DEFAULT '',
  action TEXT NOT NULL,
  resource_type TEXT NOT NULL DEFAULT '',
  resource_id TEXT NOT NULL DEFAULT '',
  result TEXT NOT NULL CHECK(result IN ('success','failure','denied')),
  client_address TEXT NOT NULL DEFAULT '',
  metadata TEXT NOT NULL DEFAULT '{}',
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_admin_audit_logs_created ON admin_audit_logs(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_admin_audit_logs_admin ON admin_audit_logs(admin_id, created_at DESC);

INSERT OR IGNORE INTO admin_roles(id,name,description,built_in,created_at,updated_at)
VALUES('super_admin','超级管理员','拥有全部管理权限',1,0,0);

INSERT OR IGNORE INTO admin_permissions(code,name,group_name,description) VALUES
('dashboard.read','查看仪表盘','仪表盘','查看管理概览与趋势'),
('filters.read','查看关键词','关键词','查看云端关键词'),
('filters.update','编辑关键词','关键词','修改关键词内容'),
('filters.status','变更关键词状态','关键词','停用、恢复或删除关键词'),
('filters.export','导出关键词','关键词','按筛选条件导出云端关键词'),
('identities.read','查看访问身份','访问身份','查看客户端身份'),
('identities.revoke','撤销访问身份','访问身份','撤销或恢复客户端身份'),
('settings.version.read','查看版本设置','系统设置','查看客户端版本'),
('settings.version.update','更新版本设置','系统设置','修改客户端版本'),
('backup.create','下载数据库备份','系统设置','创建并下载 SQLite 备份'),
('admins.read','查看管理员','管理员','查看管理员列表'),
('admins.create','创建管理员','管理员','创建并分配管理员角色'),
('admins.update','更新管理员','管理员','启停管理员并调整角色'),
('admins.reset_password','重置管理员密码','管理员','重置其他管理员密码'),
('roles.read','查看角色权限','角色权限','查看角色和权限目录'),
('roles.create','创建角色','角色权限','创建自定义角色'),
('roles.update','更新角色','角色权限','修改角色及其权限'),
('roles.delete','删除角色','角色权限','删除未使用的自定义角色'),
('audit.read','查看操作审计','操作审计','查看管理员操作和安全事件');

INSERT OR IGNORE INTO admin_role_permissions(role_id,permission_code)
SELECT 'super_admin',code FROM admin_permissions;

CREATE TABLE IF NOT EXISTS identities (
  id TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  token_hash TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,
  revoked_at INTEGER
);

CREATE TABLE IF NOT EXISTS filters (
  id TEXT PRIMARY KEY,
  owner_id TEXT NOT NULL REFERENCES identities(id),
  client_filter_id TEXT NOT NULL,
  derived_from_filter_id TEXT REFERENCES filters(id),
  status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','disabled','deleted')),
  current_revision INTEGER NOT NULL,
  like_count INTEGER NOT NULL DEFAULT 0,
  download_count INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(owner_id, client_filter_id)
);
CREATE TABLE IF NOT EXISTS filter_revisions (
  filter_id TEXT NOT NULL REFERENCES filters(id) ON DELETE CASCADE,
  revision INTEGER NOT NULL,
  name TEXT NOT NULL,
  value TEXT NOT NULL,
  use_regex INTEGER NOT NULL,
  note TEXT NOT NULL,
  collaborative INTEGER NOT NULL DEFAULT 0,
  editor_type TEXT NOT NULL CHECK(editor_type IN ('owner','collaborator','admin')),
  editor_id TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY(filter_id, revision)
);
CREATE TABLE IF NOT EXISTS filter_likes (
  filter_id TEXT NOT NULL REFERENCES filters(id) ON DELETE CASCADE,
  identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL,
  PRIMARY KEY(filter_id, identity_id)
);
CREATE TABLE IF NOT EXISTS filter_downloads (
  filter_id TEXT NOT NULL REFERENCES filters(id) ON DELETE CASCADE,
  revision INTEGER NOT NULL,
  identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL,
  PRIMARY KEY(filter_id, revision, identity_id)
);
CREATE INDEX IF NOT EXISTS idx_filters_status_updated ON filters(status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_filters_downloads ON filters(status, download_count DESC);
CREATE INDEX IF NOT EXISTS idx_filters_likes ON filters(status, like_count DESC);
CREATE INDEX IF NOT EXISTS idx_identities_name ON identities(display_name COLLATE NOCASE);

CREATE TABLE IF NOT EXISTS request_nonces (
  client_key_id TEXT NOT NULL,
  nonce TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  PRIMARY KEY(client_key_id, nonce)
);
CREATE INDEX IF NOT EXISTS idx_request_nonces_expiry ON request_nonces(expires_at);

CREATE TABLE IF NOT EXISTS upload_requests (
  identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
  request_id TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY(identity_id, request_id)
);
CREATE INDEX IF NOT EXISTS idx_upload_requests_identity_time ON upload_requests(identity_id, created_at);

CREATE TABLE IF NOT EXISTS client_sessions (
  token_hash TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
  csrf_hash TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_client_sessions_identity_created ON client_sessions(identity_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_client_sessions_expiry ON client_sessions(expires_at);
`

const migrationV4SQL = `
ALTER TABLE filter_revisions RENAME TO filter_revisions_v3;
CREATE TABLE filter_revisions (
  filter_id TEXT NOT NULL REFERENCES filters(id) ON DELETE CASCADE,
  revision INTEGER NOT NULL,
  name TEXT NOT NULL,
  value TEXT NOT NULL,
  use_regex INTEGER NOT NULL,
  note TEXT NOT NULL,
  collaborative INTEGER NOT NULL DEFAULT 0,
  editor_type TEXT NOT NULL CHECK(editor_type IN ('owner','collaborator','admin')),
  editor_id TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY(filter_id, revision)
);
INSERT INTO filter_revisions(
  filter_id, revision, name, value, use_regex, note, collaborative,
  editor_type, editor_id, created_at
)
SELECT
  filter_id, revision, name, value, use_regex, note, 0,
  editor_type, editor_id, created_at
FROM filter_revisions_v3;
DROP TABLE filter_revisions_v3;
`

const migrationV5SQL = `
CREATE TABLE IF NOT EXISTS admin_roles (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE COLLATE NOCASE,
  description TEXT NOT NULL DEFAULT '',
  built_in INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS admin_permissions (
  code TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  group_name TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS admin_role_members (
  admin_id TEXT NOT NULL REFERENCES admins(id) ON DELETE CASCADE,
  role_id TEXT NOT NULL REFERENCES admin_roles(id) ON DELETE RESTRICT,
  created_at INTEGER NOT NULL,
  PRIMARY KEY(admin_id, role_id)
);
CREATE INDEX IF NOT EXISTS idx_admin_role_members_role ON admin_role_members(role_id);
CREATE TABLE IF NOT EXISTS admin_role_permissions (
  role_id TEXT NOT NULL REFERENCES admin_roles(id) ON DELETE CASCADE,
  permission_code TEXT NOT NULL REFERENCES admin_permissions(code) ON DELETE CASCADE,
  PRIMARY KEY(role_id, permission_code)
);
CREATE TABLE IF NOT EXISTS admin_audit_logs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  admin_id TEXT,
  username TEXT NOT NULL DEFAULT '',
  action TEXT NOT NULL,
  resource_type TEXT NOT NULL DEFAULT '',
  resource_id TEXT NOT NULL DEFAULT '',
  result TEXT NOT NULL CHECK(result IN ('success','failure','denied')),
  client_address TEXT NOT NULL DEFAULT '',
  metadata TEXT NOT NULL DEFAULT '{}',
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_admin_audit_logs_created ON admin_audit_logs(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_admin_audit_logs_admin ON admin_audit_logs(admin_id, created_at DESC);

INSERT OR IGNORE INTO admin_roles(id,name,description,built_in,created_at,updated_at)
VALUES('super_admin','超级管理员','拥有全部管理权限',1,0,0);

INSERT OR IGNORE INTO admin_permissions(code,name,group_name,description) VALUES
('dashboard.read','查看仪表盘','仪表盘','查看管理概览与趋势'),
('filters.read','查看关键词','关键词','查看云端关键词'),
('filters.update','编辑关键词','关键词','修改关键词内容'),
('filters.status','变更关键词状态','关键词','停用、恢复或删除关键词'),
('filters.export','导出关键词','关键词','按筛选条件导出云端关键词'),
('identities.read','查看访问身份','访问身份','查看客户端身份'),
('identities.revoke','撤销访问身份','访问身份','撤销或恢复客户端身份'),
('settings.version.read','查看版本设置','系统设置','查看客户端版本'),
('settings.version.update','更新版本设置','系统设置','修改客户端版本'),
('backup.create','下载数据库备份','系统设置','创建并下载 SQLite 备份'),
('admins.read','查看管理员','管理员','查看管理员列表'),
('admins.create','创建管理员','管理员','创建并分配管理员角色'),
('admins.update','更新管理员','管理员','启停管理员并调整角色'),
('admins.reset_password','重置管理员密码','管理员','重置其他管理员密码'),
('roles.read','查看角色权限','角色权限','查看角色和权限目录'),
('roles.create','创建角色','角色权限','创建自定义角色'),
('roles.update','更新角色','角色权限','修改角色及其权限'),
('roles.delete','删除角色','角色权限','删除未使用的自定义角色'),
('audit.read','查看操作审计','操作审计','查看管理员操作和安全事件');

INSERT OR IGNORE INTO admin_role_permissions(role_id,permission_code)
SELECT 'super_admin',code FROM admin_permissions;
INSERT OR IGNORE INTO admin_role_members(admin_id,role_id,created_at)
SELECT id,'super_admin',0 FROM admins;
`
