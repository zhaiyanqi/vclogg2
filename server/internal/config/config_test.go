package config

import (
	"crypto/ed25519"
	"crypto/rand"
	"encoding/base64"
	"os"
	"path/filepath"
	"testing"
)

func TestLoadAppliesSecurityDefaultsAndResolvesDataDirectory(t *testing.T) {
	publicKey, _, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	directory := t.TempDir()
	path := filepath.Join(directory, "config.yaml")
	contents := "client_auth:\n  public_keys:\n    - id: production-v1\n      public_key: " + base64.RawURLEncoding.EncodeToString(publicKey) + "\n"
	if err := os.WriteFile(path, []byte(contents), 0o600); err != nil {
		t.Fatal(err)
	}
	cfg, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}
	if !cfg.ClientAuth.Enabled || cfg.Uploads.MaxRequests != 10 {
		t.Fatalf("defaults were not retained: %#v", cfg)
	}
	if cfg.DataDir != filepath.Join(directory, "data") {
		t.Fatalf("data dir = %q", cfg.DataDir)
	}
	if cfg.Updates.Directory != filepath.Join(directory, "data", "updates") ||
		cfg.Updates.VCLogg2Directory != filepath.Join(directory, "data", "updates-vclogg2") ||
		!cfg.Updates.Enabled {
		t.Fatalf("updates config = %#v", cfg.Updates)
	}
	if _, err := cfg.ClientPublicKeys(); err != nil {
		t.Fatal(err)
	}
}

func TestLoadPlacesVCLogg2UpdatesBesideConfiguredVCLoggDirectory(t *testing.T) {
	directory := t.TempDir()
	path := filepath.Join(directory, "config.yaml")
	contents := "updates:\n  directory: ./releases/vclogg\nclient_auth:\n  enabled: false\n"
	if err := os.WriteFile(path, []byte(contents), 0o600); err != nil {
		t.Fatal(err)
	}
	cfg, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Updates.Directory != filepath.Join(directory, "releases", "vclogg") ||
		cfg.Updates.VCLogg2Directory != filepath.Join(directory, "releases", "vclogg-vclogg2") {
		t.Fatalf("updates config = %#v", cfg.Updates)
	}
}

func TestEnabledClientAuthenticationRequiresAKeyForServing(t *testing.T) {
	setBundledClientKeyForTest(t, "", "")
	cfg := Default()
	if _, err := cfg.ClientPublicKeys(); err == nil {
		t.Fatal("missing client public keys were accepted")
	}
	cfg.ClientAuth.Enabled = false
	if _, err := cfg.ClientPublicKeys(); err != nil {
		t.Fatalf("disabled client authentication rejected: %v", err)
	}
}

func TestBundledAndConfiguredClientPublicKeysAreMerged(t *testing.T) {
	bundledPublic, _, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	configuredPublic, _, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	setBundledClientKeyForTest(t, "official-v1", base64.RawURLEncoding.EncodeToString(bundledPublic))
	standard := Default()
	standardKeys, err := standard.ClientPublicKeys()
	if err != nil || len(standardKeys) != 1 || !standardKeys["official-v1"].Equal(bundledPublic) {
		t.Fatalf("standard distribution did not accept its bundled key: keys=%#v err=%v", standardKeys, err)
	}
	standard.ClientAuth.AcceptBundledPublicKeys = false
	if _, err := standard.ClientPublicKeys(); err == nil {
		t.Fatal("disabling bundled keys left an empty authenticated server usable")
	}

	cfg := Default()
	cfg.ClientAuth.PublicKeys = []ClientPublicKey{{
		ID:        "custom-v1",
		PublicKey: base64.RawURLEncoding.EncodeToString(configuredPublic),
	}}
	keys, err := cfg.ClientPublicKeys()
	if err != nil {
		t.Fatal(err)
	}
	if len(keys) != 2 || !keys["official-v1"].Equal(bundledPublic) || !keys["custom-v1"].Equal(configuredPublic) {
		t.Fatalf("unexpected merged keys: %#v", keys)
	}

	cfg.ClientAuth.AcceptBundledPublicKeys = false
	keys, err = cfg.ClientPublicKeys()
	if err != nil {
		t.Fatal(err)
	}
	if len(keys) != 1 || keys["official-v1"] != nil {
		t.Fatalf("bundled key was not disabled: %#v", keys)
	}
}

func TestConfiguredClientKeyCannotShadowBundledKey(t *testing.T) {
	publicKey, _, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	encoded := base64.RawURLEncoding.EncodeToString(publicKey)
	setBundledClientKeyForTest(t, "official-v1", encoded)
	cfg := Default()
	cfg.ClientAuth.PublicKeys = []ClientPublicKey{{ID: "official-v1", PublicKey: encoded}}
	if err := cfg.validate(); err == nil {
		t.Fatal("configured key was allowed to shadow the bundled key")
	}
}

func setBundledClientKeyForTest(t *testing.T, id, publicKey string) {
	t.Helper()
	previousID, previousKey := BundledClientKeyID, BundledClientPublicKey
	BundledClientKeyID, BundledClientPublicKey = id, publicKey
	t.Cleanup(func() {
		BundledClientKeyID, BundledClientPublicKey = previousID, previousKey
	})
}
