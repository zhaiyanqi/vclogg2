package app

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"net"
	"net/http"
	"strconv"
	"strings"
	"time"
)

const superAdminRoleID = "super_admin"

type adminRoleSummary struct {
	ID          string   `json:"id"`
	Name        string   `json:"name"`
	Description string   `json:"description"`
	BuiltIn     bool     `json:"builtIn"`
	Permissions []string `json:"permissions"`
	MemberCount int      `json:"memberCount"`
}

type adminPermission struct {
	Code        string `json:"code"`
	Name        string `json:"name"`
	Group       string `json:"group"`
	Description string `json:"description"`
}

func (s *Server) requirePermission(w http.ResponseWriter, r *http.Request, permission string, mutation bool) (adminSession, bool) {
	return s.requireAnyPermission(w, r, []string{permission}, mutation)
}

func (s *Server) requireAnyPermission(w http.ResponseWriter, r *http.Request, permissions []string, mutation bool) (adminSession, bool) {
	session, ok := s.requireAdmin(w, r, mutation)
	if !ok {
		return adminSession{}, false
	}
	if session.SuperAdmin {
		return session, true
	}
	for _, permission := range permissions {
		if containsString(session.Permissions, permission) {
			return session, true
		}
	}
	s.audit(r, session.AdminID, session.Username, "authorization.denied", "permission", strings.Join(permissions, ","), "denied", map[string]any{
		"method": r.Method,
		"path":   r.URL.Path,
	})
	writeError(w, http.StatusForbidden, "permission_denied", "administrator permission is required")
	return adminSession{}, false
}

func (s *Server) loadAdminAccess(ctx context.Context, adminID string) ([]string, []string, bool, error) {
	rows, err := s.store.DB.QueryContext(ctx, `
SELECT r.id,r.id=?
FROM admin_role_members m
JOIN admin_roles r ON r.id=m.role_id
WHERE m.admin_id=?
ORDER BY r.name COLLATE NOCASE`, superAdminRoleID, adminID)
	if err != nil {
		return nil, nil, false, err
	}
	roles := make([]string, 0)
	superAdmin := false
	for rows.Next() {
		var roleID string
		var isSuper int
		if err := rows.Scan(&roleID, &isSuper); err != nil {
			rows.Close()
			return nil, nil, false, err
		}
		roles = append(roles, roleID)
		superAdmin = superAdmin || isSuper != 0
	}
	if err := rows.Close(); err != nil {
		return nil, nil, false, err
	}

	permissionRows, err := s.store.DB.QueryContext(ctx, `
SELECT DISTINCT p.code
FROM admin_role_members m
JOIN admin_role_permissions rp ON rp.role_id=m.role_id
JOIN admin_permissions p ON p.code=rp.permission_code
WHERE m.admin_id=?
ORDER BY p.code`, adminID)
	if err != nil {
		return nil, nil, false, err
	}
	defer permissionRows.Close()
	permissions := make([]string, 0)
	for permissionRows.Next() {
		var code string
		if err := permissionRows.Scan(&code); err != nil {
			return nil, nil, false, err
		}
		permissions = append(permissions, code)
	}
	return roles, permissions, superAdmin, permissionRows.Err()
}

func containsString(values []string, wanted string) bool {
	for _, value := range values {
		if value == wanted {
			return true
		}
	}
	return false
}

func uniqueStrings(values []string) []string {
	seen := make(map[string]bool, len(values))
	result := make([]string, 0, len(values))
	for _, value := range values {
		value = strings.TrimSpace(value)
		if value != "" && !seen[value] {
			seen[value] = true
			result = append(result, value)
		}
	}
	return result
}

func commaValues(value string) []string {
	if value == "" {
		return []string{}
	}
	return strings.Split(value, ",")
}

func placeholders(count int) string {
	if count < 1 {
		return ""
	}
	return strings.TrimSuffix(strings.Repeat("?,", count), ",")
}

func validateReferences(tx *sql.Tx, table, column string, values []string) error {
	values = uniqueStrings(values)
	if len(values) == 0 {
		return errors.New("at least one value is required")
	}
	args := make([]any, len(values))
	for index, value := range values {
		args[index] = value
	}
	var count int
	query := "SELECT COUNT(*) FROM " + table + " WHERE " + column + " IN (" + placeholders(len(values)) + ")"
	if err := tx.QueryRow(query, args...).Scan(&count); err != nil {
		return err
	}
	if count != len(values) {
		return errors.New("one or more references do not exist")
	}
	return nil
}

func (s *Server) audit(r *http.Request, adminID, username, action, resourceType, resourceID, result string, metadata any) {
	encoded := []byte("{}")
	if metadata != nil {
		if value, err := json.Marshal(metadata); err == nil {
			encoded = value
		}
	}
	var nullableAdmin any
	if adminID != "" {
		nullableAdmin = adminID
	}
	_, _ = s.store.DB.ExecContext(r.Context(), `
INSERT INTO admin_audit_logs(admin_id,username,action,resource_type,resource_id,result,client_address,metadata,created_at)
VALUES(?,?,?,?,?,?,?,?,?)`, nullableAdmin, username, action, resourceType, resourceID, result, s.clientAddress(r), string(encoded), time.Now().UnixMilli())
}

func (s *Server) clientAddress(r *http.Request) string {
	address := r.RemoteAddr
	if s.trustedProxy {
		if forwarded := strings.TrimSpace(strings.Split(r.Header.Get("X-Forwarded-For"), ",")[0]); forwarded != "" {
			address = forwarded
		} else if realIP := strings.TrimSpace(r.Header.Get("X-Real-IP")); realIP != "" {
			address = realIP
		}
	}
	if host, _, err := net.SplitHostPort(address); err == nil {
		return host
	}
	return address
}

func (s *Server) adminDashboard(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.requirePermission(w, r, "dashboard.read", false); !ok {
		return
	}
	result := map[string]any{}
	queries := map[string]string{
		"activeIdentities":  `SELECT COUNT(*) FROM identities WHERE revoked_at IS NULL`,
		"revokedIdentities": `SELECT COUNT(*) FROM identities WHERE revoked_at IS NOT NULL`,
		"activeFilters":     `SELECT COUNT(*) FROM filters WHERE status='active'`,
		"disabledFilters":   `SELECT COUNT(*) FROM filters WHERE status='disabled'`,
		"totalLikes":        `SELECT COALESCE(SUM(like_count),0) FROM filters`,
		"totalDownloads":    `SELECT COALESCE(SUM(download_count),0) FROM filters`,
	}
	for key, query := range queries {
		var value int64
		if err := s.store.DB.QueryRowContext(r.Context(), query).Scan(&value); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
		result[key] = value
	}
	var latestVersion string
	_ = s.store.DB.QueryRowContext(r.Context(), `SELECT value FROM settings WHERE key='latest_app_version'`).Scan(&latestVersion)
	result["latestVersion"] = latestVersion
	start := time.Now().UTC().Truncate(24*time.Hour).AddDate(0, 0, -13)
	type trendItem struct {
		Date       string `json:"date"`
		Identities int    `json:"identities"`
		Filters    int    `json:"filters"`
		Downloads  int    `json:"downloads"`
	}
	trend := make([]trendItem, 14)
	for index := range trend {
		trend[index].Date = start.AddDate(0, 0, index).Format("2006-01-02")
	}
	series := []struct {
		query string
		set   func(*trendItem, int)
	}{
		{`SELECT CAST((created_at-?)/86400000 AS INTEGER),COUNT(*) FROM identities WHERE created_at>=? GROUP BY 1`, func(item *trendItem, value int) { item.Identities = value }},
		{`SELECT CAST((created_at-?)/86400000 AS INTEGER),COUNT(*) FROM filters WHERE created_at>=? GROUP BY 1`, func(item *trendItem, value int) { item.Filters = value }},
		{`SELECT CAST((created_at-?)/86400000 AS INTEGER),COUNT(*) FROM filter_downloads WHERE created_at>=? GROUP BY 1`, func(item *trendItem, value int) { item.Downloads = value }},
	}
	startMillis := start.UnixMilli()
	for _, item := range series {
		rows, err := s.store.DB.QueryContext(r.Context(), item.query, startMillis, startMillis)
		if err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
		for rows.Next() {
			var day, count int
			if err := rows.Scan(&day, &count); err != nil {
				rows.Close()
				writeError(w, 500, "database_error", err.Error())
				return
			}
			if day >= 0 && day < len(trend) {
				item.set(&trend[day], count)
			}
		}
		rows.Close()
	}
	result["trend"] = trend
	writeJSON(w, 200, result)
}

func (s *Server) adminListAdministrators(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.requirePermission(w, r, "admins.read", false); !ok {
		return
	}
	query := strings.TrimSpace(r.URL.Query().Get("q"))
	page := boundedInt(r.URL.Query().Get("page"), 1, 1, 100000)
	pageSize := boundedInt(r.URL.Query().Get("pageSize"), 30, 1, 100)
	pattern := "%" + escapeLike(query) + "%"
	var total int
	if err := s.store.DB.QueryRowContext(r.Context(), `SELECT COUNT(*) FROM admins WHERE ?='' OR username LIKE ? ESCAPE '\' COLLATE NOCASE`, query, pattern).Scan(&total); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	rows, err := s.store.DB.QueryContext(r.Context(), `
SELECT a.id,a.username,a.status,a.created_at,a.updated_at,COALESCE(GROUP_CONCAT(m.role_id),'')
FROM admins a LEFT JOIN admin_role_members m ON m.admin_id=a.id
WHERE ?='' OR a.username LIKE ? ESCAPE '\' COLLATE NOCASE
GROUP BY a.id ORDER BY a.created_at LIMIT ? OFFSET ?`, query, pattern, pageSize, (page-1)*pageSize)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer rows.Close()
	items := make([]map[string]any, 0)
	for rows.Next() {
		var id, username, status, roleIDs string
		var createdAt, updatedAt int64
		if err := rows.Scan(&id, &username, &status, &createdAt, &updatedAt, &roleIDs); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
		items = append(items, map[string]any{"id": id, "username": username, "status": status, "createdAt": createdAt, "updatedAt": updatedAt, "roleIds": commaValues(roleIDs)})
	}
	writeJSON(w, 200, map[string]any{"items": items, "page": page, "pageSize": pageSize, "total": total})
}

func (s *Server) adminCreateAdministrator(w http.ResponseWriter, r *http.Request) {
	session, ok := s.requirePermission(w, r, "admins.create", true)
	if !ok {
		return
	}
	var input struct {
		Username string   `json:"username"`
		Password string   `json:"password"`
		RoleIDs  []string `json:"roleIds"`
	}
	if !decodeJSON(w, r, &input) {
		return
	}
	input.Username = strings.TrimSpace(input.Username)
	input.RoleIDs = uniqueStrings(input.RoleIDs)
	if input.Username == "" || len([]rune(input.Username)) > 64 || len(input.Password) < 10 || len(input.RoleIDs) == 0 {
		writeError(w, 400, "invalid_administrator", "username, a 10-character password, and at least one role are required")
		return
	}
	encoded, err := hashPassword(input.Password)
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
	if err := validateReferences(tx, "admin_roles", "id", input.RoleIDs); err != nil {
		writeError(w, 400, "invalid_roles", err.Error())
		return
	}
	id := randomID()
	now := time.Now().UnixMilli()
	if _, err = tx.ExecContext(r.Context(), `INSERT INTO admins(id,username,password_hash,status,created_at,updated_at) VALUES(?,?,?,'active',?,?)`, id, input.Username, encoded, now, now); err != nil {
		writeError(w, 409, "administrator_conflict", "administrator username already exists")
		return
	}
	for _, roleID := range input.RoleIDs {
		if _, err = tx.ExecContext(r.Context(), `INSERT INTO admin_role_members(admin_id,role_id,created_at) VALUES(?,?,?)`, id, roleID, now); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
	}
	if err = tx.Commit(); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	s.audit(r, session.AdminID, session.Username, "administrator.create", "administrator", id, "success", map[string]any{"username": input.Username, "roleIds": input.RoleIDs})
	writeJSON(w, 201, map[string]string{"id": id})
}

func (s *Server) adminUpdateAdministrator(w http.ResponseWriter, r *http.Request) {
	session, ok := s.requirePermission(w, r, "admins.update", true)
	if !ok {
		return
	}
	var input struct {
		Status  *string   `json:"status"`
		RoleIDs *[]string `json:"roleIds"`
	}
	if !decodeJSON(w, r, &input) {
		return
	}
	if input.Status == nil && input.RoleIDs == nil {
		writeError(w, 400, "empty_update", "status or roleIds is required")
		return
	}
	id := r.PathValue("id")
	tx, err := s.store.DB.BeginTx(r.Context(), nil)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer tx.Rollback()
	var currentStatus string
	var isSuper int
	err = tx.QueryRowContext(r.Context(), `SELECT a.status,EXISTS(SELECT 1 FROM admin_role_members m WHERE m.admin_id=a.id AND m.role_id=?) FROM admins a WHERE a.id=?`, superAdminRoleID, id).Scan(&currentStatus, &isSuper)
	if errors.Is(err, sql.ErrNoRows) {
		writeError(w, 404, "administrator_not_found", "administrator was not found")
		return
	}
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	newStatus := currentStatus
	if input.Status != nil {
		newStatus = strings.TrimSpace(*input.Status)
		if newStatus != "active" && newStatus != "disabled" {
			writeError(w, 400, "invalid_status", "status must be active or disabled")
			return
		}
	}
	roleIDs := []string(nil)
	removesSuper := false
	if input.RoleIDs != nil {
		roleIDs = uniqueStrings(*input.RoleIDs)
		if len(roleIDs) == 0 {
			writeError(w, 400, "invalid_roles", "at least one role is required")
			return
		}
		if err := validateReferences(tx, "admin_roles", "id", roleIDs); err != nil {
			writeError(w, 400, "invalid_roles", err.Error())
			return
		}
		removesSuper = isSuper != 0 && !containsString(roleIDs, superAdminRoleID)
	}
	if isSuper != 0 && currentStatus == "active" && (newStatus == "disabled" || removesSuper) {
		var count int
		if err := tx.QueryRowContext(r.Context(), `SELECT COUNT(*) FROM admins a JOIN admin_role_members m ON m.admin_id=a.id WHERE a.status='active' AND m.role_id=?`, superAdminRoleID).Scan(&count); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
		if count <= 1 {
			writeError(w, 409, "last_super_administrator", "the final active super administrator cannot be disabled or demoted")
			return
		}
	}
	now := time.Now().UnixMilli()
	if _, err = tx.ExecContext(r.Context(), `UPDATE admins SET status=?,updated_at=? WHERE id=?`, newStatus, now, id); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	if input.RoleIDs != nil {
		if _, err = tx.ExecContext(r.Context(), `DELETE FROM admin_role_members WHERE admin_id=?`, id); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
		for _, roleID := range roleIDs {
			if _, err = tx.ExecContext(r.Context(), `INSERT INTO admin_role_members(admin_id,role_id,created_at) VALUES(?,?,?)`, id, roleID, now); err != nil {
				writeError(w, 500, "database_error", err.Error())
				return
			}
		}
	}
	if newStatus == "disabled" {
		if _, err = tx.ExecContext(r.Context(), `DELETE FROM admin_sessions WHERE admin_id=?`, id); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
	}
	if err = tx.Commit(); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	s.audit(r, session.AdminID, session.Username, "administrator.update", "administrator", id, "success", map[string]any{"status": newStatus, "roleIds": roleIDs})
	writeJSON(w, 200, map[string]bool{"ok": true})
}

func (s *Server) adminResetAdministratorPassword(w http.ResponseWriter, r *http.Request) {
	session, ok := s.requirePermission(w, r, "admins.reset_password", true)
	if !ok {
		return
	}
	var input struct {
		Password string `json:"password"`
	}
	if !decodeJSON(w, r, &input) {
		return
	}
	if len(input.Password) < 10 {
		writeError(w, 400, "weak_password", "password must contain at least 10 characters")
		return
	}
	encoded, err := hashPassword(input.Password)
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
	result, err := tx.ExecContext(r.Context(), `UPDATE admins SET password_hash=?,updated_at=? WHERE id=?`, encoded, time.Now().UnixMilli(), r.PathValue("id"))
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	count, _ := result.RowsAffected()
	if count == 0 {
		writeError(w, 404, "administrator_not_found", "administrator was not found")
		return
	}
	if _, err = tx.ExecContext(r.Context(), `DELETE FROM admin_sessions WHERE admin_id=?`, r.PathValue("id")); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	if err = tx.Commit(); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	s.audit(r, session.AdminID, session.Username, "administrator.password_reset", "administrator", r.PathValue("id"), "success", nil)
	writeJSON(w, 200, map[string]bool{"ok": true})
}

func (s *Server) adminListPermissions(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.requirePermission(w, r, "roles.read", false); !ok {
		return
	}
	rows, err := s.store.DB.QueryContext(r.Context(), `SELECT code,name,group_name,description FROM admin_permissions ORDER BY group_name,name`)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer rows.Close()
	items := make([]adminPermission, 0)
	for rows.Next() {
		var item adminPermission
		if err := rows.Scan(&item.Code, &item.Name, &item.Group, &item.Description); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
		items = append(items, item)
	}
	writeJSON(w, 200, map[string]any{"items": items})
}

func (s *Server) adminListRoles(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.requireAnyPermission(w, r, []string{"roles.read", "admins.read", "admins.create", "admins.update"}, false); !ok {
		return
	}
	rows, err := s.store.DB.QueryContext(r.Context(), `
SELECT r.id,r.name,r.description,r.built_in,
       COALESCE(GROUP_CONCAT(DISTINCT rp.permission_code),''),
       (SELECT COUNT(*) FROM admin_role_members m WHERE m.role_id=r.id)
FROM admin_roles r
LEFT JOIN admin_role_permissions rp ON rp.role_id=r.id
GROUP BY r.id ORDER BY r.built_in DESC,r.name COLLATE NOCASE`)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer rows.Close()
	items := make([]adminRoleSummary, 0)
	for rows.Next() {
		var item adminRoleSummary
		var builtIn int
		var permissions string
		if err := rows.Scan(&item.ID, &item.Name, &item.Description, &builtIn, &permissions, &item.MemberCount); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
		item.BuiltIn = builtIn != 0
		item.Permissions = commaValues(permissions)
		items = append(items, item)
	}
	writeJSON(w, 200, map[string]any{"items": items})
}

func (s *Server) adminCreateRole(w http.ResponseWriter, r *http.Request) {
	session, ok := s.requirePermission(w, r, "roles.create", true)
	if !ok {
		return
	}
	var input struct {
		Name        string   `json:"name"`
		Description string   `json:"description"`
		Permissions []string `json:"permissions"`
	}
	if !decodeJSON(w, r, &input) {
		return
	}
	input.Name = strings.TrimSpace(input.Name)
	input.Description = strings.TrimSpace(input.Description)
	input.Permissions = uniqueStrings(input.Permissions)
	if input.Name == "" || len([]rune(input.Name)) > 64 || len([]rune(input.Description)) > 500 || len(input.Permissions) == 0 {
		writeError(w, 400, "invalid_role", "role name and at least one permission are required")
		return
	}
	tx, err := s.store.DB.BeginTx(r.Context(), nil)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer tx.Rollback()
	if err := validateReferences(tx, "admin_permissions", "code", input.Permissions); err != nil {
		writeError(w, 400, "invalid_permissions", err.Error())
		return
	}
	id := randomID()
	now := time.Now().UnixMilli()
	if _, err = tx.ExecContext(r.Context(), `INSERT INTO admin_roles(id,name,description,built_in,created_at,updated_at) VALUES(?,?,?,0,?,?)`, id, input.Name, input.Description, now, now); err != nil {
		writeError(w, 409, "role_conflict", "role name already exists")
		return
	}
	for _, permission := range input.Permissions {
		if _, err = tx.ExecContext(r.Context(), `INSERT INTO admin_role_permissions(role_id,permission_code) VALUES(?,?)`, id, permission); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
	}
	if err = tx.Commit(); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	s.audit(r, session.AdminID, session.Username, "role.create", "role", id, "success", map[string]any{"name": input.Name, "permissions": input.Permissions})
	writeJSON(w, 201, map[string]string{"id": id})
}

func (s *Server) adminUpdateRole(w http.ResponseWriter, r *http.Request) {
	session, ok := s.requirePermission(w, r, "roles.update", true)
	if !ok {
		return
	}
	id := r.PathValue("id")
	if id == superAdminRoleID {
		writeError(w, 409, "built_in_role", "the super administrator role cannot be changed")
		return
	}
	var input struct {
		Name        string   `json:"name"`
		Description string   `json:"description"`
		Permissions []string `json:"permissions"`
	}
	if !decodeJSON(w, r, &input) {
		return
	}
	input.Name = strings.TrimSpace(input.Name)
	input.Description = strings.TrimSpace(input.Description)
	input.Permissions = uniqueStrings(input.Permissions)
	if input.Name == "" || len([]rune(input.Name)) > 64 || len(input.Permissions) == 0 {
		writeError(w, 400, "invalid_role", "role name and at least one permission are required")
		return
	}
	tx, err := s.store.DB.BeginTx(r.Context(), nil)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer tx.Rollback()
	if err := validateReferences(tx, "admin_permissions", "code", input.Permissions); err != nil {
		writeError(w, 400, "invalid_permissions", err.Error())
		return
	}
	result, err := tx.ExecContext(r.Context(), `UPDATE admin_roles SET name=?,description=?,updated_at=? WHERE id=? AND built_in=0`, input.Name, input.Description, time.Now().UnixMilli(), id)
	if err != nil {
		writeError(w, 409, "role_conflict", "role name already exists")
		return
	}
	count, _ := result.RowsAffected()
	if count == 0 {
		writeError(w, 404, "role_not_found", "role was not found")
		return
	}
	if _, err = tx.ExecContext(r.Context(), `DELETE FROM admin_role_permissions WHERE role_id=?`, id); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	for _, permission := range input.Permissions {
		if _, err = tx.ExecContext(r.Context(), `INSERT INTO admin_role_permissions(role_id,permission_code) VALUES(?,?)`, id, permission); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
	}
	if err = tx.Commit(); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	s.audit(r, session.AdminID, session.Username, "role.update", "role", id, "success", map[string]any{"name": input.Name, "permissions": input.Permissions})
	writeJSON(w, 200, map[string]bool{"ok": true})
}

func (s *Server) adminDeleteRole(w http.ResponseWriter, r *http.Request) {
	session, ok := s.requirePermission(w, r, "roles.delete", true)
	if !ok {
		return
	}
	id := r.PathValue("id")
	if id == superAdminRoleID {
		writeError(w, 409, "built_in_role", "the super administrator role cannot be deleted")
		return
	}
	tx, err := s.store.DB.BeginTx(r.Context(), nil)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer tx.Rollback()
	var members int
	if err := tx.QueryRowContext(r.Context(), `SELECT COUNT(*) FROM admin_role_members WHERE role_id=?`, id).Scan(&members); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	if members > 0 {
		writeError(w, 409, "role_in_use", "assigned roles cannot be deleted")
		return
	}
	result, err := tx.ExecContext(r.Context(), `DELETE FROM admin_roles WHERE id=? AND built_in=0`, id)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	count, _ := result.RowsAffected()
	if count == 0 {
		writeError(w, 404, "role_not_found", "role was not found")
		return
	}
	if err = tx.Commit(); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	s.audit(r, session.AdminID, session.Username, "role.delete", "role", id, "success", nil)
	writeJSON(w, 200, map[string]bool{"ok": true})
}

func (s *Server) adminListAuditLogs(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.requirePermission(w, r, "audit.read", false); !ok {
		return
	}
	page := boundedInt(r.URL.Query().Get("page"), 1, 1, 100000)
	pageSize := boundedInt(r.URL.Query().Get("pageSize"), 50, 1, 100)
	where := []string{"1=1"}
	args := make([]any, 0)
	if username := strings.TrimSpace(r.URL.Query().Get("username")); username != "" {
		where = append(where, "username LIKE ? ESCAPE '\\' COLLATE NOCASE")
		args = append(args, "%"+escapeLike(username)+"%")
	}
	if action := strings.TrimSpace(r.URL.Query().Get("action")); action != "" {
		where = append(where, "action=?")
		args = append(args, action)
	}
	if result := strings.TrimSpace(r.URL.Query().Get("result")); result != "" {
		if result != "success" && result != "failure" && result != "denied" {
			writeError(w, 400, "invalid_result", "result must be success, failure, or denied")
			return
		}
		where = append(where, "result=?")
		args = append(args, result)
	}
	for _, item := range []struct {
		key      string
		operator string
	}{{"from", ">="}, {"to", "<="}} {
		if raw := r.URL.Query().Get(item.key); raw != "" {
			value, err := strconv.ParseInt(raw, 10, 64)
			if err != nil {
				writeError(w, 400, "invalid_time", item.key+" must be a Unix millisecond timestamp")
				return
			}
			where = append(where, "created_at"+item.operator+"?")
			args = append(args, value)
		}
	}
	clause := strings.Join(where, " AND ")
	var total int
	if err := s.store.DB.QueryRowContext(r.Context(), `SELECT COUNT(*) FROM admin_audit_logs WHERE `+clause, args...).Scan(&total); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	listArgs := append(append([]any{}, args...), pageSize, (page-1)*pageSize)
	rows, err := s.store.DB.QueryContext(r.Context(), `SELECT id,admin_id,username,action,resource_type,resource_id,result,client_address,metadata,created_at FROM admin_audit_logs WHERE `+clause+` ORDER BY created_at DESC,id DESC LIMIT ? OFFSET ?`, listArgs...)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer rows.Close()
	items := make([]map[string]any, 0)
	for rows.Next() {
		var id, createdAt int64
		var adminID sql.NullString
		var username, action, resourceType, resourceID, result, address, metadata string
		if err := rows.Scan(&id, &adminID, &username, &action, &resourceType, &resourceID, &result, &address, &metadata, &createdAt); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
		var decoded any
		if err := json.Unmarshal([]byte(metadata), &decoded); err != nil {
			decoded = map[string]any{}
		}
		items = append(items, map[string]any{"id": id, "adminId": nullableString(adminID), "username": username, "action": action, "resourceType": resourceType, "resourceId": resourceID, "result": result, "clientAddress": address, "metadata": decoded, "createdAt": createdAt})
	}
	writeJSON(w, 200, map[string]any{"items": items, "page": page, "pageSize": pageSize, "total": total})
}

func nullableString(value sql.NullString) any {
	if value.Valid {
		return value.String
	}
	return nil
}
