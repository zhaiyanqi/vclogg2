package app

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"database/sql"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"io"
	"net/http"
	"strconv"
	"strings"
	"time"
)

const (
	clientSignatureVersion = "VCLOGG1"
	clientKeyIDHeader      = "X-VCLogg-Key-Id"
	clientTimestampHeader  = "X-VCLogg-Timestamp"
	clientNonceHeader      = "X-VCLogg-Nonce"
	clientSignatureHeader  = "X-VCLogg-Signature"
)

type requestContextKey string

const authenticatedNonceKey requestContextKey = "authenticated-client-nonce"
const validatedBodyKey requestContextKey = "validated-request-body"
const validatedBodyBytesKey requestContextKey = "validated-request-body-bytes"

func (s *Server) prepareClientRequest(w http.ResponseWriter, r *http.Request) bool {
	body, ok := readRequestBody(w, r, s.config.Uploads.MaxRequestBodyBytes)
	if !ok {
		return false
	}
	r.Body = io.NopCloser(bytes.NewReader(body))
	r.ContentLength = int64(len(body))
	*r = *r.WithContext(context.WithValue(r.Context(), validatedBodyKey, true))
	*r = *r.WithContext(context.WithValue(r.Context(), validatedBodyBytesKey, body))
	return true
}

func (s *Server) authenticateClientRequest(w http.ResponseWriter, r *http.Request) bool {
	if !s.config.ClientAuth.Enabled {
		return true
	}
	body, _ := r.Context().Value(validatedBodyBytesKey).([]byte)

	keyID := strings.TrimSpace(r.Header.Get(clientKeyIDHeader))
	publicKey, exists := s.clientPublicKeys[keyID]
	if !exists {
		writeError(w, http.StatusUnauthorized, "client_authentication_required", "a signed VCLogg client request is required")
		return false
	}
	timestamp, err := strconv.ParseInt(r.Header.Get(clientTimestampHeader), 10, 64)
	if err != nil {
		writeError(w, http.StatusUnauthorized, "invalid_client_timestamp", "the client timestamp is invalid")
		return false
	}
	now := time.Now().UnixMilli()
	maxSkew := int64(s.config.ClientAuth.MaxClockSkewSeconds) * 1000
	if timestamp < now-maxSkew || timestamp > now+maxSkew {
		writeError(w, http.StatusUnauthorized, "expired_client_request", "the signed request timestamp is outside the allowed clock window")
		return false
	}
	nonce := strings.TrimSpace(r.Header.Get(clientNonceHeader))
	decodedNonce, err := base64.RawURLEncoding.DecodeString(nonce)
	if err != nil || len(decodedNonce) < 16 || len(decodedNonce) > 32 {
		writeError(w, http.StatusUnauthorized, "invalid_client_nonce", "the client nonce must be a 16-32 byte base64url value")
		return false
	}
	signature, err := base64.RawURLEncoding.DecodeString(r.Header.Get(clientSignatureHeader))
	if err != nil || len(signature) != ed25519.SignatureSize {
		writeError(w, http.StatusUnauthorized, "invalid_client_signature", "the client signature is invalid")
		return false
	}
	canonical := canonicalClientRequest(r.Method, requestTarget(r), timestamp, nonce, body)
	if !ed25519.Verify(publicKey, []byte(canonical), signature) {
		writeError(w, http.StatusUnauthorized, "invalid_client_signature", "the client signature is invalid")
		return false
	}

	expiresAt := now + int64(s.config.ClientAuth.NonceTTLSeconds)*1000
	if _, err = s.store.DB.ExecContext(r.Context(), `DELETE FROM request_nonces WHERE expires_at < ?`, now); err != nil {
		writeError(w, http.StatusInternalServerError, "database_error", err.Error())
		return false
	}
	result, err := s.store.DB.ExecContext(r.Context(), `INSERT OR IGNORE INTO request_nonces(client_key_id,nonce,expires_at) VALUES(?,?,?)`, keyID, nonce, expiresAt)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "database_error", err.Error())
		return false
	}
	inserted, _ := result.RowsAffected()
	if inserted != 1 {
		writeError(w, http.StatusUnauthorized, "replayed_client_request", "the signed request nonce has already been used")
		return false
	}
	*r = *r.WithContext(context.WithValue(r.Context(), authenticatedNonceKey, nonce))
	return true
}

func readRequestBody(w http.ResponseWriter, r *http.Request, maximum int64) ([]byte, bool) {
	defer r.Body.Close()
	if r.ContentLength > maximum {
		writeError(w, http.StatusRequestEntityTooLarge, "request_too_large", fmt.Sprintf("request body exceeds %d bytes", maximum))
		return nil, false
	}
	reader := io.LimitReader(r.Body, maximum+1)
	body, err := io.ReadAll(reader)
	if err != nil {
		writeError(w, http.StatusBadRequest, "invalid_request_body", "request body could not be read")
		return nil, false
	}
	if int64(len(body)) > maximum {
		writeError(w, http.StatusRequestEntityTooLarge, "request_too_large", fmt.Sprintf("request body exceeds %d bytes", maximum))
		return nil, false
	}
	return body, true
}

func requestTarget(r *http.Request) string {
	target := r.URL.EscapedPath()
	if r.URL.RawQuery != "" {
		target += "?" + r.URL.RawQuery
	}
	return target
}

func canonicalClientRequest(method, target string, timestamp int64, nonce string, body []byte) string {
	hash := sha256.Sum256(body)
	return strings.Join([]string{
		clientSignatureVersion,
		strconv.FormatInt(timestamp, 10),
		nonce,
		strings.ToUpper(method),
		target,
		hex.EncodeToString(hash[:]),
	}, "\n")
}

func (s *Server) consumeUploadRequest(w http.ResponseWriter, r *http.Request, tx *sql.Tx, identityID string, now int64) bool {
	cutoff := now - int64(s.config.Uploads.WindowSeconds)*1000
	if _, err := tx.ExecContext(r.Context(), `DELETE FROM upload_requests WHERE created_at < ?`, cutoff); err != nil {
		writeError(w, http.StatusInternalServerError, "database_error", err.Error())
		return false
	}
	var count int
	if err := tx.QueryRowContext(r.Context(), `SELECT COUNT(*) FROM upload_requests WHERE identity_id=? AND created_at>=?`, identityID, cutoff).Scan(&count); err != nil {
		writeError(w, http.StatusInternalServerError, "database_error", err.Error())
		return false
	}
	if count >= s.config.Uploads.MaxRequests {
		w.Header().Set("Retry-After", strconv.Itoa(s.config.Uploads.WindowSeconds))
		writeError(w, http.StatusTooManyRequests, "upload_rate_exceeded", "the upload request rate limit has been reached")
		return false
	}
	requestID, _ := r.Context().Value(authenticatedNonceKey).(string)
	if requestID == "" {
		requestID = randomID()
	}
	if _, err := tx.ExecContext(r.Context(), `INSERT INTO upload_requests(identity_id,request_id,created_at) VALUES(?,?,?)`, identityID, requestID, now); err != nil {
		writeError(w, http.StatusInternalServerError, "database_error", err.Error())
		return false
	}
	return true
}

func (s *Server) checkUploadQuota(w http.ResponseWriter, r *http.Request, tx *sql.Tx, identityID string) bool {
	var filters int
	var bytes int64
	err := tx.QueryRowContext(r.Context(), `
SELECT COUNT(*), COALESCE(SUM(
  length(CAST(f.client_filter_id AS BLOB)) +
  length(CAST(r.name AS BLOB)) +
  length(CAST(r.value AS BLOB)) +
  length(CAST(r.note AS BLOB))
), 0)
FROM filters f
JOIN filter_revisions r ON r.filter_id=f.id AND r.revision=f.current_revision
WHERE f.owner_id=? AND f.status!='deleted'`, identityID).Scan(&filters, &bytes)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "database_error", err.Error())
		return false
	}
	if filters > s.config.Uploads.MaxFiltersPerUser {
		writeError(w, http.StatusForbidden, "upload_filter_quota_exceeded", fmt.Sprintf("the per-user limit of %d shared filters would be exceeded", s.config.Uploads.MaxFiltersPerUser))
		return false
	}
	if bytes > s.config.Uploads.MaxBytesPerUser {
		writeError(w, http.StatusForbidden, "upload_storage_quota_exceeded", fmt.Sprintf("the per-user storage limit of %d bytes would be exceeded", s.config.Uploads.MaxBytesPerUser))
		return false
	}
	return true
}

const exampleConfig = `# HTTP 监听地址与 SQLite 数据目录。
listen: 127.0.0.1:8787
data_dir: ./data

# 仅当所有请求都必须经过可信 TLS 反向代理时启用。
trusted_proxy: false

# 两代客户端共用后台，但使用并列且隔离的更新目录。
updates:
  enabled: true
  # VCLogg：latest.yml、NSIS 安装包和 .blockmap。
  directory: ./data/updates
  # VCLogg2：latest.json、便携 ZIP 和 .blockmap.json。
  vclogg2_directory: ./data/updates-vclogg2

# 注册与未登录版本查询使用 Ed25519 签名；授权后使用 Cookie。
client_auth:
  enabled: true
  # 标准发行版默认接受随 server 二进制内置的官方客户端公钥。
  # 严格环境可关闭，并只使用下方 public_keys 中的自定义公钥。
  accept_bundled_public_keys: true
  max_clock_skew_seconds: 300
  nonce_ttl_seconds: 600
  session_ttl_hours: 720
  max_sessions_per_identity: 5
  public_keys: []
  # 使用 vclogg-server client-key generate 生成密钥对，然后替换空列表：
  # public_keys:
  #   - id: production-v1
  #     public_key: PASTE_THE_GENERATED_PUBLIC_KEY

# 每个身份的上传频率与保留内容配额。
uploads:
  window_seconds: 60
  max_requests_per_window: 10
  max_filters_per_user: 1000
  max_bytes_per_user: 10485760
  max_items_per_request: 100
  max_request_body_bytes: 524288
`
