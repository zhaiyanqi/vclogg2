package store

import (
	"context"
	"database/sql"
	"os"
	"path/filepath"
	"testing"
)

func TestOpenCreatesPortableDatabase(t *testing.T) {
	dataDir := t.TempDir()
	database, err := Open(context.Background(), dataDir)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()

	var version int
	if err := database.DB.QueryRow(`SELECT MAX(version) FROM schema_migrations`).Scan(&version); err != nil {
		t.Fatal(err)
	}
	if version != schemaVersion {
		t.Fatalf("schema version = %d, want %d", version, schemaVersion)
	}
	var journalMode string
	if err := database.DB.QueryRow(`PRAGMA journal_mode`).Scan(&journalMode); err != nil {
		t.Fatal(err)
	}
	if journalMode != "wal" {
		t.Fatalf("journal mode = %q, want wal", journalMode)
	}
	var exportPermission int
	if err := database.DB.QueryRow(`SELECT COUNT(*) FROM admin_permissions WHERE code='filters.export'`).Scan(&exportPermission); err != nil || exportPermission != 1 {
		t.Fatalf("filters.export permission count = %d, err=%v", exportPermission, err)
	}
}

func TestMigratesVersionTwoDatabaseToClientSessions(t *testing.T) {
	dataDir := t.TempDir()
	databasePath := filepath.Join(dataDir, "vclogg-server.db")
	legacy, err := sql.Open("sqlite", databasePath)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = legacy.Exec(`
CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);
INSERT INTO schema_migrations(version, applied_at) VALUES(2, 0);
`); err != nil {
		legacy.Close()
		t.Fatal(err)
	}
	if err = legacy.Close(); err != nil {
		t.Fatal(err)
	}

	database, err := Open(context.Background(), dataDir)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()

	var version int
	if err := database.DB.QueryRow(`SELECT MAX(version) FROM schema_migrations`).Scan(&version); err != nil {
		t.Fatal(err)
	}
	if version != schemaVersion {
		t.Fatalf("schema version = %d, want %d", version, schemaVersion)
	}
	var table string
	if err := database.DB.QueryRow(`SELECT name FROM sqlite_master WHERE type='table' AND name='client_sessions'`).Scan(&table); err != nil {
		t.Fatal(err)
	}
	backups, err := filepath.Glob(filepath.Join(dataDir, "backups", "pre-migration-*.db"))
	if err != nil {
		t.Fatal(err)
	}
	if len(backups) != 1 {
		t.Fatalf("pre-migration backups = %d, want 1", len(backups))
	}
}

func TestMigratesVersionThreeFilterHistoryToCollaborativeRevisions(t *testing.T) {
	dataDir := t.TempDir()
	databasePath := filepath.Join(dataDir, "vclogg-server.db")
	legacy, err := sql.Open("sqlite", databasePath)
	if err != nil {
		t.Fatal(err)
	}
	_, err = legacy.Exec(`
CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);
INSERT INTO schema_migrations(version, applied_at) VALUES(3, 0);
CREATE TABLE identities (
  id TEXT PRIMARY KEY, display_name TEXT NOT NULL, token_hash TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL, revoked_at INTEGER
);
CREATE TABLE filters (
  id TEXT PRIMARY KEY, owner_id TEXT NOT NULL REFERENCES identities(id), client_filter_id TEXT NOT NULL,
  derived_from_filter_id TEXT REFERENCES filters(id), status TEXT NOT NULL DEFAULT 'active',
  current_revision INTEGER NOT NULL, like_count INTEGER NOT NULL DEFAULT 0,
  download_count INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
  UNIQUE(owner_id, client_filter_id)
);
CREATE TABLE filter_revisions (
  filter_id TEXT NOT NULL REFERENCES filters(id) ON DELETE CASCADE, revision INTEGER NOT NULL,
  name TEXT NOT NULL, value TEXT NOT NULL, use_regex INTEGER NOT NULL, note TEXT NOT NULL,
  editor_type TEXT NOT NULL CHECK(editor_type IN ('owner','admin')), editor_id TEXT NOT NULL,
  created_at INTEGER NOT NULL, PRIMARY KEY(filter_id, revision)
);
INSERT INTO identities(id,display_name,token_hash,created_at,last_seen_at) VALUES('owner','Alice','hash',0,0);
INSERT INTO filters(id,owner_id,client_filter_id,status,current_revision,created_at,updated_at) VALUES('filter','owner','local','active',1,0,0);
INSERT INTO filter_revisions(filter_id,revision,name,value,use_regex,note,editor_type,editor_id,created_at) VALUES('filter',1,'Camera','CameraService',0,'legacy','owner','owner',0);
`)
	if err != nil {
		legacy.Close()
		t.Fatal(err)
	}
	if err = legacy.Close(); err != nil {
		t.Fatal(err)
	}

	database, err := Open(context.Background(), dataDir)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	var version, collaborative int
	var name, editorType string
	if err := database.DB.QueryRow(`SELECT MAX(version) FROM schema_migrations`).Scan(&version); err != nil {
		t.Fatal(err)
	}
	if err := database.DB.QueryRow(`SELECT name,collaborative,editor_type FROM filter_revisions WHERE filter_id='filter' AND revision=1`).Scan(&name, &collaborative, &editorType); err != nil {
		t.Fatal(err)
	}
	if version != schemaVersion || name != "Camera" || collaborative != 0 || editorType != "owner" {
		t.Fatalf("migrated revision = version %d, name %q, collaborative %d, editor %q", version, name, collaborative, editorType)
	}
}

func TestMigratesVersionFourAdministratorsToRBAC(t *testing.T) {
	dataDir := t.TempDir()
	databasePath := filepath.Join(dataDir, "vclogg-server.db")
	legacy, err := sql.Open("sqlite", databasePath)
	if err != nil {
		t.Fatal(err)
	}
	_, err = legacy.Exec(`
CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);
INSERT INTO schema_migrations(version, applied_at) VALUES(4, 0);
CREATE TABLE admins (
  id TEXT PRIMARY KEY, username TEXT NOT NULL UNIQUE COLLATE NOCASE,
  password_hash TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
INSERT INTO admins(id,username,password_hash,created_at,updated_at) VALUES('legacy-admin','admin','hash',1,1);
`)
	if err != nil {
		legacy.Close()
		t.Fatal(err)
	}
	if err = legacy.Close(); err != nil {
		t.Fatal(err)
	}
	database, err := Open(context.Background(), dataDir)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	var status, roleID string
	if err := database.DB.QueryRow(`SELECT status FROM admins WHERE id='legacy-admin'`).Scan(&status); err != nil {
		t.Fatal(err)
	}
	if err := database.DB.QueryRow(`SELECT role_id FROM admin_role_members WHERE admin_id='legacy-admin'`).Scan(&roleID); err != nil {
		t.Fatal(err)
	}
	if status != "active" || roleID != "super_admin" {
		t.Fatalf("migrated administrator status=%q role=%q", status, roleID)
	}
	backups, err := filepath.Glob(filepath.Join(dataDir, "backups", "pre-migration-*.db"))
	if err != nil || len(backups) != 1 {
		t.Fatalf("pre-migration backups=%d err=%v", len(backups), err)
	}
}

func TestOnlineBackupAndOfflineRestore(t *testing.T) {
	ctx := context.Background()
	sourceDir := t.TempDir()
	database, err := Open(ctx, sourceDir)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := database.DB.Exec(`UPDATE settings SET value='9.8.7' WHERE key='latest_app_version'`); err != nil {
		t.Fatal(err)
	}
	backupPath, err := database.Backup(ctx, filepath.Join(sourceDir, "backups"), "test")
	if err != nil {
		t.Fatal(err)
	}
	if err := database.Close(); err != nil {
		t.Fatal(err)
	}

	restoredDir := t.TempDir()
	if err := Restore(backupPath, restoredDir); err != nil {
		t.Fatal(err)
	}
	restored, err := Open(ctx, restoredDir)
	if err != nil {
		t.Fatal(err)
	}
	defer restored.Close()
	var version string
	if err := restored.DB.QueryRow(`SELECT value FROM settings WHERE key='latest_app_version'`).Scan(&version); err != nil {
		t.Fatal(err)
	}
	if version != "9.8.7" {
		t.Fatalf("restored version = %q, want 9.8.7", version)
	}
}

func TestRestoreRefusesRunningWALDatabase(t *testing.T) {
	dataDir := t.TempDir()
	source := filepath.Join(t.TempDir(), "source.db")
	if err := os.WriteFile(source, []byte("not important"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dataDir, "vclogg-server.db-wal"), nil, 0o600); err != nil {
		t.Fatal(err)
	}
	if err := Restore(source, dataDir); err == nil {
		t.Fatal("Restore succeeded while a WAL file indicated a running database")
	}
}

func TestRestoreRejectsInvalidDatabase(t *testing.T) {
	dataDir := t.TempDir()
	source := filepath.Join(t.TempDir(), "invalid.db")
	if err := os.WriteFile(source, []byte("not a sqlite database"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := Restore(source, dataDir); err == nil {
		t.Fatal("Restore accepted an invalid SQLite database")
	}
	if _, err := os.Stat(filepath.Join(dataDir, "vclogg-server.db")); !os.IsNotExist(err) {
		t.Fatalf("invalid restore created a destination database: %v", err)
	}
}
