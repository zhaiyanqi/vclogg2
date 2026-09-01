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
    $sourceExecutable = Join-Path $stageRoot 'vclogg2.exe'
    if (-not (Test-Path -LiteralPath $sourceExecutable -PathType Leaf)) {
        throw '更新包不完整：缺少 vclogg2.exe。'
    }

    New-Item -ItemType Directory -Path $resolvedInstall -Force | Out-Null
    $installedExecutable = Join-Path $resolvedInstall 'vclogg2.exe'
    Copy-Item -LiteralPath $sourceExecutable -Destination $installedExecutable -Force

    foreach ($documentName in @('README.md', 'LICENSE')) {
        $sourceDocument = Join-Path $stageRoot $documentName
        if (Test-Path -LiteralPath $sourceDocument -PathType Leaf) {
            Copy-Item -LiteralPath $sourceDocument -Destination (Join-Path $resolvedInstall $documentName) -Force
        }
    }

    if ($Launch) {
        Start-Process -FilePath $installedExecutable
    }
} finally {
    if (Test-Path -LiteralPath $stageRoot) {
        Remove-Item -LiteralPath $stageRoot -Recurse -Force
    }
}
