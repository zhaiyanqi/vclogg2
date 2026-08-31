$ErrorActionPreference = 'Stop'

cargo build --workspace --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
