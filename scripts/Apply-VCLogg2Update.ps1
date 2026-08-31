param(
    [Parameter(Mandatory)]
    [string]$ArchivePath,
    [Parameter(Mandatory)]
    [string]$InstallDirectory,
    [Parameter(Mandatory)]
    [int]$WaitForProcessId,
    [switch]$Launch
)

$ErrorActionPreference = 'Stop'

$resolvedArchive = [System.IO.Path]::GetFullPath($ArchivePath)
$resolvedInstall = [System.IO.Path]::GetFullPath($InstallDirectory)
if (-not (Test-Path -LiteralPath $resolvedArchive -PathType Leaf)) {
    throw "更新包不存在：$resolvedArchive"
}
if ([string]::IsNullOrWhiteSpace($resolvedInstall)) {
    throw '安装目录不能为空。'
}

Wait-Process -Id $WaitForProcessId -ErrorAction SilentlyContinue
$stageRoot = Join-Path ([System.IO.Path]::GetTempPath()) "vclogg2-update-$([guid]::NewGuid().ToString('N'))"
try {
    New-Item -ItemType Directory -Path $stageRoot | Out-Null
    Expand-Archive -LiteralPath $resolvedArchive -DestinationPath $stageRoot
    $installer = Join-Path $stageRoot 'Install-VCLogg2.ps1'
    if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) {
        throw '更新包不完整：缺少 Install-VCLogg2.ps1。'
    }
    & $installer -InstallDirectory $resolvedInstall -Launch:$Launch
} finally {
    if (Test-Path -LiteralPath $stageRoot) {
        Remove-Item -LiteralPath $stageRoot -Recurse -Force
    }
}
