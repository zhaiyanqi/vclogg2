package config

import (
	"crypto/ed25519"
	"encoding/base64"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"

	"gopkg.in/yaml.v3"
)

type Config struct {
	Listen       string           `yaml:"listen"`
	DataDir      string           `yaml:"data_dir"`
	TrustedProxy bool             `yaml:"trusted_proxy"`
	Updates      UpdateConfig     `yaml:"updates"`
	ClientAuth   ClientAuthConfig `yaml:"client_auth"`
	Uploads      UploadConfig     `yaml:"uploads"`
}

type UpdateConfig struct {
	Enabled          bool   `yaml:"enabled"`
	Directory        string `yaml:"directory"`
	VCLogg2Directory string `yaml:"vclogg2_directory"`
}

type ClientAuthConfig struct {
	Enabled                 bool              `yaml:"enabled"`
	AcceptBundledPublicKeys bool              `yaml:"accept_bundled_public_keys"`
	MaxClockSkewSeconds     int               `yaml:"max_clock_skew_seconds"`
	NonceTTLSeconds         int               `yaml:"nonce_ttl_seconds"`
	SessionTTLHours         int               `yaml:"session_ttl_hours"`
	MaxSessionsPerUser      int               `yaml:"max_sessions_per_identity"`
	PublicKeys              []ClientPublicKey `yaml:"public_keys"`
}

// These values are populated only in standard distribution builds through
// Go linker variables. Development builds intentionally leave them empty.
var BundledClientKeyID string
var BundledClientPublicKey string

type ClientPublicKey struct {
	ID        string `yaml:"id"`
	PublicKey string `yaml:"public_key"`
}

type UploadConfig struct {
	WindowSeconds       int   `yaml:"window_seconds"`
	MaxRequests         int   `yaml:"max_requests_per_window"`
	MaxFiltersPerUser   int   `yaml:"max_filters_per_user"`
	MaxBytesPerUser     int64 `yaml:"max_bytes_per_user"`
	MaxItemsPerRequest  int   `yaml:"max_items_per_request"`
	MaxRequestBodyBytes int64 `yaml:"max_request_body_bytes"`
}

func Default() Config {
	return Config{
		Listen:  "127.0.0.1:8787",
		DataDir: "./data",
		Updates: UpdateConfig{
			Enabled:          true,
			Directory:        "./data/updates",
			VCLogg2Directory: "./data/updates-vclogg2",
		},
		ClientAuth: ClientAuthConfig{
			Enabled:                 true,
			AcceptBundledPublicKeys: true,
			MaxClockSkewSeconds:     300,
			NonceTTLSeconds:         600,
			SessionTTLHours:         720,
			MaxSessionsPerUser:      5,
		},
		Uploads: UploadConfig{
			WindowSeconds:       60,
			MaxRequests:         10,
			MaxFiltersPerUser:   1000,
			MaxBytesPerUser:     10 * 1024 * 1024,
			MaxItemsPerRequest:  100,
			MaxRequestBodyBytes: 512 * 1024,
		},
	}
}

func Load(path string) (Config, error) {
	cfg := Default()
	data, err := os.ReadFile(path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return cfg, nil
		}
		return Config{}, err
	}
	cfg.Updates.VCLogg2Directory = ""
	if err := yaml.Unmarshal(data, &cfg); err != nil {
		return Config{}, err
	}
	if cfg.Listen == "" {
		cfg.Listen = Default().Listen
	}
	if cfg.DataDir == "" {
		cfg.DataDir = Default().DataDir
	}
	if cfg.Updates.Directory == "" {
		cfg.Updates.Directory = Default().Updates.Directory
	}
	if cfg.Updates.VCLogg2Directory == "" {
		updatesDirectory := filepath.Clean(cfg.Updates.Directory)
		cfg.Updates.VCLogg2Directory = filepath.Join(
			filepath.Dir(updatesDirectory),
			filepath.Base(updatesDirectory)+"-vclogg2",
		)
	}
	if err := cfg.validate(); err != nil {
		return Config{}, err
	}
	if !filepath.IsAbs(cfg.DataDir) {
		cfg.DataDir = filepath.Clean(filepath.Join(filepath.Dir(path), cfg.DataDir))
	}
	if !filepath.IsAbs(cfg.Updates.Directory) {
		cfg.Updates.Directory = filepath.Clean(filepath.Join(filepath.Dir(path), cfg.Updates.Directory))
	}
	if !filepath.IsAbs(cfg.Updates.VCLogg2Directory) {
		cfg.Updates.VCLogg2Directory = filepath.Clean(filepath.Join(filepath.Dir(path), cfg.Updates.VCLogg2Directory))
	}
	if strings.EqualFold(filepath.Clean(cfg.Updates.Directory), filepath.Clean(cfg.Updates.VCLogg2Directory)) {
		return Config{}, errors.New("updates.directory and updates.vclogg2_directory must be different")
	}
	return cfg, nil
}

var keyIDPattern = regexp.MustCompile(`^[A-Za-z0-9._-]{1,64}$`)

func (cfg Config) validate() error {
	if cfg.ClientAuth.MaxClockSkewSeconds < 1 || cfg.ClientAuth.MaxClockSkewSeconds > 3600 {
		return errors.New("client_auth.max_clock_skew_seconds must be between 1 and 3600")
	}
	if cfg.ClientAuth.NonceTTLSeconds < cfg.ClientAuth.MaxClockSkewSeconds || cfg.ClientAuth.NonceTTLSeconds > 86400 {
		return errors.New("client_auth.nonce_ttl_seconds must be at least max_clock_skew_seconds and at most 86400")
	}
	if cfg.ClientAuth.SessionTTLHours < 1 || cfg.ClientAuth.SessionTTLHours > 8760 {
		return errors.New("client_auth.session_ttl_hours must be between 1 and 8760")
	}
	if cfg.ClientAuth.MaxSessionsPerUser < 1 || cfg.ClientAuth.MaxSessionsPerUser > 100 {
		return errors.New("client_auth.max_sessions_per_identity must be between 1 and 100")
	}
	seen := make(map[string]struct{}, len(cfg.ClientAuth.PublicKeys)+1)
	if cfg.ClientAuth.AcceptBundledPublicKeys && (BundledClientKeyID != "" || BundledClientPublicKey != "") {
		if err := validateClientPublicKey(ClientPublicKey{ID: BundledClientKeyID, PublicKey: BundledClientPublicKey}); err != nil {
			return fmt.Errorf("bundled %w", err)
		}
		seen[BundledClientKeyID] = struct{}{}
	}
	for _, item := range cfg.ClientAuth.PublicKeys {
		if err := validateClientPublicKey(item); err != nil {
			return err
		}
		if _, exists := seen[item.ID]; exists {
			return fmt.Errorf("client_auth public key id %q conflicts with another configured or bundled key", item.ID)
		}
		seen[item.ID] = struct{}{}
	}
	if cfg.Uploads.WindowSeconds < 1 || cfg.Uploads.WindowSeconds > 86400 {
		return errors.New("uploads.window_seconds must be between 1 and 86400")
	}
	if cfg.Uploads.MaxRequests < 1 || cfg.Uploads.MaxRequests > 100000 {
		return errors.New("uploads.max_requests_per_window must be between 1 and 100000")
	}
	if cfg.Uploads.MaxFiltersPerUser < 1 || cfg.Uploads.MaxFiltersPerUser > 1000000 {
		return errors.New("uploads.max_filters_per_user must be between 1 and 1000000")
	}
	if cfg.Uploads.MaxBytesPerUser < 1024 || cfg.Uploads.MaxBytesPerUser > 1024*1024*1024 {
		return errors.New("uploads.max_bytes_per_user must be between 1024 and 1073741824")
	}
	if cfg.Uploads.MaxItemsPerRequest < 1 || cfg.Uploads.MaxItemsPerRequest > 1000 {
		return errors.New("uploads.max_items_per_request must be between 1 and 1000")
	}
	if cfg.Uploads.MaxRequestBodyBytes < 1024 || cfg.Uploads.MaxRequestBodyBytes > 4*1024*1024 {
		return errors.New("uploads.max_request_body_bytes must be between 1024 and 4194304")
	}
	return nil
}

func validateClientPublicKey(item ClientPublicKey) error {
	if !keyIDPattern.MatchString(item.ID) {
		return fmt.Errorf("client_auth public key id %q must contain only letters, digits, dot, underscore, or dash", item.ID)
	}
	decoded, err := base64.RawURLEncoding.DecodeString(item.PublicKey)
	if err != nil || len(decoded) != ed25519.PublicKeySize {
		return fmt.Errorf("client_auth public key %q must be a 32-byte base64url Ed25519 public key", item.ID)
	}
	return nil
}

func (cfg Config) ClientPublicKeys() (map[string]ed25519.PublicKey, error) {
	result := make(map[string]ed25519.PublicKey, len(cfg.ClientAuth.PublicKeys)+1)
	if cfg.ClientAuth.AcceptBundledPublicKeys && BundledClientKeyID != "" && BundledClientPublicKey != "" {
		decoded, err := base64.RawURLEncoding.DecodeString(BundledClientPublicKey)
		if err != nil || len(decoded) != ed25519.PublicKeySize {
			return nil, fmt.Errorf("invalid bundled Ed25519 public key %q", BundledClientKeyID)
		}
		result[BundledClientKeyID] = ed25519.PublicKey(decoded)
	}
	for _, item := range cfg.ClientAuth.PublicKeys {
		if _, exists := result[item.ID]; exists {
			return nil, fmt.Errorf("client_auth public key id %q conflicts with a bundled key", item.ID)
		}
		decoded, err := base64.RawURLEncoding.DecodeString(item.PublicKey)
		if err != nil || len(decoded) != ed25519.PublicKeySize {
			return nil, fmt.Errorf("invalid Ed25519 public key %q", item.ID)
		}
		result[item.ID] = ed25519.PublicKey(decoded)
	}
	if cfg.ClientAuth.Enabled && len(result) == 0 {
		return nil, errors.New("client_auth is enabled but no bundled or configured public_keys are available")
	}
	return result, nil
}
