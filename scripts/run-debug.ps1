$ErrorActionPreference = 'Stop'
$env:RUST_BACKTRACE = 'full'

cargo run -p vclogg2 --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
