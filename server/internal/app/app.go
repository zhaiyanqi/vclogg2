package app

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"database/sql"
	"embed"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/google/uuid"
	"golang.org/x/crypto/argon2"

	"vclogg/server/internal/config"
	"vclogg/server/internal/store"
)

//go:embed admin
var adminAssets embed.FS

const (
	maxBodyBytes                 = 512 * 1024
	adminCookie                  = "vclogg_admin_session"
	clientCookie                 = "vclogg_client_session"
	filterUUIDBranchesCapability = "filter-uuid-branches-v1"
)

var semverPattern = regexp.MustCompile(`^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*))*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$`)
var windowsUpdateArtifactPattern = regexp.MustCompile(`^VCLogg-(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*))*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?-Setup-x64\.exe(?:\.blockmap)?$`)
var vclogg2WindowsUpdateArtifactPattern = regexp.MustCompile(`^vclogg2-(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*))*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?-win-x64(?:\.zip|\.blockmap\.json)$`)

type Server struct {
	store            *store.Store
	mux              *http.ServeMux
	trustedProxy     bool
	config           config.Config
	clientPublicKeys map[string]ed25519.PublicKey
	releaseMu        sync.Mutex
}

type Identity struct {
	ID          string `json:"id"`
	DisplayName string `json:"displayName"`
}

type CloudFilter struct {
	ID            string `json:"id"`
	Revision      int    `json:"revision"`
	Name          string `json:"name"`
	Value         string `json:"value"`
	UseRegex      bool   `json:"useRegex"`
	Note          string `json:"note"`
	OwnerID       string `json:"ownerId"`
	OwnerName     string `json:"ownerName"`
	LikeCount     int    `json:"likeCount"`
	DownloadCount int    `json:"downloadCount"`
	Liked         bool   `json:"liked"`
	UpdatedAt     int64  `json:"updatedAt"`
	Collaborative bool   `json:"collaborative"`
	CanEdit       bool   `json:"canEdit"`
	CanDelete     bool   `json:"canDelete"`
}

type CloudFilterRevisionSummary struct {
	Revision      int    `json:"revision"`
	Collaborative bool   `json:"collaborative"`
	EditorID      string `json:"editorId"`
	EditorName    string `json:"editorName"`
	EditorRole    string `json:"editorRole"`
	CreatedAt     int64  `json:"createdAt"`
	Current       bool   `json:"current"`
}

type CloudFilterRevision struct {
	FilterID      string `json:"filterId"`
	Revision      int    `json:"revision"`
	Name          string `json:"name"`
	Value         string `json:"value"`
	UseRegex      bool   `json:"useRegex"`
	Note          string `json:"note"`
	Collaborative bool   `json:"collaborative"`
	OwnerID       string `json:"ownerId"`
	OwnerName     string `json:"ownerName"`
	EditorID      string `json:"editorId"`
	EditorName    string `json:"editorName"`
	EditorRole    string `json:"editorRole"`
	CreatedAt     int64  `json:"createdAt"`
	Current       bool   `json:"current"`
}

func New(s *store.Store) *Server {
	return NewWithTrustedProxy(s, false)
}

func NewWithTrustedProxy(s *store.Store, trustedProxy bool) *Server {
	cfg := config.Default()
	cfg.TrustedProxy = trustedProxy
	cfg.ClientAuth.Enabled = false
	server, err := NewConfigured(s, cfg)
	if err != nil {
		panic(err)
	}
	return server
}

func NewConfigured(s *store.Store, cfg config.Config) (*Server, error) {
	publicKeys, err := cfg.ClientPublicKeys()
	if err != nil {
		return nil, err
	}
	server := &Server{
		store:            s,
		mux:              http.NewServeMux(),
		trustedProxy:     cfg.TrustedProxy,
		config:           cfg,
		clientPublicKeys: publicKeys,
	}
	server.routes()
	return server, nil
}

func (s *Server) Handler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("X-Content-Type-Options", "nosniff")
		w.Header().Set("X-Frame-Options", "DENY")
		w.Header().Set("Referrer-Policy", "no-referrer")
		w.Header().Set("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self'; style-src-attr 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; font-src 'self' data:; frame-ancestors 'none'")
		if strings.HasPrefix(r.URL.Path, "/api/v1/") {
			w.Header().Set("Cache-Control", "no-store")
			if !s.prepareClientRequest(w, r) {
				return
			}
			requiresSignature := r.URL.Path == "/api/v1/identities/register"
			if r.URL.Path == "/api/v1/app-version" {
				authenticated, err := s.hasValidClientSession(r)
				if err != nil {
					writeError(w, http.StatusInternalServerError, "database_error", err.Error())
					return
				}
				requiresSignature = !authenticated
			}
			if requiresSignature && !s.authenticateClientRequest(w, r) {
				return
			}
		}
		s.mux.ServeHTTP(w, r)
	})
}

func (s *Server) routes() {
	s.mux.HandleFunc("GET /health", func(w http.ResponseWriter, _ *http.Request) { writeJSON(w, 200, map[string]bool{"ok": true}) })
	if s.config.Updates.Enabled {
		s.mux.HandleFunc("GET /updates/win-x64/{name}", s.serveWindowsUpdate)
		s.mux.HandleFunc("GET /updates-vclogg2/win-x64/{name}", s.serveVCLogg2WindowsUpdate)
	}
	s.mux.HandleFunc("GET /api/v1/app-version", s.getAppVersion)
	s.mux.HandleFunc("POST /api/v1/identities/register", s.registerIdentity)
	s.mux.HandleFunc("POST /api/v1/session/logout", s.logoutIdentity)
	s.mux.HandleFunc("GET /api/v1/me", s.getMe)
	s.mux.HandleFunc("PATCH /api/v1/me", s.updateMe)
	s.mux.HandleFunc("GET /api/v1/filters", s.listFilters)
	s.mux.HandleFunc("POST /api/v1/filters/batch", s.shareFilters)
	s.mux.HandleFunc("GET /api/v1/filters/{id}", s.getFilter)
	s.mux.HandleFunc("GET /api/v1/filters/{id}/revisions", s.listFilterRevisions)
	s.mux.HandleFunc("GET /api/v1/filters/{id}/revisions/{revision}", s.getFilterRevision)
	s.mux.HandleFunc("PATCH /api/v1/filters/{id}", s.updateFilter)
	s.mux.HandleFunc("DELETE /api/v1/filters/{id}", s.deleteOwnFilter)
	s.mux.HandleFunc("POST /api/v1/downloads/batch", s.recordDownloads)
	s.mux.HandleFunc("PUT /api/v1/filters/{id}/like", s.likeFilter)
	s.mux.HandleFunc("DELETE /api/v1/filters/{id}/like", s.unlikeFilter)

	s.mux.HandleFunc("POST /admin/api/login", s.adminLogin)
	s.mux.HandleFunc("POST /admin/api/logout", s.adminLogout)
	s.mux.HandleFunc("PUT /admin/api/password", s.adminChangePassword)
	s.mux.HandleFunc("GET /admin/api/session", s.adminSessionInfo)
	s.mux.HandleFunc("GET /admin/api/dashboard", s.adminDashboard)
	s.mux.HandleFunc("GET /admin/api/settings/version", s.adminGetVersion)
	s.mux.HandleFunc("PUT /admin/api/settings/version", s.adminSetVersion)
	s.mux.HandleFunc("GET /admin/api/settings/release", s.adminGetRelease)
	s.mux.HandleFunc("POST /admin/api/settings/release", s.adminPublishRelease)
	s.mux.HandleFunc("GET /admin/api/settings/releases/{product}", s.adminGetProductRelease)
	s.mux.HandleFunc("POST /admin/api/settings/releases/{product}", s.adminPublishProductRelease)
	s.mux.HandleFunc("GET /admin/api/filters", s.adminListFilters)
	s.mux.HandleFunc("GET /admin/api/filters/export", s.adminExportFilters)
	s.mux.HandleFunc("PATCH /admin/api/filters/{id}", s.adminUpdateFilter)
	s.mux.HandleFunc("GET /admin/api/identities", s.adminListIdentities)
	s.mux.HandleFunc("PATCH /admin/api/identities/{id}", s.adminUpdateIdentity)
	s.mux.HandleFunc("POST /admin/api/backup", s.adminBackup)
	s.mux.HandleFunc("GET /admin/api/administrators", s.adminListAdministrators)
	s.mux.HandleFunc("POST /admin/api/administrators", s.adminCreateAdministrator)
	s.mux.HandleFunc("PATCH /admin/api/administrators/{id}", s.adminUpdateAdministrator)
	s.mux.HandleFunc("PUT /admin/api/administrators/{id}/password", s.adminResetAdministratorPassword)
	s.mux.HandleFunc("GET /admin/api/permissions", s.adminListPermissions)
	s.mux.HandleFunc("GET /admin/api/roles", s.adminListRoles)
	s.mux.HandleFunc("POST /admin/api/roles", s.adminCreateRole)
	s.mux.HandleFunc("PUT /admin/api/roles/{id}", s.adminUpdateRole)
	s.mux.HandleFunc("DELETE /admin/api/roles/{id}", s.adminDeleteRole)
	s.mux.HandleFunc("GET /admin/api/audit", s.adminListAuditLogs)
	adminFS, err := fs.Sub(adminAssets, "admin")
	if err != nil {
		panic(err)
	}
	s.mux.Handle("GET /admin/", adminWebHandler(adminFS))
	s.mux.HandleFunc("GET /admin", func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, "/admin/", http.StatusTemporaryRedirect)
	})
}

func (s *Server) serveWindowsUpdate(w http.ResponseWriter, r *http.Request) {
	serveUpdateFile(w, r, s.config.Updates.Directory, "latest.yml", "application/yaml; charset=utf-8", windowsUpdateArtifactPattern)
}

func (s *Server) serveVCLogg2WindowsUpdate(w http.ResponseWriter, r *http.Request) {
	serveUpdateFile(w, r, s.config.Updates.VCLogg2Directory, "latest.json", "application/json; charset=utf-8", vclogg2WindowsUpdateArtifactPattern)
}

func serveUpdateFile(w http.ResponseWriter, r *http.Request, directory, manifestName, manifestContentType string, artifactPattern *regexp.Regexp) {
	name := r.PathValue("name")
	if name != manifestName && !artifactPattern.MatchString(name) {
		http.NotFound(w, r)
		return
	}
	filePath := filepath.Join(directory, "win-x64", name)
	file, err := os.Open(filePath)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			http.NotFound(w, r)
			return
		}
		writeError(w, http.StatusInternalServerError, "update_file_error", "update file is unavailable")
		return
	}
	defer file.Close()
	info, err := file.Stat()
	if err != nil || !info.Mode().IsRegular() {
		http.NotFound(w, r)
		return
	}
	if name == manifestName {
		w.Header().Set("Cache-Control", "no-store")
		w.Header().Set("Content-Type", manifestContentType)
	} else {
		w.Header().Set("Cache-Control", "public, max-age=31536000, immutable")
		w.Header().Set("Content-Type", "application/octet-stream")
	}
	_ = http.NewResponseController(w).SetWriteDeadline(time.Now().Add(15 * time.Minute))
	http.ServeContent(w, r, name, info.ModTime(), file)
}

func adminWebHandler(assets fs.FS) http.Handler {
	files := http.StripPrefix("/admin/", http.FileServer(http.FS(assets)))
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		relative := strings.TrimPrefix(r.URL.Path, "/admin/")
		if strings.HasPrefix(relative, "api/") {
			http.NotFound(w, r)
			return
		}
		if relative != "" {
			if info, err := fs.Stat(assets, relative); err == nil {
				if info.IsDir() {
					http.NotFound(w, r)
					return
				}
				if strings.Contains(relative, "/assets/") || strings.HasPrefix(relative, "assets/") {
					w.Header().Set("Cache-Control", "public, max-age=31536000, immutable")
				}
				files.ServeHTTP(w, r)
				return
			}
			if strings.Contains(filepath.Base(relative), ".") {
				http.NotFound(w, r)
				return
			}
		}
		index, err := fs.ReadFile(assets, "index.html")
		if err != nil {
			http.NotFound(w, r)
			return
		}
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		w.Header().Set("Cache-Control", "no-store")
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write(index)
	})
}

func (s *Server) getAppVersion(w http.ResponseWriter, r *http.Request) {
	var version string
	var updated int64
	err := s.store.DB.QueryRowContext(r.Context(), `SELECT value, updated_at FROM settings WHERE key='latest_app_version'`).Scan(&version, &updated)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	writeJSON(w, 200, map[string]any{"latestVersion": version, "updatedAt": updated})
}

func (s *Server) registerIdentity(w http.ResponseWriter, r *http.Request) {
	var input struct {
		DisplayName string `json:"displayName"`
		ClientUUID  string `json:"uuid"`
		Token       string `json:"token"`
	}
	if !decodeJSON(w, r, &input) {
		return
	}
	input.DisplayName = strings.TrimSpace(input.DisplayName)
	if len([]rune(input.DisplayName)) < 1 || len([]rune(input.DisplayName)) > 64 {
		writeError(w, 400, "invalid_display_name", "displayName must contain 1-64 characters")
		return
	}
	identityID := ""
	identityKey := input.Token
	if input.ClientUUID != "" {
		parsed, err := uuid.Parse(input.ClientUUID)
		if err != nil || parsed.Version() != 4 {
			writeError(w, 400, "invalid_uuid", "uuid must be a valid version 4 UUID")
			return
		}
		identityID = parsed.String()
		identityKey = identityID
	} else if !validClientToken(input.Token) {
		writeError(w, 400, "invalid_token", "uuid or a legacy 32-byte base64url token is required")
		return
	}
	hash := tokenHash(identityKey)
	now := time.Now().UnixMilli()
	var identity Identity
	lookup := `SELECT id, display_name FROM identities WHERE token_hash=?`
	lookupValue := any(hash)
	if identityID != "" {
		lookup = `SELECT id, display_name FROM identities WHERE id=?`
		lookupValue = identityID
	}
	err := s.store.DB.QueryRowContext(r.Context(), lookup, lookupValue).Scan(&identity.ID, &identity.DisplayName)
	if errors.Is(err, sql.ErrNoRows) {
		if identityID == "" {
			identityID = randomID()
		}
		identity = Identity{ID: identityID, DisplayName: input.DisplayName}
		_, err = s.store.DB.ExecContext(r.Context(), `INSERT INTO identities(id, display_name, token_hash, created_at, last_seen_at) VALUES(?,?,?,?,?)`, identity.ID, identity.DisplayName, hash, now, now)
	} else if err == nil {
		if identity.DisplayName != input.DisplayName {
			identity.DisplayName = input.DisplayName
		}
		_, err = s.store.DB.ExecContext(r.Context(), `UPDATE identities SET display_name=?, last_seen_at=? WHERE id=? AND revoked_at IS NULL`, identity.DisplayName, now, identity.ID)
	}
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	var revoked sql.NullInt64
	if err := s.store.DB.QueryRowContext(r.Context(), `SELECT revoked_at FROM identities WHERE id=?`, identity.ID).Scan(&revoked); err != nil || revoked.Valid {
		writeError(w, 401, "token_revoked", "this access token has been revoked")
		return
	}
	csrf, ok := s.issueClientSession(w, r, identity.ID)
	if !ok {
		return
	}
	writeJSON(w, 200, map[string]any{
		"id":           identity.ID,
		"displayName":  identity.DisplayName,
		"csrfToken":    csrf,
		"capabilities": []string{filterUUIDBranchesCapability},
	})
}

func (s *Server) getMe(w http.ResponseWriter, r *http.Request) {
	identity, ok := s.requireIdentity(w, r)
	if !ok {
		return
	}
	writeJSON(w, 200, map[string]any{
		"id":           identity.ID,
		"displayName":  identity.DisplayName,
		"capabilities": []string{filterUUIDBranchesCapability},
	})
}

func (s *Server) updateMe(w http.ResponseWriter, r *http.Request) {
	identity, ok := s.requireIdentity(w, r)
	if !ok {
		return
	}
	var input struct {
		DisplayName string `json:"displayName"`
	}
	if !decodeJSON(w, r, &input) {
		return
	}
	input.DisplayName = strings.TrimSpace(input.DisplayName)
	if len([]rune(input.DisplayName)) < 1 || len([]rune(input.DisplayName)) > 64 {
		writeError(w, 400, "invalid_display_name", "displayName must contain 1-64 characters")
		return
	}
	if _, err := s.store.DB.ExecContext(r.Context(), `UPDATE identities SET display_name=? WHERE id=?`, input.DisplayName, identity.ID); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	identity.DisplayName = input.DisplayName
	writeJSON(w, 200, identity)
}

func (s *Server) listFilters(w http.ResponseWriter, r *http.Request) {
	identity, ok := s.requireIdentity(w, r)
	if !ok {
		return
	}
	query := strings.TrimSpace(r.URL.Query().Get("q"))
	if len([]rune(query)) > 100 {
		writeError(w, 400, "invalid_query", "q is too long")
		return
	}
	sortName := r.URL.Query().Get("sort")
	order := "f.updated_at DESC"
	if sortName == "downloads" {
		order = "f.download_count DESC, f.updated_at DESC"
	} else if sortName == "likes" {
		order = "f.like_count DESC, f.updated_at DESC"
	} else if sortName != "" && sortName != "newest" {
		writeError(w, 400, "invalid_sort", "sort must be newest, downloads, or likes")
		return
	}
	page := boundedInt(r.URL.Query().Get("page"), 1, 1, 100000)
	pageSize := boundedInt(r.URL.Query().Get("pageSize"), 30, 1, 100)
	pattern := "%" + escapeLike(query) + "%"
	where := `f.status='active' AND (?='' OR r.name LIKE ? ESCAPE '\' COLLATE NOCASE OR r.value LIKE ? ESCAPE '\' COLLATE NOCASE OR r.note LIKE ? ESCAPE '\' COLLATE NOCASE OR i.display_name LIKE ? ESCAPE '\' COLLATE NOCASE)`
	args := []any{query, pattern, pattern, pattern, pattern}
	var total int
	countSQL := `SELECT COUNT(*) FROM filters f JOIN filter_revisions r ON r.filter_id=f.id AND r.revision=f.current_revision JOIN identities i ON i.id=f.owner_id WHERE ` + where
	if err := s.store.DB.QueryRowContext(r.Context(), countSQL, args...).Scan(&total); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	listSQL := `SELECT f.id, f.current_revision, r.name, r.value, r.use_regex, r.note, i.id, i.display_name, f.like_count, f.download_count, EXISTS(SELECT 1 FROM filter_likes l WHERE l.filter_id=f.id AND l.identity_id=?), f.updated_at, r.collaborative, (f.owner_id=? OR r.collaborative=1), f.owner_id=? FROM filters f JOIN filter_revisions r ON r.filter_id=f.id AND r.revision=f.current_revision JOIN identities i ON i.id=f.owner_id WHERE ` + where + ` ORDER BY ` + order + ` LIMIT ? OFFSET ?`
	listArgs := append([]any{identity.ID, identity.ID, identity.ID}, args...)
	listArgs = append(listArgs, pageSize, (page-1)*pageSize)
	rows, err := s.store.DB.QueryContext(r.Context(), listSQL, listArgs...)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer rows.Close()
	items := make([]CloudFilter, 0)
	for rows.Next() {
		var item CloudFilter
		var useRegex, liked, collaborative, canEdit, canDelete int
		if err := rows.Scan(&item.ID, &item.Revision, &item.Name, &item.Value, &useRegex, &item.Note, &item.OwnerID, &item.OwnerName, &item.LikeCount, &item.DownloadCount, &liked, &item.UpdatedAt, &collaborative, &canEdit, &canDelete); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
		item.UseRegex = useRegex != 0
		item.Liked = liked != 0
		item.Collaborative = collaborative != 0
		item.CanEdit = canEdit != 0
		item.CanDelete = canDelete != 0
		items = append(items, item)
	}
	writeJSON(w, 200, map[string]any{"items": items, "page": page, "pageSize": pageSize, "total": total})
}

func (s *Server) getFilter(w http.ResponseWriter, r *http.Request) {
	identity, ok := s.requireIdentity(w, r)
	if !ok {
		return
	}
	filterID := r.PathValue("id")
	if !validCloudFilterID(filterID) {
		writeError(w, 400, "invalid_filter_id", "filter id is invalid")
		return
	}
	var item CloudFilter
	var useRegex, liked, collaborative, canEdit, canDelete int
	err := s.store.DB.QueryRowContext(r.Context(), `
SELECT f.id,f.current_revision,r.name,r.value,r.use_regex,r.note,
       i.id,i.display_name,f.like_count,f.download_count,
       EXISTS(SELECT 1 FROM filter_likes l WHERE l.filter_id=f.id AND l.identity_id=?),
       f.updated_at,r.collaborative,(f.owner_id=? OR r.collaborative=1),f.owner_id=?
FROM filters f
JOIN filter_revisions r ON r.filter_id=f.id AND r.revision=f.current_revision
JOIN identities i ON i.id=f.owner_id
WHERE f.id=? AND f.status='active'`, identity.ID, identity.ID, identity.ID, filterID).Scan(
		&item.ID, &item.Revision, &item.Name, &item.Value, &useRegex, &item.Note,
		&item.OwnerID, &item.OwnerName, &item.LikeCount, &item.DownloadCount,
		&liked, &item.UpdatedAt, &collaborative, &canEdit, &canDelete,
	)
	if errors.Is(err, sql.ErrNoRows) {
		writeError(w, 404, "filter_not_found", "an active filter was not found")
		return
	}
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	item.UseRegex = useRegex != 0
	item.Liked = liked != 0
	item.Collaborative = collaborative != 0
	item.CanEdit = canEdit != 0
	item.CanDelete = canDelete != 0
	writeJSON(w, 200, item)
}

func (s *Server) listFilterRevisions(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.requireIdentity(w, r); !ok {
		return
	}
	filterID := r.PathValue("id")
	if !validCloudFilterID(filterID) {
		writeError(w, 400, "invalid_filter_id", "filter id is invalid")
		return
	}
	page := boundedInt(r.URL.Query().Get("page"), 1, 1, 100000)
	pageSize := boundedInt(r.URL.Query().Get("pageSize"), 30, 1, 100)
	var total int
	if err := s.store.DB.QueryRowContext(r.Context(), `SELECT COUNT(*) FROM filters f JOIN filter_revisions r ON r.filter_id=f.id WHERE f.id=? AND f.status='active'`, filterID).Scan(&total); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	if total == 0 {
		writeError(w, 404, "filter_not_found", "an active filter was not found")
		return
	}
	rows, err := s.store.DB.QueryContext(r.Context(), `
SELECT r.revision,r.collaborative,r.editor_id,
       CASE WHEN r.editor_type='admin' THEN COALESCE(a.username,'管理员') ELSE COALESCE(i.display_name,'未知用户') END,
       r.editor_type,r.created_at,r.revision=f.current_revision
FROM filters f
JOIN filter_revisions r ON r.filter_id=f.id
LEFT JOIN identities i ON r.editor_type<>'admin' AND i.id=r.editor_id
LEFT JOIN admins a ON r.editor_type='admin' AND a.id=r.editor_id
WHERE f.id=? AND f.status='active'
ORDER BY r.revision DESC LIMIT ? OFFSET ?`, filterID, pageSize, (page-1)*pageSize)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer rows.Close()
	items := make([]CloudFilterRevisionSummary, 0)
	for rows.Next() {
		var item CloudFilterRevisionSummary
		var collaborative, current int
		if err := rows.Scan(&item.Revision, &collaborative, &item.EditorID, &item.EditorName, &item.EditorRole, &item.CreatedAt, &current); err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
		item.Collaborative = collaborative != 0
		item.Current = current != 0
		items = append(items, item)
	}
	writeJSON(w, 200, map[string]any{"items": items, "page": page, "pageSize": pageSize, "total": total})
}

func (s *Server) getFilterRevision(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.requireIdentity(w, r); !ok {
		return
	}
	filterID := r.PathValue("id")
	revision, err := strconv.Atoi(r.PathValue("revision"))
	if !validCloudFilterID(filterID) || err != nil || revision < 1 {
		writeError(w, 400, "invalid_filter_revision", "filter id or revision is invalid")
		return
	}
	var item CloudFilterRevision
	var useRegex, collaborative, current int
	err = s.store.DB.QueryRowContext(r.Context(), `
SELECT f.id,r.revision,r.name,r.value,r.use_regex,r.note,r.collaborative,
       owner.id,owner.display_name,r.editor_id,
       CASE WHEN r.editor_type='admin' THEN COALESCE(a.username,'管理员') ELSE COALESCE(editor.display_name,'未知用户') END,
       r.editor_type,r.created_at,r.revision=f.current_revision
FROM filters f
JOIN filter_revisions r ON r.filter_id=f.id
JOIN identities owner ON owner.id=f.owner_id
LEFT JOIN identities editor ON r.editor_type<>'admin' AND editor.id=r.editor_id
LEFT JOIN admins a ON r.editor_type='admin' AND a.id=r.editor_id
WHERE f.id=? AND f.status='active' AND r.revision=?`, filterID, revision).Scan(
		&item.FilterID, &item.Revision, &item.Name, &item.Value, &useRegex, &item.Note,
		&collaborative, &item.OwnerID, &item.OwnerName, &item.EditorID, &item.EditorName,
		&item.EditorRole, &item.CreatedAt, &current,
	)
	if errors.Is(err, sql.ErrNoRows) {
		writeError(w, 404, "filter_revision_not_found", "the requested filter revision was not found")
		return
	}
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	item.UseRegex = useRegex != 0
	item.Collaborative = collaborative != 0
	item.Current = current != 0
	writeJSON(w, 200, item)
}

type shareItem struct {
	ClientFilterID      string `json:"clientFilterId"`
	Name                string `json:"name"`
	Value               string `json:"value"`
	UseRegex            bool   `json:"useRegex"`
	Note                string `json:"note"`
	DerivedFromFilterID string `json:"derivedFromFilterId,omitempty"`
	Collaborative       *bool  `json:"collaborative,omitempty"`
	BaseRevision        *int   `json:"baseRevision,omitempty"`
}

func (s *Server) shareFilters(w http.ResponseWriter, r *http.Request) {
	identity, ok := s.requireIdentity(w, r)
	if !ok {
		return
	}
	var input struct {
		Items []shareItem `json:"items"`
	}
	if !decodeJSON(w, r, &input) {
		return
	}
	if len(input.Items) < 1 || len(input.Items) > s.config.Uploads.MaxItemsPerRequest {
		writeError(w, 400, "invalid_batch", fmt.Sprintf("items must contain 1-%d filters", s.config.Uploads.MaxItemsPerRequest))
		return
	}
	for i := range input.Items {
		if message := validateShareItem(&input.Items[i]); message != "" {
			writeError(w, 400, "invalid_filter", fmt.Sprintf("item %d: %s", i+1, message))
			return
		}
	}
	tx, err := s.store.DB.BeginTx(r.Context(), nil)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer tx.Rollback()
	results := make([]map[string]any, 0, len(input.Items))
	now := time.Now().UnixMilli()
	if !s.consumeUploadRequest(w, r, tx, identity.ID, now) {
		return
	}
	for _, item := range input.Items {
		var filterID, ownerID, status string
		var revision int
		canonicalID := ""
		if parsed, parseErr := uuid.Parse(item.ClientFilterID); parseErr == nil {
			canonicalID = parsed.String()
			err = tx.QueryRowContext(r.Context(), `SELECT id,owner_id,status,current_revision FROM filters WHERE id=?`, canonicalID).Scan(&filterID, &ownerID, &status, &revision)
		} else {
			err = tx.QueryRowContext(r.Context(), `SELECT id,owner_id,status,current_revision FROM filters WHERE owner_id=? AND client_filter_id=?`, identity.ID, item.ClientFilterID).Scan(&filterID, &ownerID, &status, &revision)
		}
		if errors.Is(err, sql.ErrNoRows) {
			filterID = ""
		}
		if err == nil && ownerID != identity.ID {
			writeError(w, http.StatusConflict, "filter_identity_conflict", "the filter UUID belongs to another owner")
			return
		}
		if errors.Is(err, sql.ErrNoRows) {
			filterID = canonicalID
			if filterID == "" {
				filterID = uuid.NewString()
			}
			revision = 1
			var derived any
			if derivedID, parseErr := uuid.Parse(item.DerivedFromFilterID); parseErr == nil {
				var exists int
				if queryErr := tx.QueryRowContext(r.Context(), `SELECT EXISTS(SELECT 1 FROM filters WHERE id=?)`, derivedID.String()).Scan(&exists); queryErr != nil {
					writeError(w, 500, "database_error", queryErr.Error())
					return
				}
				if exists != 0 {
					derived = derivedID.String()
				}
			}
			_, err = tx.ExecContext(r.Context(), `INSERT INTO filters(id,owner_id,client_filter_id,derived_from_filter_id,current_revision,created_at,updated_at) VALUES(?,?,?,?,?,?,?)`, filterID, identity.ID, item.ClientFilterID, derived, revision, now, now)
			if err == nil {
				_, err = tx.ExecContext(r.Context(), `INSERT INTO filter_revisions(filter_id,revision,name,value,use_regex,note,collaborative,editor_type,editor_id,created_at) VALUES(?,?,?,?,?,?,?,?,?,?)`, filterID, revision, item.Name, item.Value, boolInt(item.UseRegex), item.Note, boolInt(item.Collaborative != nil && *item.Collaborative), "owner", identity.ID, now)
			}
		} else if err == nil {
			if status != "active" && status != "deleted" {
				writeError(w, 409, "filter_inactive", "an inactive shared filter cannot be updated")
				return
			}
			reactivating := status == "deleted"
			var name, value, note string
			var useRegex, collaborative int
			err = tx.QueryRowContext(r.Context(), `SELECT name,value,use_regex,note,collaborative FROM filter_revisions WHERE filter_id=? AND revision=?`, filterID, revision).Scan(&name, &value, &useRegex, &note, &collaborative)
			nextCollaborative := collaborative
			if item.Collaborative != nil {
				nextCollaborative = boolInt(*item.Collaborative)
			}
			changed := name != item.Name || value != item.Value || useRegex != boolInt(item.UseRegex) || note != item.Note || collaborative != nextCollaborative
			if err == nil && !changed && !reactivating {
				writeError(w, 409, "no_changes", "the shared filter is unchanged")
				return
			}
			// Deletion unlinks the client's baseline. The owner may explicitly
			// republish that UUID without a baseline; active updates still require it.
			if err == nil && canonicalID != "" && item.BaseRevision == nil && !reactivating {
				writeRevisionConflict(w, revision)
				return
			}
			if err == nil && item.BaseRevision != nil && *item.BaseRevision != revision {
				writeRevisionConflict(w, revision)
				return
			}
			if err == nil && changed {
				revision++
				_, err = tx.ExecContext(r.Context(), `INSERT INTO filter_revisions(filter_id,revision,name,value,use_regex,note,collaborative,editor_type,editor_id,created_at) VALUES(?,?,?,?,?,?,?,?,?,?)`, filterID, revision, item.Name, item.Value, boolInt(item.UseRegex), item.Note, nextCollaborative, "owner", identity.ID, now)
				if err == nil {
					_, err = tx.ExecContext(r.Context(), `UPDATE filters SET current_revision=?,status='active',updated_at=? WHERE id=?`, revision, now, filterID)
				}
			} else if err == nil && reactivating {
				_, err = tx.ExecContext(r.Context(), `UPDATE filters SET status='active',updated_at=? WHERE id=?`, now, filterID)
			}
		}
		if err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
		results = append(results, map[string]any{"clientFilterId": item.ClientFilterID, "filterId": filterID, "revision": revision, "note": item.Note})
	}
	if !s.checkUploadQuota(w, r, tx, identity.ID) {
		return
	}
	if err = tx.Commit(); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	writeJSON(w, 200, map[string]any{"items": results})
}

type filterUpdate struct {
	Name          string `json:"name"`
	Value         string `json:"value"`
	UseRegex      bool   `json:"useRegex"`
	Note          string `json:"note"`
	Collaborative *bool  `json:"collaborative,omitempty"`
	BaseRevision  *int   `json:"baseRevision,omitempty"`
}

func (s *Server) updateFilter(w http.ResponseWriter, r *http.Request) {
	identity, ok := s.requireIdentity(w, r)
	if !ok {
		return
	}
	filterID := r.PathValue("id")
	if !validCloudFilterID(filterID) {
		writeError(w, 400, "invalid_filter_id", "filter id is invalid")
		return
	}
	var input filterUpdate
	if !decodeJSON(w, r, &input) {
		return
	}
	candidate := shareItem{Name: input.Name, Value: input.Value, UseRegex: input.UseRegex, Note: input.Note, ClientFilterID: "edit"}
	if message := validateShareItem(&candidate); message != "" {
		writeError(w, 400, "invalid_filter", message)
		return
	}
	tx, err := s.store.DB.BeginTx(r.Context(), nil)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer tx.Rollback()
	var revision int
	var name, value, note, ownerID string
	var useRegex, collaborative int
	err = tx.QueryRowContext(r.Context(), `SELECT f.current_revision,r.name,r.value,r.use_regex,r.note,f.owner_id,r.collaborative FROM filters f JOIN filter_revisions r ON r.filter_id=f.id AND r.revision=f.current_revision WHERE f.id=? AND f.status='active'`, filterID).Scan(&revision, &name, &value, &useRegex, &note, &ownerID, &collaborative)
	if errors.Is(err, sql.ErrNoRows) {
		writeError(w, 404, "filter_not_found", "an active filter was not found")
		return
	}
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	isOwner := ownerID == identity.ID
	if !isOwner && collaborative == 0 {
		writeError(w, 403, "filter_not_editable", "this filter is not open for collaboration")
		return
	}
	if !isOwner && input.Collaborative != nil {
		writeError(w, 403, "collaboration_owner_only", "only the uploader can change collaboration")
		return
	}
	if input.BaseRevision != nil && *input.BaseRevision != revision {
		writeRevisionConflict(w, revision)
		return
	}
	nextCollaborative := collaborative
	if input.Collaborative != nil {
		nextCollaborative = boolInt(*input.Collaborative)
	}
	changed := name != candidate.Name || value != candidate.Value || useRegex != boolInt(candidate.UseRegex) || note != candidate.Note || collaborative != nextCollaborative
	if !changed {
		writeError(w, 409, "no_changes", "the filter is unchanged")
		return
	}
	revision++
	now := time.Now().UnixMilli()
	editorType := "collaborator"
	if isOwner {
		editorType = "owner"
	}
	if _, err = tx.ExecContext(r.Context(), `INSERT INTO filter_revisions(filter_id,revision,name,value,use_regex,note,collaborative,editor_type,editor_id,created_at) VALUES(?,?,?,?,?,?,?,?,?,?)`, filterID, revision, candidate.Name, candidate.Value, boolInt(candidate.UseRegex), candidate.Note, nextCollaborative, editorType, identity.ID, now); err == nil {
		_, err = tx.ExecContext(r.Context(), `UPDATE filters SET current_revision=?,updated_at=? WHERE id=?`, revision, now, filterID)
	}
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	if err = tx.Commit(); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	writeJSON(w, 200, map[string]any{"filterId": filterID, "revision": revision, "note": candidate.Note, "collaborative": nextCollaborative != 0})
}

func (s *Server) deleteOwnFilter(w http.ResponseWriter, r *http.Request) {
	identity, ok := s.requireIdentity(w, r)
	if !ok {
		return
	}
	filterID := r.PathValue("id")
	if !validCloudFilterID(filterID) {
		writeError(w, 400, "invalid_filter_id", "filter id is invalid")
		return
	}
	result, err := s.store.DB.ExecContext(r.Context(), `UPDATE filters SET status='deleted',updated_at=? WHERE id=? AND owner_id=? AND status='active'`, time.Now().UnixMilli(), filterID, identity.ID)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	affected, _ := result.RowsAffected()
	if affected != 1 {
		writeError(w, 404, "filter_not_found", "an active filter owned by this user was not found")
		return
	}
	writeJSON(w, 200, map[string]bool{"ok": true})
}

func (s *Server) recordDownloads(w http.ResponseWriter, r *http.Request) {
	identity, ok := s.requireIdentity(w, r)
	if !ok {
		return
	}
	var input struct {
		Items []struct {
			FilterID string `json:"filterId"`
			Revision int    `json:"revision"`
		} `json:"items"`
	}
	if !decodeJSON(w, r, &input) {
		return
	}
	if len(input.Items) < 1 || len(input.Items) > 100 {
		writeError(w, 400, "invalid_batch", "items must contain 1-100 downloads")
		return
	}
	tx, err := s.store.DB.BeginTx(r.Context(), nil)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer tx.Rollback()
	counted := 0
	now := time.Now().UnixMilli()
	for _, item := range input.Items {
		if item.FilterID == "" || item.Revision < 1 {
			writeError(w, 400, "invalid_download", "filterId and revision are required")
			return
		}
		result, err := tx.ExecContext(r.Context(), `INSERT OR IGNORE INTO filter_downloads(filter_id,revision,identity_id,created_at) SELECT id,?,?,? FROM filters WHERE id=? AND status='active' AND current_revision>=?`, item.Revision, identity.ID, now, item.FilterID, item.Revision)
		if err != nil {
			writeError(w, 500, "database_error", err.Error())
			return
		}
		affected, _ := result.RowsAffected()
		if affected > 0 {
			counted++
			if _, err = tx.ExecContext(r.Context(), `UPDATE filters SET download_count=download_count+1 WHERE id=?`, item.FilterID); err != nil {
				writeError(w, 500, "database_error", err.Error())
				return
			}
		}
	}
	if err = tx.Commit(); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	writeJSON(w, 200, map[string]int{"counted": counted})
}

func (s *Server) likeFilter(w http.ResponseWriter, r *http.Request)   { s.setLike(w, r, true) }
func (s *Server) unlikeFilter(w http.ResponseWriter, r *http.Request) { s.setLike(w, r, false) }
func (s *Server) setLike(w http.ResponseWriter, r *http.Request, liked bool) {
	identity, ok := s.requireIdentity(w, r)
	if !ok {
		return
	}
	id := r.PathValue("id")
	tx, err := s.store.DB.BeginTx(r.Context(), nil)
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	defer tx.Rollback()
	delta := 0
	if liked {
		result, e := tx.ExecContext(r.Context(), `INSERT OR IGNORE INTO filter_likes(filter_id,identity_id,created_at) SELECT id,?,? FROM filters WHERE id=? AND status='active'`, identity.ID, time.Now().UnixMilli(), id)
		err = e
		if result != nil {
			n, _ := result.RowsAffected()
			delta = int(n)
		}
	} else {
		result, e := tx.ExecContext(r.Context(), `DELETE FROM filter_likes WHERE filter_id=? AND identity_id=?`, id, identity.ID)
		err = e
		if result != nil {
			n, _ := result.RowsAffected()
			delta = -int(n)
		}
	}
	if err != nil {
		writeError(w, 404, "filter_not_found", "filter was not found")
		return
	}
	if delta != 0 {
		_, err = tx.ExecContext(r.Context(), `UPDATE filters SET like_count=MAX(0,like_count+?) WHERE id=?`, delta, id)
	}
	if err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	var count int
	if err = tx.QueryRowContext(r.Context(), `SELECT like_count FROM filters WHERE id=?`, id).Scan(&count); err != nil {
		writeError(w, 404, "filter_not_found", "filter was not found")
		return
	}
	if err = tx.Commit(); err != nil {
		writeError(w, 500, "database_error", err.Error())
		return
	}
	writeJSON(w, 200, map[string]any{"liked": liked, "likeCount": count})
}

func CreateAdmin(ctx context.Context, s *store.Store, username, password string) error {
	return upsertAdmin(ctx, s, username, password, false)
}
func ResetAdminPassword(ctx context.Context, s *store.Store, username, password string) error {
	return upsertAdmin(ctx, s, username, password, true)
}
func upsertAdmin(ctx context.Context, s *store.Store, username, password string, reset bool) error {
	username = strings.TrimSpace(username)
	if username == "" || len(password) < 10 {
		return errors.New("username is required and password must contain at least 10 characters")
	}
	hash, err := hashPassword(password)
	if err != nil {
		return err
	}
	now := time.Now().UnixMilli()
	if reset {
		tx, err := s.DB.BeginTx(ctx, nil)
		if err != nil {
			return err
		}
		defer tx.Rollback()
		result, err := tx.ExecContext(ctx, `UPDATE admins SET password_hash=?,status='active',updated_at=? WHERE username=? COLLATE NOCASE`, hash, now, username)
		if err != nil {
			return err
		}
		n, _ := result.RowsAffected()
		if n == 0 {
			return errors.New("administrator not found")
		}
		if _, err = tx.ExecContext(ctx, `DELETE FROM admin_sessions WHERE admin_id=(SELECT id FROM admins WHERE username=? COLLATE NOCASE)`, username); err != nil {
			return err
		}
		return tx.Commit()
	}
	tx, err := s.DB.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	id := randomID()
	if _, err = tx.ExecContext(ctx, `INSERT INTO admins(id,username,password_hash,status,created_at,updated_at) VALUES(?,?,?,'active',?,?)`, id, username, hash, now, now); err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO admin_role_members(admin_id,role_id,created_at) VALUES(?,?,?)`, id, superAdminRoleID, now); err != nil {
		return err
	}
	return tx.Commit()
}

func validClientToken(token string) bool {
	decoded, err := base64.RawURLEncoding.DecodeString(token)
	return err == nil && len(decoded) == 32
}
func validCloudFilterID(value string) bool {
	if value == "" || len(value) > 128 {
		return false
	}
	for _, character := range value {
		if (character < 'a' || character > 'z') && (character < 'A' || character > 'Z') && (character < '0' || character > '9') && character != '-' && character != '_' {
			return false
		}
	}
	return true
}
func tokenHash(token string) string {
	sum := sha256.Sum256([]byte(token))
	return hex.EncodeToString(sum[:])
}
func randomID() string {
	return uuid.NewString()
}
func randomToken(bytes int) string {
	b := make([]byte, bytes)
	if _, err := rand.Read(b); err != nil {
		panic(err)
	}
	return base64.RawURLEncoding.EncodeToString(b)
}
func boolInt(value bool) int {
	if value {
		return 1
	}
	return 0
}
func boundedInt(raw string, fallback, min, max int) int {
	value, err := strconv.Atoi(raw)
	if err != nil {
		return fallback
	}
	if value < min {
		return min
	}
	if value > max {
		return max
	}
	return value
}
func escapeLike(value string) string {
	value = strings.ReplaceAll(value, "\\", "\\\\")
	value = strings.ReplaceAll(value, "%", "\\%")
	return strings.ReplaceAll(value, "_", "\\_")
}
func validateShareItem(item *shareItem) string {
	item.ClientFilterID = strings.TrimSpace(item.ClientFilterID)
	item.Name = strings.TrimSpace(item.Name)
	item.Value = strings.TrimSpace(item.Value)
	item.Note = strings.TrimSpace(item.Note)
	if item.ClientFilterID == "" || len(item.ClientFilterID) > 128 {
		return "invalid clientFilterId"
	}
	if len([]rune(item.Name)) < 1 || len([]rune(item.Name)) > 100 {
		return "name must contain 1-100 characters"
	}
	if len([]rune(item.Value)) < 1 || len([]rune(item.Value)) > 2000 {
		return "value must contain 1-2000 characters"
	}
	if len([]rune(item.Note)) > 1000 {
		return "note must contain at most 1000 characters"
	}
	return ""
}
func decodeJSON(w http.ResponseWriter, r *http.Request, target any) bool {
	if validated, _ := r.Context().Value(validatedBodyKey).(bool); !validated {
		r.Body = http.MaxBytesReader(w, r.Body, maxBodyBytes)
	}
	decoder := json.NewDecoder(r.Body)
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(target); err != nil {
		writeError(w, 400, "invalid_json", err.Error())
		return false
	}
	if err := decoder.Decode(&struct{}{}); !errors.Is(err, io.EOF) {
		writeError(w, 400, "invalid_json", "request must contain one JSON value")
		return false
	}
	return true
}
func writeJSON(w http.ResponseWriter, status int, value any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(value)
}
func writeError(w http.ResponseWriter, status int, code, message string) {
	writeJSON(w, status, map[string]any{"error": map[string]string{"code": code, "message": message}})
}

func writeRevisionConflict(w http.ResponseWriter, currentRevision int) {
	writeJSON(w, http.StatusConflict, map[string]any{"error": map[string]any{
		"code":            "revision_conflict",
		"message":         "the filter has been updated by another user",
		"currentRevision": currentRevision,
	}})
}

func hashPassword(password string) (string, error) {
	salt := make([]byte, 16)
	if _, err := rand.Read(salt); err != nil {
		return "", err
	}
	hash := argon2.IDKey([]byte(password), salt, 3, 64*1024, 2, 32)
	return fmt.Sprintf("$argon2id$v=19$m=65536,t=3,p=2$%s$%s", base64.RawStdEncoding.EncodeToString(salt), base64.RawStdEncoding.EncodeToString(hash)), nil
}
func verifyPassword(encoded, password string) bool {
	parts := strings.Split(encoded, "$")
	if len(parts) != 6 {
		return false
	}
	salt, err1 := base64.RawStdEncoding.DecodeString(parts[4])
	expected, err2 := base64.RawStdEncoding.DecodeString(parts[5])
	if err1 != nil || err2 != nil {
		return false
	}
	actual := argon2.IDKey([]byte(password), salt, 3, 64*1024, 2, uint32(len(expected)))
	return subtle.ConstantTimeCompare(actual, expected) == 1
}

func ListenAndServe(s *store.Store, cfg config.Config) error {
	application, err := NewConfigured(s, cfg)
	if err != nil {
		return err
	}
	server := &http.Server{Addr: cfg.Listen, Handler: application.Handler(), ReadHeaderTimeout: 10 * time.Second, ReadTimeout: 30 * time.Second, WriteTimeout: 60 * time.Second, IdleTimeout: 90 * time.Second, MaxHeaderBytes: 32 * 1024}
	log.Printf("VCLogg server listening on %s", cfg.Listen)
	return server.ListenAndServe()
}

func WriteExampleConfig(path string) error {
	if _, err := os.Stat(path); err == nil {
		return nil
	}
	return os.WriteFile(path, []byte(exampleConfig), 0o640)
}
func ResolveBackupOutput(dataDir, output string) string {
	if output != "" {
		return output
	}
	return filepath.Join(dataDir, "backups")
}
