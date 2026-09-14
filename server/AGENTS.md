# VCLogg Server workspace instructions

## Project boundaries

- `cmd/` and `internal/` contain the standalone Go server.
- `admin-web/` contains the Vue administrator console.
- `internal/app/admin/` contains generated administrator console assets embedded by Go. After changing `admin-web/`, run `pnpm run build:admin` and commit the generated assets.
- The server project must not import files from the VCLogg desktop repository. Release-key integration uses a public JSON descriptor only.

## Verification

- On Windows, run `powershell -ExecutionPolicy Bypass -File scripts/verify.ps1` for the complete verification suite.
- The verification script must remain usable when `pnpm` is available through a bundled launcher but `node.exe` is not already on `PATH`.
