package app

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

type adminTestSession struct {
	ID     string
	Cookie *http.Cookie
	CSRF   string
}

func loginAdministrator(t *testing.T, handler http.Handler, username, password string) adminTestSession {
	t.Helper()
	response := adminRequest(t, handler, http.MethodPost, "/admin/api/login", map[string]string{"username": username, "password": password}, adminTestSession{})
	if response.Code != http.StatusOK {
		t.Fatalf("administrator login: %d %s", response.Code, response.Body.String())
	}
	login := decode[struct {
		CSRF string `json:"csrfToken"`
	}](t, response)
	cookies := response.Result().Cookies()
	if len(cookies) != 1 {
		t.Fatalf("administrator login cookies: %#v", cookies)
	}
	session := adminTestSession{Cookie: cookies[0], CSRF: login.CSRF}
	infoResponse := adminRequest(t, handler, http.MethodGet, "/admin/api/session", nil, session)
	info := decode[struct {
		ID string `json:"id"`
	}](t, infoResponse)
	session.ID = info.ID
	return session
}

func adminRequest(t *testing.T, handler http.Handler, method, path string, body any, session adminTestSession) *httptest.ResponseRecorder {
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
	if session.Cookie != nil {
		request.AddCookie(session.Cookie)
	}
	if session.CSRF != "" && method != http.MethodGet && method != http.MethodHead {
		request.Header.Set("X-CSRF-Token", session.CSRF)
	}
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	return response
}

func TestAdministratorRBACUpdatesImmediatelyAndProtectsFinalSuperAdmin(t *testing.T) {
	database, client := newTestClient(t)
	if err := CreateAdmin(context.Background(), database, "root", "strong-password"); err != nil {
		t.Fatal(err)
	}
	super := loginAdministrator(t, client.handler, "root", "strong-password")
	createRole := adminRequest(t, client.handler, http.MethodPost, "/admin/api/roles", map[string]any{
		"name": "身份审阅员", "description": "只能查看访问身份", "permissions": []string{"identities.read"},
	}, super)
	if createRole.Code != http.StatusCreated {
		t.Fatalf("create role: %d %s", createRole.Code, createRole.Body.String())
	}
	role := decode[struct {
		ID string `json:"id"`
	}](t, createRole)
	createAdmin := adminRequest(t, client.handler, http.MethodPost, "/admin/api/administrators", map[string]any{
		"username": "auditor", "password": "auditor-password", "roleIds": []string{role.ID},
	}, super)
	if createAdmin.Code != http.StatusCreated {
		t.Fatalf("create administrator: %d %s", createAdmin.Code, createAdmin.Body.String())
	}
	auditor := loginAdministrator(t, client.handler, "auditor", "auditor-password")
	allowed := adminRequest(t, client.handler, http.MethodGet, "/admin/api/identities", nil, auditor)
	if allowed.Code != http.StatusOK {
		t.Fatalf("allowed permission: %d %s", allowed.Code, allowed.Body.String())
	}
	denied := adminRequest(t, client.handler, http.MethodGet, "/admin/api/dashboard", nil, auditor)
	if denied.Code != http.StatusForbidden || !strings.Contains(denied.Body.String(), "permission_denied") {
		t.Fatalf("denied permission: %d %s", denied.Code, denied.Body.String())
	}
	updateRole := adminRequest(t, client.handler, http.MethodPut, "/admin/api/roles/"+role.ID, map[string]any{
		"name": "仪表盘审阅员", "description": "权限变更立即生效", "permissions": []string{"dashboard.read"},
	}, super)
	if updateRole.Code != http.StatusOK {
		t.Fatalf("update role: %d %s", updateRole.Code, updateRole.Body.String())
	}
	allowed = adminRequest(t, client.handler, http.MethodGet, "/admin/api/dashboard", nil, auditor)
	if allowed.Code != http.StatusOK {
		t.Fatalf("updated permission was not immediate: %d %s", allowed.Code, allowed.Body.String())
	}
	lastSuper := adminRequest(t, client.handler, http.MethodPatch, "/admin/api/administrators/"+super.ID, map[string]string{"status": "disabled"}, super)
	if lastSuper.Code != http.StatusConflict || !strings.Contains(lastSuper.Body.String(), "last_super_administrator") {
		t.Fatalf("final super administrator was not protected: %d %s", lastSuper.Code, lastSuper.Body.String())
	}
	var auditCount int
	if err := database.DB.QueryRow(`SELECT COUNT(*) FROM admin_audit_logs WHERE action='authorization.denied' AND result='denied'`).Scan(&auditCount); err != nil || auditCount != 1 {
		t.Fatalf("denied audit count = %d, err=%v", auditCount, err)
	}
}

func TestAdminSPAHistoryFallbackDoesNotMaskMissingAssetsOrAPIs(t *testing.T) {
	_, client := newTestClient(t)
	page := client.request(t, http.MethodGet, "/admin/roles", nil, false)
	if page.Code != http.StatusOK || !strings.Contains(page.Body.String(), `<div id="app"></div>`) {
		t.Fatalf("SPA fallback: %d %s", page.Code, page.Body.String())
	}
	asset := client.request(t, http.MethodGet, "/admin/assets/missing.js", nil, false)
	if asset.Code != http.StatusNotFound {
		t.Fatalf("missing asset = %d", asset.Code)
	}
	api := client.request(t, http.MethodGet, "/admin/api/missing", nil, false)
	if api.Code != http.StatusNotFound || strings.Contains(api.Body.String(), `<div id="app"></div>`) {
		t.Fatalf("missing API was masked: %d %s", api.Code, api.Body.String())
	}
}

func TestFilterExportUsesIndependentPermission(t *testing.T) {
	database, client := newTestClient(t)
	if err := CreateAdmin(context.Background(), database, "root", "strong-password"); err != nil {
		t.Fatal(err)
	}
	super := loginAdministrator(t, client.handler, "root", "strong-password")
	roleResponse := adminRequest(t, client.handler, http.MethodPost, "/admin/api/roles", map[string]any{
		"name": "关键词导出员", "description": "只能导出关键词", "permissions": []string{"filters.export"},
	}, super)
	role := decode[struct {
		ID string `json:"id"`
	}](t, roleResponse)
	create := adminRequest(t, client.handler, http.MethodPost, "/admin/api/administrators", map[string]any{
		"username": "exporter", "password": "exporter-password", "roleIds": []string{role.ID},
	}, super)
	if create.Code != http.StatusCreated {
		t.Fatalf("create exporter: %d %s", create.Code, create.Body.String())
	}
	exporter := loginAdministrator(t, client.handler, "exporter", "exporter-password")
	exported := adminRequest(t, client.handler, http.MethodGet, "/admin/api/filters/export?format=csv&status=all", nil, exporter)
	if exported.Code != http.StatusOK || !strings.HasPrefix(exported.Body.String(), "\ufeffid,status,revision") {
		t.Fatalf("export response: %d %q", exported.Code, exported.Body.String())
	}
	denied := adminRequest(t, client.handler, http.MethodGet, "/admin/api/filters", nil, exporter)
	if denied.Code != http.StatusForbidden {
		t.Fatalf("filters.read without permission: %d %s", denied.Code, denied.Body.String())
	}
}
