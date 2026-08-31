$ErrorActionPreference = 'Stop'

cargo build --workspace --release --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
