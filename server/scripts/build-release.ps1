[CmdletBinding()]
param(
    [string]$OutputDirectory = '',
    [string]$ClientKeyFile = '',
    [string]$Version = ''
)

# Keep the former standalone entry point available after moving into server/.
$ErrorActionPreference = 'Stop'
$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
& (Join-Path $repositoryRoot 'scripts\package-server-release.ps1') @PSBoundParameters
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
