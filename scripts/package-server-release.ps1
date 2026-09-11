[CmdletBinding()]
param(
    [string]$Version = '',
    [string]$OutputDirectory = '',
    [string]$ClientKeyFile = '',
    [switch]$Help
)

$ErrorActionPreference = 'Stop'
$nodeCommand = Get-Command node.exe -ErrorAction SilentlyContinue
$nodeExecutable = if ($nodeCommand) { $nodeCommand.Source } else { $null }
if (-not $nodeExecutable -and $env:npm_node_execpath -and
    (Test-Path -LiteralPath $env:npm_node_execpath)) {
    $nodeExecutable = $env:npm_node_execpath
}
if (-not $nodeExecutable) {
    $pnpmCommand = Get-Command pnpm.cmd -ErrorAction SilentlyContinue
    if (-not $pnpmCommand) { $pnpmCommand = Get-Command pnpm -ErrorAction SilentlyContinue }
    if ($pnpmCommand) {
        $pnpmDirectory = Split-Path -Parent $pnpmCommand.Source
        foreach ($candidate in @(
            (Join-Path $pnpmDirectory 'node.exe'),
            (Join-Path $pnpmDirectory '..\..\node\bin\node.exe')
        )) {
            if (Test-Path -LiteralPath $candidate) {
                $nodeExecutable = (Resolve-Path -LiteralPath $candidate).Path
                break
            }
        }
    }
}
if (-not $nodeExecutable) { throw 'Node.js 22.12 or newer is required, on PATH or beside the pnpm launcher.' }

$arguments = @((Join-Path $PSScriptRoot 'package-server-release.mjs'))
if ($Version) { $arguments += @('--version', $Version) }
if ($OutputDirectory) { $arguments += @('--output-dir', $OutputDirectory) }
if ($ClientKeyFile) { $arguments += @('--client-key-file', $ClientKeyFile) }
if ($Help) { $arguments += '--help' }
& $nodeExecutable @arguments
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
