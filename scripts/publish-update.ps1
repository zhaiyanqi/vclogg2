param(
    [Parameter(Mandatory)]
    [string]$TargetDirectory,
    [string]$SourceDirectory = (Join-Path $PSScriptRoot '..\dist\windows-x86_64')
)

$ErrorActionPreference = 'Stop'

$source = [System.IO.Path]::GetFullPath($SourceDirectory)
$target = [System.IO.Path]::GetFullPath($TargetDirectory)
$manifestPath = Join-Path $source 'latest.json'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "缺少更新发布文件：$manifestPath"
}

$manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding utf8 | ConvertFrom-Json
if ($manifest.schemaVersion -ne 1 -or $manifest.product -ne 'VCLogg2' -or
    $manifest.platform -ne 'windows' -or $manifest.architecture -ne 'x86_64') {
    throw 'latest.json 与 Windows x86_64 VCLogg2 更新契约不兼容。'
}

$artifactName = [string]$manifest.artifact
$blockmapName = [string]$manifest.blockmap
foreach ($name in @($artifactName, $blockmapName)) {
    if ([string]::IsNullOrWhiteSpace($name) -or
        [System.IO.Path]::GetFileName($name) -ne $name) {
        throw "更新清单包含无效文件名：$name"
    }
}
$artifactPath = Join-Path $source $artifactName
$blockmapPath = Join-Path $source $blockmapName
foreach ($required in @($artifactPath, $blockmapPath)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "缺少更新发布文件：$required"
    }
}
$actualSize = (Get-Item -LiteralPath $artifactPath).Length
$actualHash = (Get-FileHash -LiteralPath $artifactPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSize -ne [int64]$manifest.size -or
    $actualHash -ne ([string]$manifest.sha256).ToLowerInvariant()) {
    throw '更新包大小或 SHA-256 与 latest.json 不一致。'
}

New-Item -ItemType Directory -Path $target -Force | Out-Null
Copy-Item -LiteralPath $artifactPath -Destination (Join-Path $target $artifactName) -Force
Copy-Item -LiteralPath $blockmapPath -Destination (Join-Path $target $blockmapName) -Force

$pendingManifest = Join-Path $target "latest.json.pending-$PID"
Copy-Item -LiteralPath $manifestPath -Destination $pendingManifest -Force
Move-Item -LiteralPath $pendingManifest -Destination (Join-Path $target 'latest.json') -Force

Write-Output "已发布 VCLogg2 $($manifest.version) 到：$target"
