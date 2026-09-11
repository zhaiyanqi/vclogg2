package app

import (
	"archive/zip"
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"crypto/sha512"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"vclogg/server/internal/config"
	"vclogg/server/internal/store"
)

type testClient struct {
	handler http.Handler
	token   string
	uuid    string
	cookie  *http.Cookie
	csrf    string
}

func newTestClient(t *testing.T) (*store.Store, *testClient) {
	t.Helper()
	database, err := store.Open(context.Background(), t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { database.Close() })
	return database, &testClient{handler: New(database).Handler(), token: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", uuid: "11111111-1111-4111-8111-111111111111"}
}
func (c *testClient) request(t *testing.T, method, path string, body any, authenticated bool) *httptest.ResponseRecorder {
	t.Helper()
	var data []byte
	if body != nil {
		var err error
		data, err = json.Marshal(body)
		if err != nil {
			t.Fatal(err)
		}
	}
	request := httptest.NewRequest(method, path, bytes.NewReader(data))
	if body != nil {
		request.Header.Set("Content-Type", "application/json")
	}
	if authenticated {
		if c.cookie != nil {
			request.AddCookie(c.cookie)
		}
		if method != http.MethodGet && method != http.MethodHead {
			request.Header.Set("X-VCLogg-CSRF", c.csrf)
		}
	}
	response := httptest.NewRecorder()
	c.handler.ServeHTTP(response, request)
	return response
}
func decode[T any](t *testing.T, response *httptest.ResponseRecorder) T {
	t.Helper()
	var result T
	if err := json.Unmarshal(response.Body.Bytes(), &result); err != nil {
		t.Fatalf("decode response: %v: %s", err, response.Body.String())
	}
	return result
}
func register(t *testing.T, c *testClient) Identity {
	t.Helper()
	response := c.request(t, "POST", "/api/v1/identities/register", map[string]any{"displayName": "Alice", "uuid": c.uuid}, false)
	if response.Code != 200 {
		t.Fatalf("register: %d %s", response.Code, response.Body.String())
	}
	result := decode[struct {
		ID           string   `json:"id"`
		DisplayName  string   `json:"displayName"`
		CSRF         string   `json:"csrfToken"`
		Capabilities []string `json:"capabilities"`
	}](t, response)
	if len(result.Capabilities) != 1 || result.Capabilities[0] != filterUUIDBranchesCapability {
		t.Fatalf("register capabilities = %#v", result.Capabilities)
	}
	cookies := response.Result().Cookies()
	if len(cookies) != 1 {
		t.Fatalf("register did not issue one cookie: %#v", cookies)
	}
	c.cookie = cookies[0]
	c.csrf = result.CSRF
	return Identity{ID: result.ID, DisplayName: result.DisplayName}
}

func TestRegisterUsesClientUUIDAndStoresOnlyItsHash(t *testing.T) {
	database, client := newTestClient(t)
	identity := register(t, client)
	if identity.DisplayName != "Alice" {
		t.Fatalf("unexpected identity: %#v", identity)
	}
	if identity.ID != client.uuid {
		t.Fatalf("identity ID %q did not use client UUID", identity.ID)
	}
	var stored string
	if err := database.DB.QueryRow(`SELECT token_hash FROM identities WHERE id=?`, identity.ID).Scan(&stored); err != nil {
		t.Fatal(err)
	}
	if stored == client.uuid {
		t.Fatal("raw UUID was stored as the identity key")
	}
	if stored != tokenHash(client.uuid) {
		t.Fatal("unexpected token hash")
	}
	if client.cookie.Name != clientCookie || !client.cookie.HttpOnly || client.cookie.SameSite != http.SameSiteStrictMode || client.cookie.Path != "/api/v1" || client.cookie.MaxAge != 720*60*60 {
		t.Fatalf("client cookie is not hardened: %#v", client.cookie)
	}
	if remaining := time.Until(client.cookie.Expires); remaining < 719*time.Hour || remaining > 721*time.Hour {
		t.Fatalf("client cookie expiry = %s", remaining)
	}
	var sessionHash, csrfHash string
	if err := database.DB.QueryRow(`SELECT token_hash,csrf_hash FROM client_sessions WHERE identity_id=?`, identity.ID).Scan(&sessionHash, &csrfHash); err != nil {
		t.Fatal(err)
	}
	if sessionHash != tokenHash(client.cookie.Value) || csrfHash != tokenHash(client.csrf) || sessionHash == client.cookie.Value || csrfHash == client.csrf {
		t.Fatal("client session secrets were not stored as hashes")
	}
	me := client.request(t, http.MethodGet, "/api/v1/me", nil, true)
	identityResponse := decode[struct {
		Capabilities []string `json:"capabilities"`
	}](t, me)
	if len(identityResponse.Capabilities) != 1 || identityResponse.Capabilities[0] != filterUUIDBranchesCapability {
		t.Fatalf("me capabilities = %#v", identityResponse.Capabilities)
	}
}

func TestUUIDFilterBranchesPreserveIdentityAndOwnership(t *testing.T) {
	_, client := newTestClient(t)
	register(t, client)
	branchID := "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
	share := map[string]any{"items": []map[string]any{{
		"clientFilterId": branchID,
		"name":           "Same",
		"value":          "VALUE",
		"useRegex":       false,
		"note":           "",
	}}}
	created := client.request(t, http.MethodPost, "/api/v1/filters/batch", share, true)
	if created.Code != http.StatusOK {
		t.Fatalf("create UUID branch: %d %s", created.Code, created.Body.String())
	}
	result := decode[struct {
		Items []struct {
			FilterID string `json:"filterId"`
			Revision int    `json:"revision"`
		} `json:"items"`
	}](t, created)
	if len(result.Items) != 1 || result.Items[0].FilterID != branchID || result.Items[0].Revision != 1 {
		t.Fatalf("UUID branch response: %s", created.Body.String())
	}
	idempotent := client.request(t, http.MethodPost, "/api/v1/filters/batch", share, true)
	if idempotent.Code != http.StatusConflict || !strings.Contains(idempotent.Body.String(), "no_changes") {
		t.Fatalf("unchanged UUID branch: %d %s", idempotent.Code, idempotent.Body.String())
	}
	changedBranch := map[string]any{"items": []map[string]any{{
		"clientFilterId": branchID,
		"name":           "Same",
		"value":          "VALUE",
		"useRegex":       false,
		"note":           "changed",
	}}}
	missingBase := client.request(t, http.MethodPost, "/api/v1/filters/batch", changedBranch, true)
	if missingBase.Code != http.StatusConflict || !strings.Contains(missingBase.Body.String(), "revision_conflict") || !strings.Contains(missingBase.Body.String(), "currentRevision") {
		t.Fatalf("UUID update without base revision: %d %s", missingBase.Code, missingBase.Body.String())
	}
	changedBranch["items"].([]map[string]any)[0]["baseRevision"] = 1
	updated := client.request(t, http.MethodPost, "/api/v1/filters/batch", changedBranch, true)
	if updated.Code != http.StatusOK || !strings.Contains(updated.Body.String(), `"revision":2`) {
		t.Fatalf("UUID update with base revision: %d %s", updated.Code, updated.Body.String())
	}
	secondBranch := client.request(t, http.MethodPost, "/api/v1/filters/batch", map[string]any{
		"items": []map[string]any{{
			"clientFilterId": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
			"name":           "Same",
			"value":          "VALUE",
			"useRegex":       false,
			"note":           "",
		}},
	}, true)
	if secondBranch.Code != http.StatusOK {
		t.Fatalf("equal-content UUID branch: %d %s", secondBranch.Code, secondBranch.Body.String())
	}

	other := &testClient{
		handler: client.handler,
		uuid:    "22222222-2222-4222-8222-222222222222",
	}
	register(t, other)
	identityConflict := other.request(t, http.MethodPost, "/api/v1/filters/batch", share, true)
	if identityConflict.Code != http.StatusConflict || !strings.Contains(identityConflict.Body.String(), "filter_identity_conflict") {
		t.Fatalf("cross-owner UUID branch: %d %s", identityConflict.Code, identityConflict.Body.String())
	}

	legacy := other.request(t, http.MethodPost, "/api/v1/filters/batch", map[string]any{
		"items": []map[string]any{{
			"clientFilterId": "legacy-local-id",
			"name":           "Legacy",
			"value":          "LEGACY",
			"useRegex":       false,
			"note":           "",
		}},
	}, true)
	if legacy.Code != http.StatusOK {
		t.Fatalf("legacy client share: %d %s", legacy.Code, legacy.Body.String())
	}
	legacyResult := decode[struct {
		Items []struct {
			FilterID string `json:"filterId"`
		} `json:"items"`
	}](t, legacy)
	if legacyResult.Items[0].FilterID == "legacy-local-id" {
		t.Fatal("legacy non-UUID clientFilterId was used as the server primary key")
	}
}

func TestUnchangedShareIsRejectedWhileEqualContentBranchesCoexist(t *testing.T) {
	database, client := newTestClient(t)
	register(t, client)
	share := map[string]any{"items": []map[string]any{{"clientFilterId": "local-1", "name": "Camera", "value": "CameraService", "useRegex": false, "note": "camera logs"}}}
	first := client.request(t, "POST", "/api/v1/filters/batch", share, true)
	if first.Code != 200 {
		t.Fatalf("share: %d %s", first.Code, first.Body.String())
	}
	result := decode[struct {
		Items []struct {
			FilterID string `json:"filterId"`
			Revision int    `json:"revision"`
		} `json:"items"`
	}](t, first)
	filterID := result.Items[0].FilterID
	second := client.request(t, "POST", "/api/v1/filters/batch", share, true)
	if second.Code != http.StatusConflict || !strings.Contains(second.Body.String(), "no_changes") {
		t.Fatalf("unchanged share was accepted: %d %s", second.Code, second.Body.String())
	}
	share["items"].([]map[string]any)[0]["note"] = "updated camera logs"
	changed := client.request(t, http.MethodPost, "/api/v1/filters/batch", share, true)
	if changed.Code != http.StatusOK {
		t.Fatalf("changed share: %d %s", changed.Code, changed.Body.String())
	}
	changedResult := decode[struct {
		Items []struct {
			Revision int `json:"revision"`
		} `json:"items"`
	}](t, changed)
	if changedResult.Items[0].Revision != 2 {
		t.Fatalf("changed share did not create revision 2: %s", changed.Body.String())
	}
	duplicate := client.request(t, http.MethodPost, "/api/v1/filters/batch", map[string]any{"items": []map[string]any{{"clientFilterId": "local-copy", "name": "Camera", "value": "CameraService", "useRegex": false, "note": "another note"}}}, true)
	if duplicate.Code != http.StatusOK {
		t.Fatalf("equal-content branch: %d %s", duplicate.Code, duplicate.Body.String())
	}
	updated := client.request(t, http.MethodPatch, "/api/v1/filters/"+filterID, map[string]any{"name": "Camera edited", "value": "CameraService", "useRegex": false, "note": "owner edit"}, true)
	if updated.Code != http.StatusOK {
		t.Fatalf("owner edit: %d %s", updated.Code, updated.Body.String())
	}
	if result := decode[struct {
		Revision int `json:"revision"`
	}](t, updated); result.Revision != 3 {
		t.Fatalf("owner edit revision: %s", updated.Body.String())
	}

	listed := client.request(t, "GET", "/api/v1/filters?q=camera&sort=newest&page=1", nil, true)
	if listed.Code != 200 {
		t.Fatalf("list: %d %s", listed.Code, listed.Body.String())
	}
	page := decode[struct {
		Items []CloudFilter `json:"items"`
		Total int           `json:"total"`
	}](t, listed)
	if page.Total != 2 || len(page.Items) != 2 {
		t.Fatalf("unexpected page: %#v", page)
	}

	for i := 0; i < 2; i++ {
		response := client.request(t, "PUT", "/api/v1/filters/"+filterID+"/like", nil, true)
		if response.Code != 200 {
			t.Fatalf("like: %s", response.Body.String())
		}
	}
	download := map[string]any{"items": []map[string]any{{"filterId": filterID, "revision": 1}}}
	for i := 0; i < 2; i++ {
		response := client.request(t, "POST", "/api/v1/downloads/batch", download, true)
		if response.Code != 200 {
			t.Fatalf("download: %s", response.Body.String())
		}
	}
	download = map[string]any{"items": []map[string]any{{"filterId": filterID, "revision": 2}}}
	for i := 0; i < 2; i++ {
		response := client.request(t, "POST", "/api/v1/downloads/batch", download, true)
		if response.Code != 200 {
			t.Fatalf("download new revision: %s", response.Body.String())
		}
	}
	var likes, downloads int
	if err := database.DB.QueryRow(`SELECT like_count,download_count FROM filters WHERE id=?`, filterID).Scan(&likes, &downloads); err != nil {
		t.Fatal(err)
	}
	if likes != 1 || downloads != 2 {
		t.Fatalf("counts were likes=%d downloads=%d", likes, downloads)
	}
	deleted := client.request(t, http.MethodDelete, "/api/v1/filters/"+filterID, nil, true)
	if deleted.Code != http.StatusOK {
		t.Fatalf("owner delete: %d %s", deleted.Code, deleted.Body.String())
	}
	listed = client.request(t, http.MethodGet, "/api/v1/filters", nil, true)
	if decode[struct {
		Total int `json:"total"`
	}](t, listed).Total != 1 {
		t.Fatal("deleting one UUID branch affected another branch")
	}
	reShared := client.request(t, http.MethodPost, "/api/v1/filters/batch", share, true)
	if reShared.Code != http.StatusOK {
		t.Fatalf("re-share deleted filter: %d %s", reShared.Code, reShared.Body.String())
	}
	reSharedResult := decode[struct {
		Items []struct {
			FilterID string `json:"filterId"`
		} `json:"items"`
	}](t, reShared)
	if len(reSharedResult.Items) != 1 || reSharedResult.Items[0].FilterID != filterID {
		t.Fatalf("re-share did not reactivate the deleted filter: %s", reShared.Body.String())
	}
	listed = client.request(t, http.MethodGet, "/api/v1/filters", nil, true)
	if decode[struct {
		Total int `json:"total"`
	}](t, listed).Total != 2 {
		t.Fatal("re-shared filter was not visible")
	}
}

func TestCollaborativeEditingPermissionsAndRevisionHistory(t *testing.T) {
	_, owner := newTestClient(t)
	ownerIdentity := register(t, owner)
	other := &testClient{
		handler: owner.handler,
		uuid:    "22222222-2222-4222-8222-222222222222",
	}
	register(t, other)
	response := other.request(t, http.MethodPatch, "/api/v1/me", map[string]string{"displayName": "Bob"}, true)
	if response.Code != http.StatusOK {
		t.Fatalf("rename collaborator: %d %s", response.Code, response.Body.String())
	}

	shared := owner.request(t, http.MethodPost, "/api/v1/filters/batch", map[string]any{"items": []map[string]any{{
		"clientFilterId": "collaborative-1",
		"name":           "Camera",
		"value":          "CameraService",
		"useRegex":       false,
		"note":           "initial",
	}}}, true)
	if shared.Code != http.StatusOK {
		t.Fatalf("share: %d %s", shared.Code, shared.Body.String())
	}
	result := decode[struct {
		Items []struct {
			FilterID string `json:"filterId"`
		} `json:"items"`
	}](t, shared)
	filterID := result.Items[0].FilterID

	denied := other.request(t, http.MethodPatch, "/api/v1/filters/"+filterID, map[string]any{
		"name": "Camera", "value": "CameraService2", "useRegex": false, "note": "blocked", "baseRevision": 1,
	}, true)
	if denied.Code != http.StatusForbidden || !strings.Contains(denied.Body.String(), "filter_not_editable") {
		t.Fatalf("non-collaborative edit: %d %s", denied.Code, denied.Body.String())
	}

	enabled := owner.request(t, http.MethodPatch, "/api/v1/filters/"+filterID, map[string]any{
		"name": "Camera", "value": "CameraService", "useRegex": false, "note": "initial", "collaborative": true, "baseRevision": 1,
	}, true)
	if enabled.Code != http.StatusOK || decode[struct {
		Revision int `json:"revision"`
	}](t, enabled).Revision != 2 {
		t.Fatalf("enable collaboration: %d %s", enabled.Code, enabled.Body.String())
	}

	detail := other.request(t, http.MethodGet, "/api/v1/filters/"+filterID, nil, true)
	item := decode[CloudFilter](t, detail)
	if !item.Collaborative || !item.CanEdit || item.CanDelete || item.OwnerID != ownerIdentity.ID {
		t.Fatalf("collaborator permissions: %#v", item)
	}

	edited := other.request(t, http.MethodPatch, "/api/v1/filters/"+filterID, map[string]any{
		"name": "Camera shared", "value": "CameraService2", "useRegex": true, "note": "Bob edit", "baseRevision": 2,
	}, true)
	if edited.Code != http.StatusOK || decode[struct {
		Revision int `json:"revision"`
	}](t, edited).Revision != 3 {
		t.Fatalf("collaborator edit: %d %s", edited.Code, edited.Body.String())
	}

	ownerOnly := other.request(t, http.MethodPatch, "/api/v1/filters/"+filterID, map[string]any{
		"name": "Camera shared", "value": "CameraService2", "useRegex": true, "note": "Bob edit", "collaborative": false, "baseRevision": 3,
	}, true)
	if ownerOnly.Code != http.StatusForbidden || !strings.Contains(ownerOnly.Body.String(), "collaboration_owner_only") {
		t.Fatalf("collaborator changed collaboration: %d %s", ownerOnly.Code, ownerOnly.Body.String())
	}
	deleted := other.request(t, http.MethodDelete, "/api/v1/filters/"+filterID, nil, true)
	if deleted.Code == http.StatusOK {
		t.Fatal("collaborator deleted the owner's filter")
	}

	history := other.request(t, http.MethodGet, "/api/v1/filters/"+filterID+"/revisions?page=1&pageSize=30", nil, true)
	page := decode[struct {
		Items []CloudFilterRevisionSummary `json:"items"`
		Total int                          `json:"total"`
	}](t, history)
	if page.Total != 3 || len(page.Items) != 3 || page.Items[0].EditorRole != "collaborator" || page.Items[0].EditorName != "Bob" || !page.Items[0].Current {
		t.Fatalf("revision history: %#v", page)
	}
	firstRevision := other.request(t, http.MethodGet, "/api/v1/filters/"+filterID+"/revisions/1", nil, true)
	snapshot := decode[CloudFilterRevision](t, firstRevision)
	if snapshot.Name != "Camera" || snapshot.Value != "CameraService" || snapshot.Collaborative || snapshot.Current {
		t.Fatalf("revision 1 snapshot: %#v", snapshot)
	}

	conflict := owner.request(t, http.MethodPatch, "/api/v1/filters/"+filterID, map[string]any{
		"name": "stale", "value": "stale", "useRegex": false, "note": "stale", "baseRevision": 1,
	}, true)
	if conflict.Code != http.StatusConflict || !strings.Contains(conflict.Body.String(), "revision_conflict") || !strings.Contains(conflict.Body.String(), "currentRevision") {
		t.Fatalf("revision conflict: %d %s", conflict.Code, conflict.Body.String())
	}

	disabled := owner.request(t, http.MethodPatch, "/api/v1/filters/"+filterID, map[string]any{
		"name": "Camera shared", "value": "CameraService2", "useRegex": true, "note": "Bob edit", "collaborative": false, "baseRevision": 3,
	}, true)
	if disabled.Code != http.StatusOK {
		t.Fatalf("disable collaboration: %d %s", disabled.Code, disabled.Body.String())
	}
	unchangedEdit := owner.request(t, http.MethodPatch, "/api/v1/filters/"+filterID, map[string]any{
		"name": "Camera shared", "value": "CameraService2", "useRegex": true, "note": "Bob edit", "collaborative": false, "baseRevision": 4,
	}, true)
	if unchangedEdit.Code != http.StatusConflict || !strings.Contains(unchangedEdit.Body.String(), "no_changes") {
		t.Fatalf("unchanged edit was accepted: %d %s", unchangedEdit.Code, unchangedEdit.Body.String())
	}
	unchangedShare := owner.request(t, http.MethodPost, "/api/v1/filters/batch", map[string]any{"items": []map[string]any{{
		"clientFilterId": "collaborative-1",
		"name":           "Camera shared",
		"value":          "CameraService2",
		"useRegex":       true,
		"note":           "Bob edit",
		"collaborative":  false,
		"baseRevision":   4,
	}}}, true)
	if unchangedShare.Code != http.StatusConflict || !strings.Contains(unchangedShare.Body.String(), "no_changes") {
		t.Fatalf("unchanged share was accepted: %d %s", unchangedShare.Code, unchangedShare.Body.String())
	}
	history = owner.request(t, http.MethodGet, "/api/v1/filters/"+filterID+"/revisions?page=1&pageSize=30", nil, true)
	page = decode[struct {
		Items []CloudFilterRevisionSummary `json:"items"`
		Total int                          `json:"total"`
	}](t, history)
	if page.Total != 4 || len(page.Items) != 4 || page.Items[0].Revision != 4 {
		t.Fatalf("unchanged writes modified history: %#v", page)
	}
	denied = other.request(t, http.MethodPatch, "/api/v1/filters/"+filterID, map[string]any{
		"name": "again", "value": "again", "useRegex": false, "note": "again", "baseRevision": 4,
	}, true)
	if denied.Code != http.StatusForbidden {
		t.Fatalf("edit after collaboration disabled: %d %s", denied.Code, denied.Body.String())
	}
	restored := owner.request(t, http.MethodPatch, "/api/v1/filters/"+filterID, map[string]any{
		"name": "Camera", "value": "CameraService", "useRegex": false, "note": "initial", "collaborative": false, "baseRevision": 4,
	}, true)
	if restored.Code != http.StatusOK || decode[struct {
		Revision int `json:"revision"`
	}](t, restored).Revision != 5 {
		t.Fatalf("owner restore snapshot: %d %s", restored.Code, restored.Body.String())
	}
	restoredDetail := decode[CloudFilter](t, owner.request(t, http.MethodGet, "/api/v1/filters/"+filterID, nil, true))
	if restoredDetail.Name != "Camera" || restoredDetail.Value != "CameraService" || restoredDetail.UseRegex || restoredDetail.Note != "initial" || restoredDetail.Collaborative || restoredDetail.Revision != 5 {
		t.Fatalf("restored detail: %#v", restoredDetail)
	}
}

func TestRevokedIdentityCannotUseAPI(t *testing.T) {
	database, client := newTestClient(t)
	identity := register(t, client)
	if err := CreateAdmin(context.Background(), database, "admin", "strong-password"); err != nil {
		t.Fatal(err)
	}
	login := client.request(t, http.MethodPost, "/admin/api/login", map[string]string{
		"username": "admin",
		"password": "strong-password",
	}, false)
	if login.Code != http.StatusOK {
		t.Fatalf("login: %d %s", login.Code, login.Body.String())
	}
	session := decode[map[string]string](t, login)
	revoke := httptest.NewRequest(http.MethodPatch, "/admin/api/identities/"+identity.ID, bytes.NewBufferString(`{"revoked":true}`))
	revoke.Header.Set("Content-Type", "application/json")
	revoke.Header.Set("X-CSRF-Token", session["csrfToken"])
	revoke.AddCookie(login.Result().Cookies()[0])
	revokeResponse := httptest.NewRecorder()
	client.handler.ServeHTTP(revokeResponse, revoke)
	if revokeResponse.Code != http.StatusOK {
		t.Fatalf("revoke: %d %s", revokeResponse.Code, revokeResponse.Body.String())
	}
	var sessions int
	if err := database.DB.QueryRow(`SELECT COUNT(*) FROM client_sessions WHERE identity_id=?`, identity.ID).Scan(&sessions); err != nil || sessions != 0 {
		t.Fatalf("revoked identity sessions = %d, err=%v", sessions, err)
	}
	response := client.request(t, "GET", "/api/v1/me", nil, true)
	if response.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401, got %d", response.Code)
	}
}

func TestBearerIsRejectedAndMutationsRequireCSRF(t *testing.T) {
	_, client := newTestClient(t)
	register(t, client)
	bearer := httptest.NewRequest(http.MethodGet, "/api/v1/me", nil)
	bearer.Header.Set("Authorization", "Bearer "+client.token)
	bearerResponse := httptest.NewRecorder()
	client.handler.ServeHTTP(bearerResponse, bearer)
	if bearerResponse.Code != http.StatusUnauthorized || !strings.Contains(bearerResponse.Body.String(), "client_session_required") {
		t.Fatalf("bearer request: %d %s", bearerResponse.Code, bearerResponse.Body.String())
	}

	mutation := httptest.NewRequest(http.MethodPatch, "/api/v1/me", bytes.NewBufferString(`{"displayName":"Bob"}`))
	mutation.Header.Set("Content-Type", "application/json")
	mutation.AddCookie(client.cookie)
	mutationResponse := httptest.NewRecorder()
	client.handler.ServeHTTP(mutationResponse, mutation)
	if mutationResponse.Code != http.StatusForbidden || !strings.Contains(mutationResponse.Body.String(), "csrf_failed") {
		t.Fatalf("csrf request: %d %s", mutationResponse.Code, mutationResponse.Body.String())
	}
}

func TestExpiredSessionAndSecureCookie(t *testing.T) {
	database, client := newTestClient(t)
	register(t, client)
	if _, err := database.DB.Exec(`UPDATE client_sessions SET expires_at=1 WHERE token_hash=?`, tokenHash(client.cookie.Value)); err != nil {
		t.Fatal(err)
	}
	expired := client.request(t, http.MethodGet, "/api/v1/me", nil, true)
	if expired.Code != http.StatusUnauthorized || !strings.Contains(expired.Body.String(), "client_session_expired") {
		t.Fatalf("expired session: %d %s", expired.Code, expired.Body.String())
	}

	body := bytes.NewBufferString(`{"displayName":"Alice","token":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}`)
	request := httptest.NewRequest(http.MethodPost, "https://cloud.example/api/v1/identities/register", body)
	request.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	client.handler.ServeHTTP(response, request)
	if response.Code != http.StatusOK || len(response.Result().Cookies()) != 1 || !response.Result().Cookies()[0].Secure {
		t.Fatalf("HTTPS cookie was not secure: %d %#v", response.Code, response.Result().Cookies())
	}
}

func TestLogoutAndSessionLimit(t *testing.T) {
	database, client := newTestClient(t)
	identity := register(t, client)
	for range 6 {
		register(t, client)
	}
	var sessions int
	if err := database.DB.QueryRow(`SELECT COUNT(*) FROM client_sessions WHERE identity_id=?`, identity.ID).Scan(&sessions); err != nil {
		t.Fatal(err)
	}
	if sessions != 5 {
		t.Fatalf("session count = %d", sessions)
	}
	logout := client.request(t, http.MethodPost, "/api/v1/session/logout", nil, true)
	if logout.Code != http.StatusOK || len(logout.Result().Cookies()) != 1 || logout.Result().Cookies()[0].MaxAge >= 0 {
		t.Fatalf("logout: %d %s %#v", logout.Code, logout.Body.String(), logout.Result().Cookies())
	}
	me := client.request(t, http.MethodGet, "/api/v1/me", nil, true)
	if me.Code != http.StatusUnauthorized {
		t.Fatalf("logged-out session remained valid: %d", me.Code)
	}
}

func TestClientSignatureIsRequiredAndCannotBeReplayed(t *testing.T) {
	database, err := store.Open(context.Background(), t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { database.Close() })
	publicKey, privateKey, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	cfg := config.Default()
	cfg.ClientAuth.PublicKeys = []config.ClientPublicKey{{
		ID:        "test-key",
		PublicKey: base64.RawURLEncoding.EncodeToString(publicKey),
	}}
	server, err := NewConfigured(database, cfg)
	if err != nil {
		t.Fatal(err)
	}
	body := []byte(`{"displayName":"Alice","token":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}`)
	unsigned := httptest.NewRequest(http.MethodPost, "/api/v1/identities/register", bytes.NewReader(body))
	unsignedResponse := httptest.NewRecorder()
	server.Handler().ServeHTTP(unsignedResponse, unsigned)
	if unsignedResponse.Code != http.StatusUnauthorized {
		t.Fatalf("unsigned request status = %d", unsignedResponse.Code)
	}

	timestamp := time.Now().UnixMilli()
	nonce := base64.RawURLEncoding.EncodeToString(bytes.Repeat([]byte{7}, 16))
	signature := ed25519.Sign(privateKey, []byte(canonicalClientRequest(http.MethodPost, "/api/v1/identities/register", timestamp, nonce, body)))
	request := func(payload []byte) *httptest.ResponseRecorder {
		r := httptest.NewRequest(http.MethodPost, "/api/v1/identities/register", bytes.NewReader(payload))
		r.Header.Set(clientKeyIDHeader, "test-key")
		r.Header.Set(clientTimestampHeader, fmt.Sprint(timestamp))
		r.Header.Set(clientNonceHeader, nonce)
		r.Header.Set(clientSignatureHeader, base64.RawURLEncoding.EncodeToString(signature))
		response := httptest.NewRecorder()
		server.Handler().ServeHTTP(response, r)
		return response
	}
	tampered := request([]byte(`{"displayName":"Mallory","token":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}`))
	if tampered.Code != http.StatusUnauthorized || !strings.Contains(tampered.Body.String(), "invalid_client_signature") {
		t.Fatalf("tampered request: %d %s", tampered.Code, tampered.Body.String())
	}
	first := request(body)
	if first.Code != http.StatusOK {
		t.Fatalf("signed request: %d %s", first.Code, first.Body.String())
	}
	versionWithCookie := httptest.NewRequest(http.MethodGet, "/api/v1/app-version", nil)
	versionWithCookie.AddCookie(first.Result().Cookies()[0])
	versionWithCookieResponse := httptest.NewRecorder()
	server.Handler().ServeHTTP(versionWithCookieResponse, versionWithCookie)
	if versionWithCookieResponse.Code != http.StatusOK {
		t.Fatalf("cookie version request: %d %s", versionWithCookieResponse.Code, versionWithCookieResponse.Body.String())
	}
	unsignedVersion := httptest.NewRequest(http.MethodGet, "/api/v1/app-version", nil)
	unsignedVersionResponse := httptest.NewRecorder()
	server.Handler().ServeHTTP(unsignedVersionResponse, unsignedVersion)
	if unsignedVersionResponse.Code != http.StatusUnauthorized {
		t.Fatalf("unsigned version request status = %d", unsignedVersionResponse.Code)
	}
	versionTimestamp := time.Now().UnixMilli()
	versionNonce := base64.RawURLEncoding.EncodeToString(bytes.Repeat([]byte{8}, 16))
	versionSignature := ed25519.Sign(privateKey, []byte(canonicalClientRequest(http.MethodGet, "/api/v1/app-version", versionTimestamp, versionNonce, nil)))
	signedVersion := httptest.NewRequest(http.MethodGet, "/api/v1/app-version", nil)
	signedVersion.Header.Set(clientKeyIDHeader, "test-key")
	signedVersion.Header.Set(clientTimestampHeader, fmt.Sprint(versionTimestamp))
	signedVersion.Header.Set(clientNonceHeader, versionNonce)
	signedVersion.Header.Set(clientSignatureHeader, base64.RawURLEncoding.EncodeToString(versionSignature))
	signedVersionResponse := httptest.NewRecorder()
	server.Handler().ServeHTTP(signedVersionResponse, signedVersion)
	if signedVersionResponse.Code != http.StatusOK {
		t.Fatalf("signed version request: %d %s", signedVersionResponse.Code, signedVersionResponse.Body.String())
	}
	replay := request(body)
	if replay.Code != http.StatusUnauthorized || !strings.Contains(replay.Body.String(), "replayed_client_request") {
		t.Fatalf("replayed request: %d %s", replay.Code, replay.Body.String())
	}
}

func TestUploadRateAndQuotaAreEnforcedPerIdentity(t *testing.T) {
	database, client := newTestClient(t)
	register(t, client)
	cfg := config.Default()
	cfg.ClientAuth.Enabled = false
	cfg.Uploads.MaxRequests = 10
	cfg.Uploads.MaxFiltersPerUser = 1
	server, err := NewConfigured(database, cfg)
	if err != nil {
		t.Fatal(err)
	}
	client.handler = server.Handler()
	twoFilters := map[string]any{"items": []map[string]any{
		{"clientFilterId": "one", "name": "One", "value": "ONE", "useRegex": false, "note": ""},
		{"clientFilterId": "two", "name": "Two", "value": "TWO", "useRegex": false, "note": ""},
	}}
	quota := client.request(t, http.MethodPost, "/api/v1/filters/batch", twoFilters, true)
	if quota.Code != http.StatusForbidden || !strings.Contains(quota.Body.String(), "upload_filter_quota_exceeded") {
		t.Fatalf("quota response: %d %s", quota.Code, quota.Body.String())
	}
	var stored int
	if err := database.DB.QueryRow(`SELECT COUNT(*) FROM filters`).Scan(&stored); err != nil || stored != 0 {
		t.Fatalf("quota failure was not rolled back: count=%d err=%v", stored, err)
	}

	cfg.Uploads.MaxFiltersPerUser = 10
	cfg.Uploads.MaxRequests = 1
	server, err = NewConfigured(database, cfg)
	if err != nil {
		t.Fatal(err)
	}
	client.handler = server.Handler()
	oneFilter := map[string]any{"items": []map[string]any{{"clientFilterId": "one", "name": "One", "value": "ONE", "useRegex": false, "note": ""}}}
	first := client.request(t, http.MethodPost, "/api/v1/filters/batch", oneFilter, true)
	if first.Code != http.StatusOK {
		t.Fatalf("first upload: %d %s", first.Code, first.Body.String())
	}
	second := client.request(t, http.MethodPost, "/api/v1/filters/batch", oneFilter, true)
	if second.Code != http.StatusTooManyRequests || second.Header().Get("Retry-After") == "" {
		t.Fatalf("rate response: %d %s", second.Code, second.Body.String())
	}
}

func TestAdminVersionValidation(t *testing.T) {
	database, client := newTestClient(t)
	if err := CreateAdmin(context.Background(), database, "admin", "strong-password"); err != nil {
		t.Fatal(err)
	}
	if !semverPattern.MatchString("1.2.3-beta.1") {
		t.Fatal("valid semantic version rejected")
	}
	if semverPattern.MatchString("1.2") {
		t.Fatal("invalid semantic version accepted")
	}
	if semverPattern.MatchString("1.2.3-01") {
		t.Fatal("semantic version with a leading-zero prerelease was accepted")
	}

	login := client.request(t, "POST", "/admin/api/login", map[string]string{
		"username": "admin",
		"password": "strong-password",
	}, false)
	if login.Code != http.StatusOK {
		t.Fatalf("login: %d %s", login.Code, login.Body.String())
	}
	session := decode[map[string]string](t, login)
	cookies := login.Result().Cookies()
	if len(cookies) != 1 || !cookies[0].HttpOnly || cookies[0].SameSite != http.SameSiteStrictMode {
		t.Fatalf("admin cookie is not hardened: %#v", cookies)
	}
	request := httptest.NewRequest(http.MethodPut, "/admin/api/settings/version", bytes.NewBufferString(`{"latestVersion":"1.2.3"}`))
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("X-CSRF-Token", session["csrfToken"])
	request.AddCookie(cookies[0])
	response := httptest.NewRecorder()
	client.handler.ServeHTTP(response, request)
	if response.Code != http.StatusOK {
		t.Fatalf("set version: %d %s", response.Code, response.Body.String())
	}
}

func TestWindowsUpdateFilesSupportMetadataCachingAndRanges(t *testing.T) {
	database, _ := newTestClient(t)
	updateDirectory := t.TempDir()
	vclogg2UpdateDirectory := t.TempDir()
	platformDirectory := filepath.Join(updateDirectory, "win-x64")
	vclogg2PlatformDirectory := filepath.Join(vclogg2UpdateDirectory, "win-x64")
	if err := os.MkdirAll(platformDirectory, 0o750); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(vclogg2PlatformDirectory, 0o750); err != nil {
		t.Fatal(err)
	}
	manifest := []byte("version: 0.8.0\npath: VCLogg-0.8.0-Setup-x64.exe\n")
	installer := []byte("0123456789")
	if err := os.WriteFile(filepath.Join(platformDirectory, "latest.yml"), manifest, 0o640); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(platformDirectory, "VCLogg-0.8.0-Setup-x64.exe"), installer, 0o640); err != nil {
		t.Fatal(err)
	}
	vclogg2Manifest := []byte(`{"schemaVersion":1,"product":"VCLogg2","version":"2.0.0"}`)
	vclogg2Archive := []byte("vclogg2-archive")
	if err := os.WriteFile(filepath.Join(vclogg2PlatformDirectory, "latest.json"), vclogg2Manifest, 0o640); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(vclogg2PlatformDirectory, "vclogg2-2.0.0-win-x64.zip"), vclogg2Archive, 0o640); err != nil {
		t.Fatal(err)
	}
	cfg := config.Default()
	cfg.ClientAuth.Enabled = false
	cfg.Updates.Directory = updateDirectory
	cfg.Updates.VCLogg2Directory = vclogg2UpdateDirectory
	server, err := NewConfigured(database, cfg)
	if err != nil {
		t.Fatal(err)
	}

	metadataRequest := httptest.NewRequest(http.MethodGet, "/updates/win-x64/latest.yml", nil)
	metadataResponse := httptest.NewRecorder()
	server.Handler().ServeHTTP(metadataResponse, metadataRequest)
	if metadataResponse.Code != http.StatusOK || metadataResponse.Header().Get("Cache-Control") != "no-store" {
		t.Fatalf("manifest response: %d headers=%v", metadataResponse.Code, metadataResponse.Header())
	}
	rangeRequest := httptest.NewRequest(http.MethodGet, "/updates/win-x64/VCLogg-0.8.0-Setup-x64.exe", nil)
	rangeRequest.Header.Set("Range", "bytes=2-5")
	rangeResponse := httptest.NewRecorder()
	server.Handler().ServeHTTP(rangeResponse, rangeRequest)
	if rangeResponse.Code != http.StatusPartialContent || rangeResponse.Body.String() != "2345" {
		t.Fatalf("range response: %d body=%q", rangeResponse.Code, rangeResponse.Body.String())
	}
	if !strings.Contains(rangeResponse.Header().Get("Cache-Control"), "immutable") {
		t.Fatalf("installer cache header = %q", rangeResponse.Header().Get("Cache-Control"))
	}

	unknownRequest := httptest.NewRequest(http.MethodGet, "/updates/win-x64/private.txt", nil)
	unknownResponse := httptest.NewRecorder()
	server.Handler().ServeHTTP(unknownResponse, unknownRequest)
	if unknownResponse.Code != http.StatusNotFound {
		t.Fatalf("unexpected update file status = %d", unknownResponse.Code)
	}

	vclogg2MetadataRequest := httptest.NewRequest(http.MethodGet, "/updates-vclogg2/win-x64/latest.json", nil)
	vclogg2MetadataResponse := httptest.NewRecorder()
	server.Handler().ServeHTTP(vclogg2MetadataResponse, vclogg2MetadataRequest)
	if vclogg2MetadataResponse.Code != http.StatusOK || vclogg2MetadataResponse.Header().Get("Cache-Control") != "no-store" || !strings.HasPrefix(vclogg2MetadataResponse.Header().Get("Content-Type"), "application/json") {
		t.Fatalf("VCLogg2 manifest response: %d headers=%v", vclogg2MetadataResponse.Code, vclogg2MetadataResponse.Header())
	}
	vclogg2RangeRequest := httptest.NewRequest(http.MethodGet, "/updates-vclogg2/win-x64/vclogg2-2.0.0-win-x64.zip", nil)
	vclogg2RangeRequest.Header.Set("Range", "bytes=2-5")
	vclogg2RangeResponse := httptest.NewRecorder()
	server.Handler().ServeHTTP(vclogg2RangeResponse, vclogg2RangeRequest)
	if vclogg2RangeResponse.Code != http.StatusPartialContent || vclogg2RangeResponse.Body.String() != "logg" || !strings.Contains(vclogg2RangeResponse.Header().Get("Cache-Control"), "immutable") {
		t.Fatalf("VCLogg2 range response: %d body=%q headers=%v", vclogg2RangeResponse.Code, vclogg2RangeResponse.Body.String(), vclogg2RangeResponse.Header())
	}
	crossProductRequest := httptest.NewRequest(http.MethodGet, "/updates-vclogg2/win-x64/VCLogg-0.8.0-Setup-x64.exe", nil)
	crossProductResponse := httptest.NewRecorder()
	server.Handler().ServeHTTP(crossProductResponse, crossProductRequest)
	if crossProductResponse.Code != http.StatusNotFound {
		t.Fatalf("cross-product update file status = %d", crossProductResponse.Code)
	}
}

func TestAdminPublishesValidatedReleasePackage(t *testing.T) {
	database, client := newTestClient(t)
	if err := CreateAdmin(context.Background(), database, "admin", "strong-password"); err != nil {
		t.Fatal(err)
	}
	updateDirectory := t.TempDir()
	cfg := config.Default()
	cfg.ClientAuth.Enabled = false
	cfg.Updates.Directory = updateDirectory
	server, err := NewConfigured(database, cfg)
	if err != nil {
		t.Fatal(err)
	}
	client.handler = server.Handler()
	login := client.request(t, http.MethodPost, "/admin/api/login", map[string]string{"username": "admin", "password": "strong-password"}, false)
	if login.Code != http.StatusOK {
		t.Fatalf("login: %d %s", login.Code, login.Body.String())
	}
	session := decode[map[string]string](t, login)
	cookie := login.Result().Cookies()[0]

	publish := func(version string, installer []byte, manifestInstaller []byte) *httptest.ResponseRecorder {
		body, contentType := releaseUploadBody(t, version, installer, manifestInstaller)
		request := httptest.NewRequest(http.MethodPost, "/admin/api/settings/release", body)
		request.Header.Set("Content-Type", contentType)
		request.Header.Set("X-CSRF-Token", session["csrfToken"])
		request.AddCookie(cookie)
		response := httptest.NewRecorder()
		client.handler.ServeHTTP(response, request)
		return response
	}

	firstInstaller := []byte("first installer")
	first := publish("1.2.3", firstInstaller, firstInstaller)
	if first.Code != http.StatusOK {
		t.Fatalf("publish first release: %d %s", first.Code, first.Body.String())
	}
	secondInstaller := []byte("second installer")
	second := publish("1.2.4", secondInstaller, secondInstaller)
	if second.Code != http.StatusOK {
		t.Fatalf("publish replacement release: %d %s", second.Code, second.Body.String())
	}
	platformDirectory := filepath.Join(updateDirectory, "win-x64")
	manifest, err := os.ReadFile(filepath.Join(platformDirectory, "latest.yml"))
	if err != nil || !strings.Contains(string(manifest), "version: 1.2.4") {
		t.Fatalf("published manifest = %q err=%v", manifest, err)
	}
	for _, version := range []string{"1.2.3", "1.2.4"} {
		if _, err := os.Stat(filepath.Join(platformDirectory, "VCLogg-"+version+"-Setup-x64.exe.blockmap")); err != nil {
			t.Fatalf("historical blockmap %s was not retained: %v", version, err)
		}
	}
	var latestVersion string
	if err := database.DB.QueryRow(`SELECT value FROM settings WHERE key='latest_app_version'`).Scan(&latestVersion); err != nil || latestVersion != "1.2.4" {
		t.Fatalf("latest version = %q err=%v", latestVersion, err)
	}

	invalid := publish("1.2.5", []byte("tampered installer"), []byte("expected installer"))
	if invalid.Code != http.StatusBadRequest || !strings.Contains(invalid.Body.String(), "release_checksum_mismatch") {
		t.Fatalf("checksum mismatch: %d %s", invalid.Code, invalid.Body.String())
	}
	manifest, err = os.ReadFile(filepath.Join(platformDirectory, "latest.yml"))
	if err != nil || !strings.Contains(string(manifest), "version: 1.2.4") {
		t.Fatalf("failed upload changed manifest = %q err=%v", manifest, err)
	}
}

func TestAdminPublishesVCLogg2ReleaseSeparately(t *testing.T) {
	database, client := newTestClient(t)
	if err := CreateAdmin(context.Background(), database, "admin", "strong-password"); err != nil {
		t.Fatal(err)
	}
	legacyUpdateDirectory := t.TempDir()
	vclogg2UpdateDirectory := t.TempDir()
	cfg := config.Default()
	cfg.ClientAuth.Enabled = false
	cfg.Updates.Directory = legacyUpdateDirectory
	cfg.Updates.VCLogg2Directory = vclogg2UpdateDirectory
	server, err := NewConfigured(database, cfg)
	if err != nil {
		t.Fatal(err)
	}
	client.handler = server.Handler()
	login := client.request(t, http.MethodPost, "/admin/api/login", map[string]string{"username": "admin", "password": "strong-password"}, false)
	if login.Code != http.StatusOK {
		t.Fatalf("login: %d %s", login.Code, login.Body.String())
	}
	session := decode[map[string]string](t, login)
	cookie := login.Result().Cookies()[0]
	var originalLegacyVersion string
	if err := database.DB.QueryRow(`SELECT value FROM settings WHERE key='latest_app_version'`).Scan(&originalLegacyVersion); err != nil {
		t.Fatal(err)
	}

	publish := func(version string, archive, manifestArchive []byte) *httptest.ResponseRecorder {
		body, contentType := vclogg2ReleaseUploadBody(t, version, archive, manifestArchive)
		request := httptest.NewRequest(http.MethodPost, "/admin/api/settings/releases/vclogg2", body)
		request.Header.Set("Content-Type", contentType)
		request.Header.Set("X-CSRF-Token", session["csrfToken"])
		request.AddCookie(cookie)
		response := httptest.NewRecorder()
		client.handler.ServeHTTP(response, request)
		return response
	}

	firstArchive := []byte("first VCLogg2 archive")
	if response := publish("2.0.0", firstArchive, firstArchive); response.Code != http.StatusOK {
		t.Fatalf("publish first VCLogg2 release: %d %s", response.Code, response.Body.String())
	}
	secondArchive := []byte("second VCLogg2 archive")
	if response := publish("2.0.1", secondArchive, secondArchive); response.Code != http.StatusOK {
		t.Fatalf("publish replacement VCLogg2 release: %d %s", response.Code, response.Body.String())
	}
	platformDirectory := filepath.Join(vclogg2UpdateDirectory, "win-x64")
	manifest, err := os.ReadFile(filepath.Join(platformDirectory, "latest.json"))
	if err != nil || !strings.Contains(string(manifest), `"version":"2.0.1"`) {
		t.Fatalf("published VCLogg2 manifest = %q err=%v", manifest, err)
	}
	for _, version := range []string{"2.0.0", "2.0.1"} {
		if _, err := os.Stat(filepath.Join(platformDirectory, "vclogg2-"+version+"-win-x64.blockmap.json")); err != nil {
			t.Fatalf("historical VCLogg2 blockmap %s was not retained: %v", version, err)
		}
	}
	if _, err := os.Stat(filepath.Join(legacyUpdateDirectory, "win-x64", "latest.yml")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("VCLogg2 publish touched the VCLogg manifest: %v", err)
	}
	var latestVersion string
	if err := database.DB.QueryRow(`SELECT value FROM settings WHERE key='latest_app_version_vclogg2'`).Scan(&latestVersion); err != nil || latestVersion != "2.0.1" {
		t.Fatalf("latest VCLogg2 version = %q err=%v", latestVersion, err)
	}
	var legacyVersion string
	if err := database.DB.QueryRow(`SELECT value FROM settings WHERE key='latest_app_version'`).Scan(&legacyVersion); err != nil || legacyVersion != originalLegacyVersion {
		t.Fatalf("legacy version setting changed from %q to %q err=%v", originalLegacyVersion, legacyVersion, err)
	}

	invalid := publish("2.0.2", []byte("tampered VCLogg2 archive"), []byte("expected VCLogg2 archive"))
	if invalid.Code != http.StatusBadRequest || !strings.Contains(invalid.Body.String(), "release_checksum_mismatch") {
		t.Fatalf("VCLogg2 checksum mismatch: %d %s", invalid.Code, invalid.Body.String())
	}
	manifest, err = os.ReadFile(filepath.Join(platformDirectory, "latest.json"))
	if err != nil || !strings.Contains(string(manifest), `"version":"2.0.1"`) {
		t.Fatalf("failed VCLogg2 upload changed manifest = %q err=%v", manifest, err)
	}
}

func releaseUploadBody(t *testing.T, version string, installer, manifestInstaller []byte) (*bytes.Buffer, string) {
	t.Helper()
	installerName := "VCLogg-" + version + "-Setup-x64.exe"
	digest := sha512.Sum512(manifestInstaller)
	encodedDigest := base64.StdEncoding.EncodeToString(digest[:])
	manifest := fmt.Sprintf("version: %s\nfiles:\n  - url: %s\n    sha512: %s\n    size: %d\npath: %s\nsha512: %s\nreleaseDate: '2026-08-19T13:06:33.555Z'\n", version, installerName, encodedDigest, len(manifestInstaller), installerName, encodedDigest)
	packageBuffer := &bytes.Buffer{}
	archive := zip.NewWriter(packageBuffer)
	for name, contents := range map[string][]byte{
		"latest.yml":                []byte(manifest),
		installerName:               installer,
		installerName + ".blockmap": []byte("blockmap"),
	} {
		entry, err := archive.Create(name)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := entry.Write(contents); err != nil {
			t.Fatal(err)
		}
	}
	if err := archive.Close(); err != nil {
		t.Fatal(err)
	}
	body := &bytes.Buffer{}
	multipartWriter := multipart.NewWriter(body)
	part, err := multipartWriter.CreateFormFile("package", version+".zip")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := part.Write(packageBuffer.Bytes()); err != nil {
		t.Fatal(err)
	}
	if err := multipartWriter.Close(); err != nil {
		t.Fatal(err)
	}
	return body, multipartWriter.FormDataContentType()
}

func vclogg2ReleaseUploadBody(t *testing.T, version string, artifact, manifestArtifact []byte) (*bytes.Buffer, string) {
	t.Helper()
	artifactName := "vclogg2-" + version + "-win-x64.zip"
	blockmapName := "vclogg2-" + version + "-win-x64.blockmap.json"
	manifestDigest := sha256.Sum256(manifestArtifact)
	chunkDigest := sha256.Sum256(artifact)
	manifest, err := json.Marshal(map[string]any{
		"schemaVersion": 1,
		"product":       "VCLogg2",
		"version":       version,
		"platform":      "windows",
		"architecture":  "x86_64",
		"artifact":      artifactName,
		"sha256":        fmt.Sprintf("%x", manifestDigest),
		"size":          len(manifestArtifact),
		"blockmap":      blockmapName,
	})
	if err != nil {
		t.Fatal(err)
	}
	blockmap, err := json.Marshal(map[string]any{
		"schemaVersion": 1,
		"algorithm":     "sha256",
		"chunkSize":     1024 * 1024,
		"file":          artifactName,
		"chunks":        []string{fmt.Sprintf("%x", chunkDigest)},
	})
	if err != nil {
		t.Fatal(err)
	}
	packageBuffer := &bytes.Buffer{}
	archive := zip.NewWriter(packageBuffer)
	for name, contents := range map[string][]byte{
		"latest.json": manifest,
		artifactName:  artifact,
		blockmapName:  blockmap,
	} {
		entry, err := archive.Create(name)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := entry.Write(contents); err != nil {
			t.Fatal(err)
		}
	}
	if err := archive.Close(); err != nil {
		t.Fatal(err)
	}
	body := &bytes.Buffer{}
	multipartWriter := multipart.NewWriter(body)
	part, err := multipartWriter.CreateFormFile("package", "vclogg2-"+version+"-release.zip")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := part.Write(packageBuffer.Bytes()); err != nil {
		t.Fatal(err)
	}
	if err := multipartWriter.Close(); err != nil {
		t.Fatal(err)
	}
	return body, multipartWriter.FormDataContentType()
}

func TestAdminCanDisableAndRestoreFilter(t *testing.T) {
	database, client := newTestClient(t)
	register(t, client)
	shared := client.request(t, http.MethodPost, "/api/v1/filters/batch", map[string]any{
		"items": []map[string]any{{
			"clientFilterId": "managed-filter",
			"name":           "Managed",
			"value":          "MANAGED",
			"useRegex":       false,
			"note":           "managed by an administrator",
		}},
	}, true)
	if shared.Code != http.StatusOK {
		t.Fatalf("share: %d %s", shared.Code, shared.Body.String())
	}
	var filterID string
	if err := database.DB.QueryRow(`SELECT id FROM filters WHERE client_filter_id='managed-filter'`).Scan(&filterID); err != nil {
		t.Fatal(err)
	}
	if err := CreateAdmin(context.Background(), database, "admin", "strong-password"); err != nil {
		t.Fatal(err)
	}
	login := client.request(t, http.MethodPost, "/admin/api/login", map[string]string{
		"username": "admin",
		"password": "strong-password",
	}, false)
	if login.Code != http.StatusOK {
		t.Fatalf("login: %d %s", login.Code, login.Body.String())
	}
	session := decode[map[string]string](t, login)
	cookie := login.Result().Cookies()[0]

	setStatus := func(status string) {
		t.Helper()
		request := httptest.NewRequest(http.MethodPatch, "/admin/api/filters/"+filterID, bytes.NewBufferString(`{"status":"`+status+`"}`))
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("X-CSRF-Token", session["csrfToken"])
		request.AddCookie(cookie)
		response := httptest.NewRecorder()
		client.handler.ServeHTTP(response, request)
		if response.Code != http.StatusOK {
			t.Fatalf("set status %s: %d %s", status, response.Code, response.Body.String())
		}
	}

	setStatus("disabled")
	list := client.request(t, http.MethodGet, "/api/v1/filters", nil, true)
	if decode[struct {
		Total int `json:"total"`
	}](t, list).Total != 0 {
		t.Fatal("disabled filter remained visible to clients")
	}
	setStatus("active")
	list = client.request(t, http.MethodGet, "/api/v1/filters", nil, true)
	if decode[struct {
		Total int `json:"total"`
	}](t, list).Total != 1 {
		t.Fatal("restored filter was not visible to clients")
	}
}

func TestAdminFilterEditRejectsStaleAndAllowsEqualContent(t *testing.T) {
	database, client := newTestClient(t)
	register(t, client)
	shared := client.request(t, http.MethodPost, "/api/v1/filters/batch", map[string]any{
		"items": []map[string]any{
			{"clientFilterId": "managed-a", "name": "Managed A", "value": "MANAGED_A", "useRegex": false, "note": ""},
			{"clientFilterId": "managed-b", "name": "Managed B", "value": "MANAGED_B", "useRegex": false, "note": ""},
		},
	}, true)
	if shared.Code != http.StatusOK {
		t.Fatalf("share: %d %s", shared.Code, shared.Body.String())
	}
	var firstID string
	if err := database.DB.QueryRow(`SELECT id FROM filters WHERE client_filter_id='managed-a'`).Scan(&firstID); err != nil {
		t.Fatal(err)
	}
	if err := CreateAdmin(context.Background(), database, "admin", "strong-password"); err != nil {
		t.Fatal(err)
	}
	admin := loginAdministrator(t, client.handler, "admin", "strong-password")
	stale := adminRequest(t, client.handler, http.MethodPatch, "/admin/api/filters/"+firstID, map[string]any{
		"name": "Changed", "value": "CHANGED", "useRegex": false, "note": "", "baseRevision": 0,
	}, admin)
	if stale.Code != http.StatusConflict || !strings.Contains(stale.Body.String(), "revision_conflict") {
		t.Fatalf("stale administrator edit: %d %s", stale.Code, stale.Body.String())
	}
	equalContent := adminRequest(t, client.handler, http.MethodPatch, "/admin/api/filters/"+firstID, map[string]any{
		"name": "Managed B", "value": "MANAGED_B", "useRegex": false, "note": "", "baseRevision": 1,
	}, admin)
	if equalContent.Code != http.StatusOK {
		t.Fatalf("equal-content administrator edit: %d %s", equalContent.Code, equalContent.Body.String())
	}
}
