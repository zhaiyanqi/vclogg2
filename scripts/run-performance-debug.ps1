param(
    [ValidateRange(1, 60000)]
    [int]$WarnAfterMilliseconds = 16,

    [ValidateRange(1, 60000)]
    [int]$RepeatAfterMilliseconds = 2000,

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
$env:VCLOGG2_UI_PERF_WARN_MS = $WarnAfterMilliseconds.ToString()
$env:VCLOGG2_UI_PERF_REPEAT_MS = $RepeatAfterMilliseconds.ToString()

$cargoArguments = @(
    'run',
    '-p',
    'vclogg2',
    '--features',
    'ui-performance-profiler',
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
