package app

import (
	"database/sql"
	"encoding/csv"
	"encoding/json"
	"errors"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

type adminSession struct {
	AdminID     string
	Username    string
	CSRF        string
	RoleIDs     []string
	Permissions []string
	SuperAdmin  bool
}

func (s *Server) adminLogin(w http.ResponseWriter, r *http.Request) {
	var input struct {
		Username string `json:"username"`
		Password string `json:"password"`
	}
	if !decodeJSON(w, r, &input) {
		return
	}
	input.Username = strings.TrimSpace(input.Username)
	var id, username, passwordHash, status string
	err := s.store.DB.QueryRowContext(r.Context(), `SELECT id,username,password_hash,status FROM admins WHERE username=? COLLATE NOCASE`, input.Username).Scan(&id, &username, &passwordHash, &status)
	if err != nil || status != "active" || !verifyPassword(passwordHash, input.Password) {
		s.audit(r, "", input.Username, "auth.login", "administrator", "", "failure", nil)
		writeError(w, 401, "invalid_credentials", "username or password is incorrect")
		return
	}
	token := randomToken(32)
	csrf := randomToken(24)
	now := time.Now()
	_, err = s.store.DB.ExecContext(r.Context(), `INSERT INTO admin_sessions(token_hash,admin_id,csrf_token,expires_at,created_at) VALUES(?,?,?,?,?)`, tokenHash(token), id, csrf, now.Add(12*time.Hour).UnixMilli(), now.UnixMilli())
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	http.SetCookie(w, &http.Cookie{Name: adminCookie, Value: token, Path: "/admin", HttpOnly: true, SameSite: http.SameSiteStrictMode, Secure: s.isSecureRequest(r), MaxAge: 12 * 60 * 60})
	s.audit(r, id, username, "auth.login", "administrator", id, "success", nil)
	writeJSON(w, 200, map[string]string{"username": username, "csrfToken": csrf})
}

func (s *Server) adminLogout(w http.ResponseWriter, r *http.Request) {
	session, ok := s.requireAdmin(w, r, true)
	if !ok {
		return
	}
	_, _ = s.store.DB.ExecContext(r.Context(), `DELETE FROM admin_sessions WHERE admin_id=? AND csrf_token=?`, session.AdminID, session.CSRF)
	http.SetCookie(w, &http.Cookie{Name: adminCookie, Value: "", Path: "/admin", HttpOnly: true, SameSite: http.SameSiteStrictMode, Secure: s.isSecureRequest(r), MaxAge: -1})
	s.audit(r, session.AdminID, session.Username, "auth.logout", "administrator", session.AdminID, "success", nil)
	writeJSON(w, 200, map[string]bool{"ok": true})
}
func (s *Server) adminSessionInfo(w http.ResponseWriter, r *http.Request) {
	session, ok := s.requireAdmin(w, r, false)
	if !ok {
		return
	}
	writeJSON(w, 200, adminSessionPayload(session))
}
func (s *Server) adminChangePassword(w http.ResponseWriter, r *http.Request) {
	session, ok := s.requireAdmin(w, r, true)
	if !ok {
		return
	}
	var input struct {
		CurrentPassword string `json:"currentPassword"`
		NewPassword     string `json:"newPassword"`
	}
	if !decodeJSON(w, r, &input) {
		return
	}
	var encoded string
	if err := s.store.DB.QueryRowContext(r.Context(), `SELECT password_hash FROM admins WHERE id=?`, session.AdminID).Scan(&encoded); err != nil || !verifyPassword(encoded, input.CurrentPassword) {
		writeError(w, 401, "invalid_password", "current password is incorrect")
		return
	}
	if len(input.NewPassword) < 10 {
		writeError(w, 400, "weak_password", "new password must contain at least 10 characters")
		return
	}
	encoded, err := hashPassword(input.NewPassword)
	if err != nil {
		writeError(w, 500, "password_failed", err.Error())
		return
	}
	tx, err := s.store.DB.BeginTx(r.Context(), nil)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(r.Context(), `UPDATE admins SET password_hash=?,updated_at=? WHERE id=?`, encoded, time.Now().UnixMilli(), session.AdminID); err == nil {
		_, err = tx.ExecContext(r.Context(), `DELETE FROM admin_sessions WHERE admin_id=? AND csrf_token<>?`, session.AdminID, session.CSRF)
	}
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	if err = tx.Commit(); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	s.audit(r, session.AdminID, session.Username, "account.password_change", "administrator", session.AdminID, "success", nil)
	writeJSON(w, 200, map[string]bool{"ok": true})
}

func adminSessionPayload(session adminSession) map[string]any {
	return map[string]any{
		"id":          session.AdminID,
		"username":    session.Username,
		"csrfToken":   session.CSRF,
		"roleIds":     session.RoleIDs,
		"permissions": session.Permissions,
		"superAdmin":  session.SuperAdmin,
	}
}

func (s *Server) requireAdmin(w http.ResponseWriter, r *http.Request, mutation bool) (adminSession, bool) {
	cookie, err := r.Cookie(adminCookie)
	if err != nil {
		writeError(w, 401, "admin_authentication_required", "administrator login is required")
		return adminSession{}, false
	}
	var session adminSession
	err = s.store.DB.QueryRowContext(r.Context(), `SELECT a.id,a.username,s.csrf_token FROM admin_sessions s JOIN admins a ON a.id=s.admin_id WHERE s.token_hash=? AND s.expires_at>? AND a.status='active'`, tokenHash(cookie.Value), time.Now().UnixMilli()).Scan(&session.AdminID, &session.Username, &session.CSRF)
	if err != nil {
		writeError(w, 401, "admin_session_expired", "administrator session has expired")
		return adminSession{}, false
	}
	if mutation && r.Header.Get("X-CSRF-Token") != session.CSRF {
		writeError(w, 403, "csrf_failed", "a valid CSRF token is required")
		return adminSession{}, false
	}
	session.RoleIDs, session.Permissions, session.SuperAdmin, err = s.loadAdminAccess(r.Context(), session.AdminID)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return adminSession{}, false
	}
	return session, true
}

func (s *Server) adminGetVersion(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.requirePermission(w, r, "settings.version.read", false); !ok {
		return
	}
	s.getAppVersion(w, r)
}
func (s *Server) adminSetVersion(w http.ResponseWriter, r *http.Request) {
	session, ok := s.requirePermission(w, r, "settings.version.update", true)
	if !ok {
		return
	}
	var input struct {
		LatestVersion string `json:"latestVersion"`
	}
	if !decodeJSON(w, r, &input) {
		return
	}
	input.LatestVersion = strings.TrimSpace(input.LatestVersion)
	if !semverPattern.MatchString(input.LatestVersion) {
		writeError(w, 400, "invalid_version", "latestVersion must be a valid semantic version")
		return
	}
	now := time.Now().UnixMilli()
	_, err := s.store.DB.ExecContext(r.Context(), `INSERT INTO settings(key,value,updated_at) VALUES('latest_app_version',?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at`, input.LatestVersion, now)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	s.audit(r, session.AdminID, session.Username, "settings.version_update", "setting", "latest_app_version", "success", map[string]any{"latestVersion": input.LatestVersion})
	writeJSON(w, 200, map[string]any{"latestVersion": input.LatestVersion, "updatedAt": now})
}

func (s *Server) adminListFilters(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.requirePermission(w, r, "filters.read", false); !ok {
		return
	}
	query := strings.TrimSpace(r.URL.Query().Get("q"))
	status := r.URL.Query().Get("status")
	if status == "" {
		status = "active"
	}
	if status != "active" && status != "disabled" && status != "deleted" && status != "all" {
		writeError(w, 400, "invalid_status", "invalid status")
		return
	}
	page := boundedInt(r.URL.Query().Get("page"), 1, 1, 100000)
	pageSize := boundedInt(r.URL.Query().Get("pageSize"), 50, 1, 100)
	pattern := "%" + escapeLike(query) + "%"
	where := `(?='all' OR f.status=?) AND (?='' OR r.name LIKE ? ESCAPE '\' COLLATE NOCASE OR r.value LIKE ? ESCAPE '\' COLLATE NOCASE OR r.note LIKE ? ESCAPE '\' COLLATE NOCASE OR i.display_name LIKE ? ESCAPE '\' COLLATE NOCASE)`
	args := []any{status, status, query, pattern, pattern, pattern, pattern}
	var total int
	if err := s.store.DB.QueryRowContext(r.Context(), `SELECT COUNT(*) FROM filters f JOIN filter_revisions r ON r.filter_id=f.id AND r.revision=f.current_revision JOIN identities i ON i.id=f.owner_id WHERE `+where, args...).Scan(&total); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	rows, err := s.store.DB.QueryContext(r.Context(), `SELECT f.id,f.status,f.current_revision,r.name,r.value,r.use_regex,r.note,r.collaborative,i.id,i.display_name,f.like_count,f.download_count,f.updated_at FROM filters f JOIN filter_revisions r ON r.filter_id=f.id AND r.revision=f.current_revision JOIN identities i ON i.id=f.owner_id WHERE `+where+` ORDER BY f.updated_at DESC LIMIT ? OFFSET ?`, append(args, pageSize, (page-1)*pageSize)...)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer rows.Close()
	items := make([]map[string]any, 0)
	for rows.Next() {
		var id, state, name, value, note, ownerID, ownerName string
		var revision, useRegex, collaborative, likes, downloads int
		var updated int64
		if err := rows.Scan(&id, &state, &revision, &name, &value, &useRegex, &note, &collaborative, &ownerID, &ownerName, &likes, &downloads, &updated); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
		items = append(items, map[string]any{"id": id, "status": state, "revision": revision, "name": name, "value": value, "useRegex": useRegex != 0, "note": note, "collaborative": collaborative != 0, "ownerId": ownerID, "ownerName": ownerName, "likeCount": likes, "downloadCount": downloads, "updatedAt": updated})
	}
	writeJSON(w, 200, map[string]any{"items": items, "page": page, "pageSize": pageSize, "total": total})
}

type adminFilterExportItem struct {
	ID            string `json:"id"`
	Status        string `json:"status"`
	Revision      int    `json:"revision"`
	Name          string `json:"name"`
	Value         string `json:"value"`
	UseRegex      bool   `json:"useRegex"`
	Note          string `json:"note"`
	Collaborative bool   `json:"collaborative"`
	OwnerID       string `json:"ownerId"`
	OwnerName     string `json:"ownerName"`
	LikeCount     int    `json:"likeCount"`
	DownloadCount int    `json:"downloadCount"`
	UpdatedAt     int64  `json:"updatedAt"`
}

func (s *Server) adminExportFilters(w http.ResponseWriter, r *http.Request) {
	session, ok := s.requirePermission(w, r, "filters.export", false)
	if !ok {
		return
	}
	query := strings.TrimSpace(r.URL.Query().Get("q"))
	status := r.URL.Query().Get("status")
	if status == "" {
		status = "active"
	}
	if status != "active" && status != "disabled" && status != "deleted" && status != "all" {
		writeError(w, 400, "invalid_status", "invalid status")
		return
	}
	format := strings.ToLower(strings.TrimSpace(r.URL.Query().Get("format")))
	if format == "" {
		format = "json"
	}
	if format != "json" && format != "csv" {
		writeError(w, 400, "invalid_export_format", "format must be json or csv")
		return
	}
	pattern := "%" + escapeLike(query) + "%"
	where := `(?='all' OR f.status=?) AND (?='' OR r.name LIKE ? ESCAPE '\' COLLATE NOCASE OR r.value LIKE ? ESCAPE '\' COLLATE NOCASE OR r.note LIKE ? ESCAPE '\' COLLATE NOCASE OR i.display_name LIKE ? ESCAPE '\' COLLATE NOCASE)`
	args := []any{status, status, query, pattern, pattern, pattern, pattern}
	rows, err := s.store.DB.QueryContext(r.Context(), `SELECT f.id,f.status,f.current_revision,r.name,r.value,r.use_regex,r.note,r.collaborative,i.id,i.display_name,f.like_count,f.download_count,f.updated_at FROM filters f JOIN filter_revisions r ON r.filter_id=f.id AND r.revision=f.current_revision JOIN identities i ON i.id=f.owner_id WHERE `+where+` ORDER BY f.updated_at DESC,f.id`, args...)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer rows.Close()
	items := make([]adminFilterExportItem, 0)
	for rows.Next() {
		var item adminFilterExportItem
		var useRegex, collaborative int
		if err := rows.Scan(&item.ID, &item.Status, &item.Revision, &item.Name, &item.Value, &useRegex, &item.Note, &collaborative, &item.OwnerID, &item.OwnerName, &item.LikeCount, &item.DownloadCount, &item.UpdatedAt); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
		item.UseRegex = useRegex != 0
		item.Collaborative = collaborative != 0
		items = append(items, item)
	}
	if err := rows.Err(); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	filename := "vclogg-filters-" + time.Now().UTC().Format("2006-01-02") + "." + format
	w.Header().Set("Content-Disposition", `attachment; filename="`+filename+`"`)
	if format == "json" {
		w.Header().Set("Content-Type", "application/json; charset=utf-8")
		_ = json.NewEncoder(w).Encode(map[string]any{"version": 1, "exportedAt": time.Now().UnixMilli(), "filters": items})
	} else {
		w.Header().Set("Content-Type", "text/csv; charset=utf-8")
		_, _ = w.Write([]byte{0xef, 0xbb, 0xbf})
		writer := csv.NewWriter(w)
		_ = writer.Write([]string{"id", "status", "revision", "name", "value", "useRegex", "note", "collaborative", "ownerId", "ownerName", "likeCount", "downloadCount", "updatedAt"})
		for _, item := range items {
			_ = writer.Write([]string{item.ID, item.Status, strconv.Itoa(item.Revision), item.Name, item.Value, strconv.FormatBool(item.UseRegex), item.Note, strconv.FormatBool(item.Collaborative), item.OwnerID, item.OwnerName, strconv.Itoa(item.LikeCount), strconv.Itoa(item.DownloadCount), strconv.FormatInt(item.UpdatedAt, 10)})
		}
		writer.Flush()
	}
	s.audit(r, session.AdminID, session.Username, "filter.export", "filter", "", "success", map[string]any{"format": format, "status": status, "query": query, "count": len(items)})
}

func (s *Server) adminUpdateFilter(w http.ResponseWriter, r *http.Request) {
	admin, ok := s.requireAdmin(w, r, true)
	if !ok {
		return
	}
	id := r.PathValue("id")
	var input struct {
		Name         *string `json:"name"`
		Value        *string `json:"value"`
		UseRegex     *bool   `json:"useRegex"`
		Note         *string `json:"note"`
		Status       *string `json:"status"`
		BaseRevision *int    `json:"baseRevision"`
	}
	if !decodeJSON(w, r, &input) {
		return
	}
	contentUpdate := input.Name != nil || input.Value != nil || input.UseRegex != nil || input.Note != nil
	statusUpdate := input.Status != nil
	if !contentUpdate && !statusUpdate {
		writeError(w, 400, "empty_update", "filter content or status is required")
		return
	}
	if contentUpdate && !admin.SuperAdmin && !containsString(admin.Permissions, "filters.update") {
		s.audit(r, admin.AdminID, admin.Username, "authorization.denied", "permission", "filters.update", "denied", map[string]any{"method": r.Method, "path": r.URL.Path})
		writeError(w, http.StatusForbidden, "permission_denied", "administrator permission is required")
		return
	}
	if statusUpdate && !admin.SuperAdmin && !containsString(admin.Permissions, "filters.status") {
		s.audit(r, admin.AdminID, admin.Username, "authorization.denied", "permission", "filters.status", "denied", map[string]any{"method": r.Method, "path": r.URL.Path})
		writeError(w, http.StatusForbidden, "permission_denied", "administrator permission is required")
		return
	}
	tx, err := s.store.DB.BeginTx(r.Context(), nil)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer tx.Rollback()
	var revision int
	var name, value, note, state string
	var useRegex, collaborative int
	err = tx.QueryRowContext(r.Context(), `SELECT f.current_revision,r.name,r.value,r.use_regex,r.note,r.collaborative,f.status FROM filters f JOIN filter_revisions r ON r.filter_id=f.id AND r.revision=f.current_revision WHERE f.id=?`, id).Scan(&revision, &name, &value, &useRegex, &note, &collaborative, &state)
	if errors.Is(err, sql.ErrNoRows) {
		writeError(w, 404, "filter_not_found", "filter was not found")
		return
	}
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	if input.BaseRevision != nil && *input.BaseRevision != revision {
		writeRevisionConflict(w, revision)
		return
	}
	if input.Name != nil {
		name = strings.TrimSpace(*input.Name)
	}
	if input.Value != nil {
		value = strings.TrimSpace(*input.Value)
	}
	if input.Note != nil {
		note = strings.TrimSpace(*input.Note)
	}
	if input.UseRegex != nil {
		useRegex = boolInt(*input.UseRegex)
	}
	if input.Status != nil {
		state = *input.Status
	}
	if state != "active" && state != "disabled" && state != "deleted" {
		writeError(w, 400, "invalid_status", "status must be active, disabled, or deleted")
		return
	}
	candidate := shareItem{ClientFilterID: "admin", Name: name, Value: value, UseRegex: useRegex != 0, Note: note}
	if message := validateShareItem(&candidate); message != "" {
		writeError(w, 400, "invalid_filter", message)
		return
	}
	var previousName, previousValue, previousNote string
	var previousRegex int
	_ = tx.QueryRowContext(r.Context(), `SELECT name,value,use_regex,note FROM filter_revisions WHERE filter_id=? AND revision=?`, id, revision).Scan(&previousName, &previousValue, &previousRegex, &previousNote)
	now := time.Now().UnixMilli()
	if previousName != name || previousValue != value || previousRegex != useRegex || previousNote != note {
		revision++
		if _, err = tx.ExecContext(r.Context(), `INSERT INTO filter_revisions(filter_id,revision,name,value,use_regex,note,collaborative,editor_type,editor_id,created_at) VALUES(?,?,?,?,?,?,?,?,?,?)`, id, revision, name, value, useRegex, note, collaborative, "admin", admin.AdminID, now); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
	}
	if _, err = tx.ExecContext(r.Context(), `UPDATE filters SET current_revision=?,status=?,updated_at=? WHERE id=?`, revision, state, now, id); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	if err = tx.Commit(); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	s.audit(r, admin.AdminID, admin.Username, "filter.update", "filter", id, "success", map[string]any{"status": state, "revision": revision})
	writeJSON(w, 200, map[string]any{"id": id, "revision": revision, "status": state})
}

func (s *Server) adminListIdentities(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.requirePermission(w, r, "identities.read", false); !ok {
		return
	}
	query := strings.TrimSpace(r.URL.Query().Get("q"))
	page := boundedInt(r.URL.Query().Get("page"), 1, 1, 100000)
	pageSize := boundedInt(r.URL.Query().Get("pageSize"), 50, 1, 100)
	pattern := "%" + escapeLike(query) + "%"
	var total int
	if err := s.store.DB.QueryRowContext(r.Context(), `SELECT COUNT(*) FROM identities WHERE ?='' OR display_name LIKE ? ESCAPE '\' COLLATE NOCASE`, query, pattern).Scan(&total); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	rows, err := s.store.DB.QueryContext(r.Context(), `SELECT id,display_name,created_at,last_seen_at,revoked_at FROM identities WHERE ?='' OR display_name LIKE ? ESCAPE '\' COLLATE NOCASE ORDER BY last_seen_at DESC LIMIT ? OFFSET ?`, query, pattern, pageSize, (page-1)*pageSize)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer rows.Close()
	items := make([]map[string]any, 0)
	for rows.Next() {
		var id, name string
		var created, lastSeen int64
		var revoked sql.NullInt64
		if err := rows.Scan(&id, &name, &created, &lastSeen, &revoked); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
		items = append(items, map[string]any{"id": id, "displayName": name, "createdAt": created, "lastSeenAt": lastSeen, "revokedAt": nullableInt64(revoked)})
	}
	writeJSON(w, 200, map[string]any{"items": items, "page": page, "pageSize": pageSize, "total": total})
}
func (s *Server) adminUpdateIdentity(w http.ResponseWriter, r *http.Request) {
	session, ok := s.requirePermission(w, r, "identities.revoke", true)
	if !ok {
		return
	}
	var input struct {
		Revoked bool `json:"revoked"`
	}
	if !decodeJSON(w, r, &input) {
		return
	}
	var revoked any
	if input.Revoked {
		revoked = time.Now().UnixMilli()
	}
	tx, err := s.store.DB.BeginTx(r.Context(), nil)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer tx.Rollback()
	result, err := tx.ExecContext(r.Context(), `UPDATE identities SET revoked_at=? WHERE id=?`, revoked, r.PathValue("id"))
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	count, _ := result.RowsAffected()
	if count == 0 {
		writeError(w, 404, "identity_not_found", "identity was not found")
		return
	}
	if input.Revoked {
		if _, err := tx.ExecContext(r.Context(), `DELETE FROM client_sessions WHERE identity_id=?`, r.PathValue("id")); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
	}
	if err := tx.Commit(); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	s.audit(r, session.AdminID, session.Username, "identity.update", "identity", r.PathValue("id"), "success", map[string]any{"revoked": input.Revoked})
	writeJSON(w, 200, map[string]bool{"revoked": input.Revoked})
}

func (s *Server) adminBackup(w http.ResponseWriter, r *http.Request) {
	session, ok := s.requirePermission(w, r, "backup.create", true)
	if !ok {
		return
	}
	target, err := s.store.Backup(r.Context(), filepath.Join(s.store.DataDir, "backups"), "vclogg")
	if err != nil {
		writeError(w, 500, "backup_failed", err.Error())
		return
	}
	file, err := os.Open(target)
	if err != nil {
		writeError(w, 500, "backup_failed", err.Error())
		return
	}
	defer file.Close()
	s.audit(r, session.AdminID, session.Username, "backup.create", "database", filepath.Base(target), "success", nil)
	w.Header().Set("Content-Type", "application/vnd.sqlite3")
	w.Header().Set("Content-Disposition", `attachment; filename="`+filepath.Base(target)+`"`)
	http.ServeContent(w, r, filepath.Base(target), time.Now(), file)
}
func nullableInt64(value sql.NullInt64) any {
	if value.Valid {
		return value.Int64
	}
	return nil
}

func (s *Server) isSecureRequest(r *http.Request) bool {
	return r.TLS != nil || (s.trustedProxy && strings.EqualFold(r.Header.Get("X-Forwarded-Proto"), "https"))
}
