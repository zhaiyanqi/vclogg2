param(
    [string]$DataDirectory,

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Paths = @()
)

$ErrorActionPreference = 'Stop'

$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$targetDirectory = Join-Path $projectRoot 'target\perf-debug'
if ([string]::IsNullOrWhiteSpace($DataDirectory)) {
    $DataDirectory = Join-Path $targetDirectory 'local-app-data'
}
$DataDirectory = [System.IO.Path]::GetFullPath($DataDirectory)
[System.IO.Directory]::CreateDirectory($DataDirectory) | Out-Null

$env:RUST_BACKTRACE = 'full'
$env:VCLOGG2_DEV_DATA_DIR = $DataDirectory

$cargoArguments = @(
    'run',
    '-p',
    'vclogg2',
    '--locked',
    '--target-dir',
    $targetDirectory
)
if ($Paths.Count -gt 0) {
    $cargoArguments += '--'
    $cargoArguments += $Paths
}

& cargo @cargoArguments
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
