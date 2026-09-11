package app

import (
	"crypto/subtle"
	"database/sql"
	"errors"
	"net/http"
	"time"
)

type clientSession struct {
	Identity Identity
	CSRFHash string
}

func (s *Server) issueClientSession(w http.ResponseWriter, r *http.Request, identityID string) (string, bool) {
	now := time.Now()
	expires := now.Add(time.Duration(s.config.ClientAuth.SessionTTLHours) * time.Hour)
	token := randomToken(32)
	csrf := randomToken(24)
	tx, err := s.store.DB.BeginTx(r.Context(), nil)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "database_error", err.Error())
		return "", false
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(r.Context(), `DELETE FROM client_sessions WHERE expires_at<=?`, now.UnixMilli()); err == nil {
		_, err = tx.ExecContext(r.Context(), `DELETE FROM client_sessions WHERE token_hash IN (
SELECT token_hash FROM client_sessions WHERE identity_id=? ORDER BY created_at DESC LIMIT -1 OFFSET ?
)`, identityID, s.config.ClientAuth.MaxSessionsPerUser-1)
	}
	if err == nil {
		_, err = tx.ExecContext(r.Context(), `INSERT INTO client_sessions(token_hash,identity_id,csrf_hash,expires_at,created_at,last_seen_at) VALUES(?,?,?,?,?,?)`, tokenHash(token), identityID, tokenHash(csrf), expires.UnixMilli(), now.UnixMilli(), now.UnixMilli())
	}
	if err == nil {
		err = tx.Commit()
	}
	if err != nil {
		writeError(w, http.StatusInternalServerError, "database_error", err.Error())
		return "", false
	}
	http.SetCookie(w, &http.Cookie{
		Name:     clientCookie,
		Value:    token,
		Path:     "/api/v1",
		HttpOnly: true,
		Secure:   s.isSecureRequest(r),
		SameSite: http.SameSiteStrictMode,
		MaxAge:   s.config.ClientAuth.SessionTTLHours * 60 * 60,
		Expires:  expires,
	})
	return csrf, true
}

func (s *Server) hasValidClientSession(r *http.Request) (bool, error) {
	_, err := s.lookupClientSession(r)
	if errors.Is(err, http.ErrNoCookie) || errors.Is(err, sql.ErrNoRows) {
		return false, nil
	}
	return err == nil, err
}

func (s *Server) lookupClientSession(r *http.Request) (clientSession, error) {
	cookie, err := r.Cookie(clientCookie)
	if err != nil {
		return clientSession{}, err
	}
	var session clientSession
	err = s.store.DB.QueryRowContext(r.Context(), `
SELECT i.id,i.display_name,s.csrf_hash
FROM client_sessions s
JOIN identities i ON i.id=s.identity_id
WHERE s.token_hash=? AND s.expires_at>? AND i.revoked_at IS NULL`, tokenHash(cookie.Value), time.Now().UnixMilli()).Scan(&session.Identity.ID, &session.Identity.DisplayName, &session.CSRFHash)
	return session, err
}

func (s *Server) requireIdentity(w http.ResponseWriter, r *http.Request) (Identity, bool) {
	session, err := s.lookupClientSession(r)
	if errors.Is(err, http.ErrNoCookie) {
		writeError(w, http.StatusUnauthorized, "client_session_required", "a client session cookie is required")
		return Identity{}, false
	}
	if errors.Is(err, sql.ErrNoRows) {
		writeError(w, http.StatusUnauthorized, "client_session_expired", "the client session is expired, invalid, or revoked")
		return Identity{}, false
	}
	if err != nil {
		writeError(w, http.StatusInternalServerError, "database_error", err.Error())
		return Identity{}, false
	}
	if r.Method != http.MethodGet && r.Method != http.MethodHead {
		actual := tokenHash(r.Header.Get("X-VCLogg-CSRF"))
		if subtle.ConstantTimeCompare([]byte(actual), []byte(session.CSRFHash)) != 1 {
			writeError(w, http.StatusForbidden, "csrf_failed", "a valid client CSRF token is required")
			return Identity{}, false
		}
	}
	now := time.Now().UnixMilli()
	cookie, _ := r.Cookie(clientCookie)
	_, _ = s.store.DB.ExecContext(r.Context(), `UPDATE client_sessions SET last_seen_at=? WHERE token_hash=?`, now, tokenHash(cookie.Value))
	_, _ = s.store.DB.ExecContext(r.Context(), `UPDATE identities SET last_seen_at=? WHERE id=?`, now, session.Identity.ID)
	return session.Identity, true
}

func (s *Server) logoutIdentity(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.requireIdentity(w, r); !ok {
		return
	}
	cookie, _ := r.Cookie(clientCookie)
	_, _ = s.store.DB.ExecContext(r.Context(), `DELETE FROM client_sessions WHERE token_hash=?`, tokenHash(cookie.Value))
	http.SetCookie(w, &http.Cookie{
		Name:     clientCookie,
		Value:    "",
		Path:     "/api/v1",
		HttpOnly: true,
		Secure:   s.isSecureRequest(r),
		SameSite: http.SameSiteStrictMode,
		MaxAge:   -1,
	})
	writeJSON(w, http.StatusOK, map[string]bool{"ok": true})
}
