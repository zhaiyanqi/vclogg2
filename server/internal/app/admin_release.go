package app

import (
	"archive/zip"
	"crypto/sha256"
	"crypto/sha512"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"gopkg.in/yaml.v3"
)

const (
	maxReleaseUploadBytes   int64 = 512 * 1024 * 1024
	maxReleaseManifestBytes int64 = 64 * 1024
	maxReleaseInstallerSize int64 = 512 * 1024 * 1024
	maxReleaseBlockmapSize  int64 = 16 * 1024 * 1024
)

type releaseManifest struct {
	Version     string                `yaml:"version"`
	Files       []releaseManifestFile `yaml:"files"`
	Path        string                `yaml:"path"`
	SHA512      string                `yaml:"sha512"`
	ReleaseDate string                `yaml:"releaseDate"`
}

type releaseManifestFile struct {
	URL    string `yaml:"url"`
	SHA512 string `yaml:"sha512"`
	Size   int64  `yaml:"size"`
}

type vclogg2ReleaseManifest struct {
	SchemaVersion int    `json:"schemaVersion"`
	Product       string `json:"product"`
	Version       string `json:"version"`
	Platform      string `json:"platform"`
	Architecture  string `json:"architecture"`
	Artifact      string `json:"artifact"`
	SHA256        string `json:"sha256"`
	Size          int64  `json:"size"`
	Blockmap      string `json:"blockmap"`
}

type vclogg2ReleaseBlockmap struct {
	SchemaVersion int      `json:"schemaVersion"`
	Algorithm     string   `json:"algorithm"`
	ChunkSize     int64    `json:"chunkSize"`
	File          string   `json:"file"`
	Chunks        []string `json:"chunks"`
}

type publishedRelease struct {
	Version       string `json:"version"`
	InstallerName string `json:"installerName"`
	InstallerSize int64  `json:"installerSize"`
	PublishedAt   int64  `json:"publishedAt"`
}

type releaseHTTPError struct {
	status  int
	code    string
	message string
}

func (err *releaseHTTPError) Error() string { return err.message }

func releaseError(status int, code, message string) error {
	return &releaseHTTPError{status: status, code: code, message: message}
}

func (s *Server) adminGetRelease(w http.ResponseWriter, r *http.Request) {
	s.adminGetProductReleaseFor(w, r, "vclogg")
}

func (s *Server) adminGetProductRelease(w http.ResponseWriter, r *http.Request) {
	s.adminGetProductReleaseFor(w, r, r.PathValue("product"))
}

func (s *Server) adminGetProductReleaseFor(w http.ResponseWriter, r *http.Request, product string) {
	if _, ok := s.requirePermission(w, r, "settings.version.read", false); !ok {
		return
	}
	var (
		release  *publishedRelease
		err      error
		feedPath string
	)
	switch product {
	case "vclogg":
		release, err = s.currentPublishedRelease()
		feedPath = "/updates/win-x64/"
	case "vclogg2":
		release, err = s.currentPublishedVCLogg2Release()
		feedPath = "/updates-vclogg2/win-x64/"
	default:
		writeError(w, http.StatusBadRequest, "invalid_release_product", "release product must be vclogg or vclogg2")
		return
	}
	result := map[string]any{
		"enabled":        s.config.Updates.Enabled,
		"maxUploadBytes": maxReleaseUploadBytes,
		"product":        product,
		"feedPath":       feedPath,
	}
	if err == nil {
		result["release"] = release
	} else if !errors.Is(err, os.ErrNotExist) {
		result["warning"] = "当前更新清单无效，请重新发布发行包"
	}
	writeJSON(w, http.StatusOK, result)
}

func (s *Server) adminPublishRelease(w http.ResponseWriter, r *http.Request) {
	s.adminPublishProductReleaseFor(w, r, "vclogg")
}

func (s *Server) adminPublishProductRelease(w http.ResponseWriter, r *http.Request) {
	s.adminPublishProductReleaseFor(w, r, r.PathValue("product"))
}

func (s *Server) adminPublishProductReleaseFor(w http.ResponseWriter, r *http.Request, product string) {
	session, ok := s.requirePermission(w, r, "settings.version.update", true)
	if !ok {
		return
	}
	if product != "vclogg" && product != "vclogg2" {
		writeError(w, http.StatusBadRequest, "invalid_release_product", "release product must be vclogg or vclogg2")
		return
	}
	if !s.config.Updates.Enabled {
		writeError(w, http.StatusConflict, "updates_disabled", "software updates are disabled in server configuration")
		return
	}

	s.releaseMu.Lock()
	defer s.releaseMu.Unlock()

	var (
		release   *publishedRelease
		updatedAt int64
		err       error
	)
	if product == "vclogg2" {
		release, updatedAt, err = s.publishVCLogg2ReleasePackage(w, r)
	} else {
		release, updatedAt, err = s.publishReleasePackage(w, r)
	}
	if err != nil {
		var clientErr *releaseHTTPError
		if errors.As(err, &clientErr) {
			writeError(w, clientErr.status, clientErr.code, clientErr.message)
			return
		}
		writeError(w, http.StatusInternalServerError, "release_publish_failed", err.Error())
		return
	}
	s.audit(r, session.AdminID, session.Username, "settings.release_publish", "release", release.Version, "success", map[string]any{
		"installerName": release.InstallerName,
		"installerSize": release.InstallerSize,
		"product":       product,
	})
	writeJSON(w, http.StatusOK, map[string]any{
		"latestVersion": release.Version,
		"updatedAt":     updatedAt,
		"release":       release,
		"product":       product,
	})
}

func (s *Server) publishReleasePackage(w http.ResponseWriter, r *http.Request) (*publishedRelease, int64, error) {
	platformDirectory := filepath.Join(s.config.Updates.Directory, "win-x64")
	if err := os.MkdirAll(platformDirectory, 0o750); err != nil {
		return nil, 0, fmt.Errorf("create update directory: %w", err)
	}
	r.Body = http.MaxBytesReader(w, r.Body, maxReleaseUploadBytes+1024*1024)
	reader, err := r.MultipartReader()
	if err != nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_upload", "a multipart release package is required")
	}
	temporaryPackage, err := os.CreateTemp(platformDirectory, ".release-upload-*.zip")
	if err != nil {
		return nil, 0, fmt.Errorf("create temporary release package: %w", err)
	}
	temporaryPackagePath := temporaryPackage.Name()
	defer os.Remove(temporaryPackagePath)
	defer temporaryPackage.Close()

	foundPackage := false
	for {
		part, nextErr := reader.NextPart()
		if errors.Is(nextErr, io.EOF) {
			break
		}
		if nextErr != nil {
			return nil, 0, releaseError(http.StatusRequestEntityTooLarge, "release_package_too_large", "the release package exceeds the 512 MiB limit")
		}
		if part.FormName() != "package" || part.FileName() == "" || foundPackage {
			part.Close()
			return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_upload", "the request must contain exactly one package file")
		}
		foundPackage = true
		written, copyErr := io.Copy(temporaryPackage, part)
		part.Close()
		if copyErr != nil {
			return nil, 0, releaseError(http.StatusRequestEntityTooLarge, "release_package_too_large", "the release package exceeds the 512 MiB limit")
		}
		if written <= 0 || written > maxReleaseUploadBytes {
			return nil, 0, releaseError(http.StatusRequestEntityTooLarge, "release_package_too_large", "the release package must be between 1 byte and 512 MiB")
		}
	}
	if !foundPackage {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_upload", "the release package file is missing")
	}
	if err := temporaryPackage.Sync(); err != nil {
		return nil, 0, fmt.Errorf("flush temporary release package: %w", err)
	}
	info, err := temporaryPackage.Stat()
	if err != nil {
		return nil, 0, fmt.Errorf("inspect temporary release package: %w", err)
	}
	archive, err := zip.NewReader(temporaryPackage, info.Size())
	if err != nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the uploaded file is not a valid ZIP package")
	}

	entries := make(map[string]*zip.File, len(archive.File))
	for _, entry := range archive.File {
		if entry.FileInfo().IsDir() {
			continue
		}
		if entry.Name != filepath.Base(entry.Name) || strings.ContainsAny(entry.Name, `/\\`) || !entry.FileInfo().Mode().IsRegular() {
			return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "release files must be regular files at the ZIP root")
		}
		if _, duplicate := entries[entry.Name]; duplicate {
			return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the release package contains duplicate files")
		}
		entries[entry.Name] = entry
	}
	manifestEntry := entries["latest.yml"]
	if manifestEntry == nil || manifestEntry.UncompressedSize64 > uint64(maxReleaseManifestBytes) {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_manifest", "latest.yml is missing or too large")
	}
	manifestBytes, err := readZipEntry(manifestEntry, maxReleaseManifestBytes)
	if err != nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_manifest", "latest.yml could not be read")
	}
	var manifest releaseManifest
	if err := yaml.Unmarshal(manifestBytes, &manifest); err != nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_manifest", "latest.yml is not valid YAML")
	}
	manifest.Version = strings.TrimSpace(manifest.Version)
	if !semverPattern.MatchString(manifest.Version) {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_manifest", "latest.yml contains an invalid semantic version")
	}
	installerName := "VCLogg-" + manifest.Version + "-Setup-x64.exe"
	blockmapName := installerName + ".blockmap"
	if manifest.Path != installerName || len(manifest.Files) != 1 || manifest.Files[0].URL != installerName || manifest.Files[0].Size <= 0 || manifest.Files[0].Size > maxReleaseInstallerSize || manifest.Files[0].SHA512 != manifest.SHA512 {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_manifest", "latest.yml does not describe exactly one matching x64 installer")
	}
	if _, err := decodeSHA512(manifest.SHA512); err != nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_manifest", "latest.yml contains an invalid SHA-512 digest")
	}
	installerEntry, blockmapEntry := entries[installerName], entries[blockmapName]
	if len(entries) != 3 || installerEntry == nil || blockmapEntry == nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the ZIP must contain only latest.yml, its installer, and its blockmap")
	}
	if installerEntry.UncompressedSize64 != uint64(manifest.Files[0].Size) || installerEntry.UncompressedSize64 > uint64(maxReleaseInstallerSize) {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the installer size does not match latest.yml")
	}
	if blockmapEntry.UncompressedSize64 == 0 || blockmapEntry.UncompressedSize64 > uint64(maxReleaseBlockmapSize) {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the blockmap is empty or too large")
	}

	stagingDirectory, err := os.MkdirTemp(platformDirectory, ".release-stage-")
	if err != nil {
		return nil, 0, fmt.Errorf("create release staging directory: %w", err)
	}
	defer os.RemoveAll(stagingDirectory)
	installerPath := filepath.Join(stagingDirectory, installerName)
	installerDigest, installerSize, err := extractZipEntry(installerEntry, installerPath, maxReleaseInstallerSize)
	if err != nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the installer could not be extracted")
	}
	if installerSize != manifest.Files[0].Size || installerDigest != manifest.SHA512 {
		return nil, 0, releaseError(http.StatusBadRequest, "release_checksum_mismatch", "the installer checksum or size does not match latest.yml")
	}
	blockmapPath := filepath.Join(stagingDirectory, blockmapName)
	if _, _, err := extractZipEntry(blockmapEntry, blockmapPath, maxReleaseBlockmapSize); err != nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the blockmap could not be extracted")
	}
	manifestPath := filepath.Join(stagingDirectory, "latest.yml")
	if err := os.WriteFile(manifestPath, manifestBytes, 0o640); err != nil {
		return nil, 0, fmt.Errorf("stage update manifest: %w", err)
	}

	if err := publishVersionedArtifact(installerPath, filepath.Join(platformDirectory, installerName)); err != nil {
		return nil, 0, err
	}
	if err := publishVersionedArtifact(blockmapPath, filepath.Join(platformDirectory, blockmapName)); err != nil {
		return nil, 0, err
	}
	finalManifestPath := filepath.Join(platformDirectory, "latest.yml")
	previousManifest, hadPreviousManifest, err := preserveCurrentManifest(finalManifestPath, stagingDirectory)
	if err != nil {
		return nil, 0, fmt.Errorf("preserve current update manifest: %w", err)
	}
	if err := os.Rename(manifestPath, finalManifestPath); err != nil {
		return nil, 0, fmt.Errorf("publish update manifest: %w", err)
	}

	updatedAt := time.Now().UnixMilli()
	_, databaseErr := s.store.DB.ExecContext(r.Context(), `INSERT INTO settings(key,value,updated_at) VALUES('latest_app_version',?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at`, manifest.Version, updatedAt)
	if databaseErr != nil {
		if hadPreviousManifest {
			_ = os.Rename(previousManifest, finalManifestPath)
		} else {
			_ = os.Remove(finalManifestPath)
		}
		return nil, 0, fmt.Errorf("update published version: %w", databaseErr)
	}
	publishedAt := updatedAt
	if manifest.ReleaseDate != "" {
		if parsed, parseErr := time.Parse(time.RFC3339, manifest.ReleaseDate); parseErr == nil {
			publishedAt = parsed.UnixMilli()
		}
	}
	return &publishedRelease{Version: manifest.Version, InstallerName: installerName, InstallerSize: installerSize, PublishedAt: publishedAt}, updatedAt, nil
}

func (s *Server) publishVCLogg2ReleasePackage(w http.ResponseWriter, r *http.Request) (*publishedRelease, int64, error) {
	platformDirectory := filepath.Join(s.config.Updates.VCLogg2Directory, "win-x64")
	if err := os.MkdirAll(platformDirectory, 0o750); err != nil {
		return nil, 0, fmt.Errorf("create VCLogg2 update directory: %w", err)
	}
	r.Body = http.MaxBytesReader(w, r.Body, maxReleaseUploadBytes+1024*1024)
	reader, err := r.MultipartReader()
	if err != nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_upload", "a multipart release package is required")
	}
	temporaryPackage, err := os.CreateTemp(platformDirectory, ".release-upload-*.zip")
	if err != nil {
		return nil, 0, fmt.Errorf("create temporary VCLogg2 release package: %w", err)
	}
	temporaryPackagePath := temporaryPackage.Name()
	defer os.Remove(temporaryPackagePath)
	defer temporaryPackage.Close()

	foundPackage := false
	for {
		part, nextErr := reader.NextPart()
		if errors.Is(nextErr, io.EOF) {
			break
		}
		if nextErr != nil {
			return nil, 0, releaseError(http.StatusRequestEntityTooLarge, "release_package_too_large", "the release package exceeds the 512 MiB limit")
		}
		if part.FormName() != "package" || part.FileName() == "" || foundPackage {
			part.Close()
			return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_upload", "the request must contain exactly one package file")
		}
		foundPackage = true
		written, copyErr := io.Copy(temporaryPackage, part)
		part.Close()
		if copyErr != nil {
			return nil, 0, releaseError(http.StatusRequestEntityTooLarge, "release_package_too_large", "the release package exceeds the 512 MiB limit")
		}
		if written <= 0 || written > maxReleaseUploadBytes {
			return nil, 0, releaseError(http.StatusRequestEntityTooLarge, "release_package_too_large", "the release package must be between 1 byte and 512 MiB")
		}
	}
	if !foundPackage {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_upload", "the release package file is missing")
	}
	if err := temporaryPackage.Sync(); err != nil {
		return nil, 0, fmt.Errorf("flush temporary VCLogg2 release package: %w", err)
	}
	info, err := temporaryPackage.Stat()
	if err != nil {
		return nil, 0, fmt.Errorf("inspect temporary VCLogg2 release package: %w", err)
	}
	archive, err := zip.NewReader(temporaryPackage, info.Size())
	if err != nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the uploaded file is not a valid ZIP package")
	}

	entries := make(map[string]*zip.File, len(archive.File))
	for _, entry := range archive.File {
		if entry.FileInfo().IsDir() {
			continue
		}
		if entry.Name != filepath.Base(entry.Name) || strings.ContainsAny(entry.Name, `/\\`) || !entry.FileInfo().Mode().IsRegular() {
			return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "release files must be regular files at the ZIP root")
		}
		if _, duplicate := entries[entry.Name]; duplicate {
			return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the release package contains duplicate files")
		}
		entries[entry.Name] = entry
	}
	manifestEntry := entries["latest.json"]
	if manifestEntry == nil || manifestEntry.UncompressedSize64 > uint64(maxReleaseManifestBytes) {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_manifest", "latest.json is missing or too large")
	}
	manifestBytes, err := readZipEntry(manifestEntry, maxReleaseManifestBytes)
	if err != nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_manifest", "latest.json could not be read")
	}
	var manifest vclogg2ReleaseManifest
	if err := json.Unmarshal(manifestBytes, &manifest); err != nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_manifest", "latest.json is not valid JSON")
	}
	manifest.Version = strings.TrimSpace(manifest.Version)
	artifactName := "vclogg2-" + manifest.Version + "-win-x64.zip"
	blockmapName := "vclogg2-" + manifest.Version + "-win-x64.blockmap.json"
	if manifest.SchemaVersion != 1 || manifest.Product != "VCLogg2" || manifest.Platform != "windows" || manifest.Architecture != "x86_64" || !semverPattern.MatchString(manifest.Version) {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_manifest", "latest.json is not a compatible VCLogg2 Windows x64 manifest")
	}
	if manifest.Artifact != artifactName || manifest.Blockmap != blockmapName || manifest.Size <= 0 || manifest.Size > maxReleaseInstallerSize || !validSHA256(manifest.SHA256) {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_manifest", "latest.json does not describe the matching VCLogg2 release files")
	}
	artifactEntry, blockmapEntry := entries[artifactName], entries[blockmapName]
	if len(entries) != 3 || artifactEntry == nil || blockmapEntry == nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the ZIP must contain only latest.json, its VCLogg2 archive, and its blockmap")
	}
	if artifactEntry.UncompressedSize64 != uint64(manifest.Size) || artifactEntry.UncompressedSize64 > uint64(maxReleaseInstallerSize) {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the VCLogg2 archive size does not match latest.json")
	}
	if blockmapEntry.UncompressedSize64 == 0 || blockmapEntry.UncompressedSize64 > uint64(maxReleaseBlockmapSize) {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the blockmap is empty or too large")
	}
	blockmapBytes, err := readZipEntry(blockmapEntry, maxReleaseBlockmapSize)
	if err != nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the blockmap could not be read")
	}
	var blockmap vclogg2ReleaseBlockmap
	if err := json.Unmarshal(blockmapBytes, &blockmap); err != nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the blockmap is not valid JSON")
	}
	if blockmap.SchemaVersion != 1 || blockmap.Algorithm != "sha256" || blockmap.File != artifactName || blockmap.ChunkSize <= 0 || blockmap.ChunkSize > 16*1024*1024 {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the blockmap metadata is invalid")
	}
	expectedChunks := (manifest.Size + blockmap.ChunkSize - 1) / blockmap.ChunkSize
	if int64(len(blockmap.Chunks)) != expectedChunks {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the blockmap chunk count does not match the VCLogg2 archive")
	}
	for _, digest := range blockmap.Chunks {
		if !validSHA256(digest) {
			return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the blockmap contains an invalid SHA-256 digest")
		}
	}

	stagingDirectory, err := os.MkdirTemp(platformDirectory, ".release-stage-")
	if err != nil {
		return nil, 0, fmt.Errorf("create VCLogg2 release staging directory: %w", err)
	}
	defer os.RemoveAll(stagingDirectory)
	artifactPath := filepath.Join(stagingDirectory, artifactName)
	artifactDigest, artifactSize, err := extractZipEntrySHA256(artifactEntry, artifactPath, maxReleaseInstallerSize)
	if err != nil {
		return nil, 0, releaseError(http.StatusBadRequest, "invalid_release_package", "the VCLogg2 archive could not be extracted")
	}
	if artifactSize != manifest.Size || !strings.EqualFold(artifactDigest, manifest.SHA256) {
		return nil, 0, releaseError(http.StatusBadRequest, "release_checksum_mismatch", "the VCLogg2 archive checksum or size does not match latest.json")
	}
	if err := validateVCLogg2Chunks(artifactPath, &blockmap); err != nil {
		return nil, 0, releaseError(http.StatusBadRequest, "release_checksum_mismatch", err.Error())
	}
	blockmapPath := filepath.Join(stagingDirectory, blockmapName)
	if err := os.WriteFile(blockmapPath, blockmapBytes, 0o640); err != nil {
		return nil, 0, fmt.Errorf("stage VCLogg2 blockmap: %w", err)
	}
	manifestPath := filepath.Join(stagingDirectory, "latest.json")
	if err := os.WriteFile(manifestPath, manifestBytes, 0o640); err != nil {
		return nil, 0, fmt.Errorf("stage VCLogg2 update manifest: %w", err)
	}

	if err := publishVersionedArtifact(artifactPath, filepath.Join(platformDirectory, artifactName)); err != nil {
		return nil, 0, err
	}
	if err := publishVersionedArtifact(blockmapPath, filepath.Join(platformDirectory, blockmapName)); err != nil {
		return nil, 0, err
	}
	finalManifestPath := filepath.Join(platformDirectory, "latest.json")
	previousManifest, hadPreviousManifest, err := preserveCurrentManifest(finalManifestPath, stagingDirectory)
	if err != nil {
		return nil, 0, fmt.Errorf("preserve current VCLogg2 update manifest: %w", err)
	}
	if err := os.Rename(manifestPath, finalManifestPath); err != nil {
		return nil, 0, fmt.Errorf("publish VCLogg2 update manifest: %w", err)
	}

	updatedAt := time.Now().UnixMilli()
	_, databaseErr := s.store.DB.ExecContext(r.Context(), `INSERT INTO settings(key,value,updated_at) VALUES('latest_app_version_vclogg2',?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at`, manifest.Version, updatedAt)
	if databaseErr != nil {
		if hadPreviousManifest {
			_ = os.Rename(previousManifest, finalManifestPath)
		} else {
			_ = os.Remove(finalManifestPath)
		}
		return nil, 0, fmt.Errorf("update published VCLogg2 version: %w", databaseErr)
	}
	return &publishedRelease{Version: manifest.Version, InstallerName: artifactName, InstallerSize: artifactSize, PublishedAt: updatedAt}, updatedAt, nil
}

func (s *Server) currentPublishedRelease() (*publishedRelease, error) {
	manifestPath := filepath.Join(s.config.Updates.Directory, "win-x64", "latest.yml")
	data, err := os.ReadFile(manifestPath)
	if err != nil {
		return nil, err
	}
	var manifest releaseManifest
	if err := yaml.Unmarshal(data, &manifest); err != nil {
		return nil, err
	}
	expectedInstallerName := "VCLogg-" + manifest.Version + "-Setup-x64.exe"
	if !semverPattern.MatchString(manifest.Version) || manifest.Path != expectedInstallerName || !windowsUpdateArtifactPattern.MatchString(manifest.Path) {
		return nil, errors.New("invalid update manifest")
	}
	installerInfo, err := os.Stat(filepath.Join(s.config.Updates.Directory, "win-x64", manifest.Path))
	if err != nil {
		return nil, err
	}
	if !installerInfo.Mode().IsRegular() {
		return nil, errors.New("update installer is not a regular file")
	}
	blockmapInfo, err := os.Stat(filepath.Join(s.config.Updates.Directory, "win-x64", manifest.Path+".blockmap"))
	if err != nil {
		return nil, err
	}
	if !blockmapInfo.Mode().IsRegular() {
		return nil, errors.New("update blockmap is not a regular file")
	}
	publishedAt := installerInfo.ModTime().UnixMilli()
	if manifest.ReleaseDate != "" {
		if parsed, parseErr := time.Parse(time.RFC3339, manifest.ReleaseDate); parseErr == nil {
			publishedAt = parsed.UnixMilli()
		}
	}
	return &publishedRelease{Version: manifest.Version, InstallerName: manifest.Path, InstallerSize: installerInfo.Size(), PublishedAt: publishedAt}, nil
}

func (s *Server) currentPublishedVCLogg2Release() (*publishedRelease, error) {
	platformDirectory := filepath.Join(s.config.Updates.VCLogg2Directory, "win-x64")
	data, err := os.ReadFile(filepath.Join(platformDirectory, "latest.json"))
	if err != nil {
		return nil, err
	}
	var manifest vclogg2ReleaseManifest
	if err := json.Unmarshal(data, &manifest); err != nil {
		return nil, err
	}
	expectedArtifact := "vclogg2-" + manifest.Version + "-win-x64.zip"
	expectedBlockmap := "vclogg2-" + manifest.Version + "-win-x64.blockmap.json"
	if manifest.SchemaVersion != 1 || manifest.Product != "VCLogg2" || manifest.Platform != "windows" || manifest.Architecture != "x86_64" || !semverPattern.MatchString(manifest.Version) || manifest.Artifact != expectedArtifact || manifest.Blockmap != expectedBlockmap || !validSHA256(manifest.SHA256) {
		return nil, errors.New("invalid VCLogg2 update manifest")
	}
	artifactInfo, err := os.Stat(filepath.Join(platformDirectory, manifest.Artifact))
	if err != nil {
		return nil, err
	}
	if !artifactInfo.Mode().IsRegular() || artifactInfo.Size() != manifest.Size {
		return nil, errors.New("VCLogg2 update archive is invalid")
	}
	blockmapInfo, err := os.Stat(filepath.Join(platformDirectory, manifest.Blockmap))
	if err != nil {
		return nil, err
	}
	if !blockmapInfo.Mode().IsRegular() {
		return nil, errors.New("VCLogg2 update blockmap is not a regular file")
	}
	return &publishedRelease{Version: manifest.Version, InstallerName: manifest.Artifact, InstallerSize: artifactInfo.Size(), PublishedAt: artifactInfo.ModTime().UnixMilli()}, nil
}

func readZipEntry(entry *zip.File, limit int64) ([]byte, error) {
	reader, err := entry.Open()
	if err != nil {
		return nil, err
	}
	defer reader.Close()
	return io.ReadAll(io.LimitReader(reader, limit+1))
}

func extractZipEntry(entry *zip.File, target string, limit int64) (string, int64, error) {
	reader, err := entry.Open()
	if err != nil {
		return "", 0, err
	}
	defer reader.Close()
	file, err := os.OpenFile(target, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o640)
	if err != nil {
		return "", 0, err
	}
	hash := sha512.New()
	written, copyErr := io.Copy(io.MultiWriter(file, hash), io.LimitReader(reader, limit+1))
	closeErr := file.Close()
	if copyErr != nil || closeErr != nil || written > limit {
		return "", written, errors.Join(copyErr, closeErr, errors.New("extracted file exceeds limit"))
	}
	return base64.StdEncoding.EncodeToString(hash.Sum(nil)), written, nil
}

func extractZipEntrySHA256(entry *zip.File, target string, limit int64) (string, int64, error) {
	reader, err := entry.Open()
	if err != nil {
		return "", 0, err
	}
	defer reader.Close()
	file, err := os.OpenFile(target, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o640)
	if err != nil {
		return "", 0, err
	}
	hash := sha256.New()
	written, copyErr := io.Copy(io.MultiWriter(file, hash), io.LimitReader(reader, limit+1))
	closeErr := file.Close()
	if copyErr != nil || closeErr != nil || written > limit {
		return "", written, errors.Join(copyErr, closeErr, errors.New("extracted file exceeds limit"))
	}
	return hex.EncodeToString(hash.Sum(nil)), written, nil
}

func validateVCLogg2Chunks(artifactPath string, blockmap *vclogg2ReleaseBlockmap) error {
	file, err := os.Open(artifactPath)
	if err != nil {
		return err
	}
	defer file.Close()
	buffer := make([]byte, int(blockmap.ChunkSize))
	for index, expected := range blockmap.Chunks {
		read, readErr := io.ReadFull(file, buffer)
		if readErr != nil && !errors.Is(readErr, io.ErrUnexpectedEOF) {
			return errors.New("the VCLogg2 archive does not match its blockmap")
		}
		digest := sha256.Sum256(buffer[:read])
		if !strings.EqualFold(hex.EncodeToString(digest[:]), expected) {
			return fmt.Errorf("VCLogg2 archive chunk %d does not match its blockmap", index)
		}
	}
	return nil
}

func validSHA256(value string) bool {
	if len(value) != sha256.Size*2 {
		return false
	}
	_, err := hex.DecodeString(value)
	return err == nil
}

func decodeSHA512(value string) ([]byte, error) {
	decoded, err := base64.StdEncoding.DecodeString(value)
	if err != nil || len(decoded) != sha512.Size {
		return nil, errors.New("invalid SHA-512 digest")
	}
	return decoded, nil
}

func publishVersionedArtifact(stagedPath, finalPath string) error {
	if _, err := os.Stat(finalPath); err == nil {
		same, compareErr := filesEqual(stagedPath, finalPath)
		if compareErr != nil {
			return fmt.Errorf("compare existing release artifact: %w", compareErr)
		}
		if !same {
			return releaseError(http.StatusConflict, "release_artifact_conflict", "this version already exists with different release files")
		}
		return nil
	} else if !errors.Is(err, os.ErrNotExist) {
		return fmt.Errorf("inspect existing release artifact: %w", err)
	}
	if err := os.Rename(stagedPath, finalPath); err != nil {
		return fmt.Errorf("publish release artifact: %w", err)
	}
	return nil
}

func filesEqual(leftPath, rightPath string) (bool, error) {
	left, err := os.Open(leftPath)
	if err != nil {
		return false, err
	}
	defer left.Close()
	right, err := os.Open(rightPath)
	if err != nil {
		return false, err
	}
	defer right.Close()
	leftHash, rightHash := sha512.New(), sha512.New()
	leftSize, err := io.Copy(leftHash, left)
	if err != nil {
		return false, err
	}
	rightSize, err := io.Copy(rightHash, right)
	if err != nil {
		return false, err
	}
	return leftSize == rightSize && string(leftHash.Sum(nil)) == string(rightHash.Sum(nil)), nil
}

func preserveCurrentManifest(finalPath, stagingDirectory string) (string, bool, error) {
	current, err := os.Open(finalPath)
	if errors.Is(err, os.ErrNotExist) {
		return "", false, nil
	}
	if err != nil {
		return "", false, err
	}
	defer current.Close()
	backupPath := filepath.Join(stagingDirectory, "previous-"+filepath.Base(finalPath))
	backup, err := os.OpenFile(backupPath, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o640)
	if err != nil {
		return "", false, err
	}
	_, copyErr := io.Copy(backup, current)
	closeErr := backup.Close()
	if copyErr != nil || closeErr != nil {
		return "", false, errors.Join(copyErr, closeErr)
	}
	return backupPath, true, nil
}
