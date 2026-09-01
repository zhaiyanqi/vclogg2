param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\dist')
)

$ErrorActionPreference = 'Stop'

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$resolvedOutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)

Push-Location -LiteralPath $repositoryRoot
$metadata = cargo metadata --no-deps --format-version 1 --locked | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$package = $metadata.packages | Where-Object { $_.name -eq 'vclogg2' } | Select-Object -First 1
if ($null -eq $package) { throw '无法从 Cargo 元数据读取 vclogg2 版本。' }
$version = $package.version

& (Join-Path $PSScriptRoot 'build-release.ps1')
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

New-Item -ItemType Directory -Path $resolvedOutputDirectory -Force | Out-Null
$platformDirectory = [System.IO.Path]::GetFullPath(
    (Join-Path $resolvedOutputDirectory 'windows-x86_64')
)
New-Item -ItemType Directory -Path $platformDirectory -Force | Out-Null
$stageDirectory = [System.IO.Path]::GetFullPath(
    (Join-Path $platformDirectory "vclogg2-$version-windows-x86_64")
)
$outputPrefix = $platformDirectory.TrimEnd('\') + '\'
if (-not $stageDirectory.StartsWith($outputPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "暂存目录不在指定输出目录内：$stageDirectory"
}
if (Test-Path -LiteralPath $stageDirectory) {
    Remove-Item -LiteralPath $stageDirectory -Recurse -Force
}
New-Item -ItemType Directory -Path $stageDirectory -Force | Out-Null

$releaseExecutable = Join-Path $repositoryRoot 'target\release\vclogg2.exe'
Copy-Item -LiteralPath $releaseExecutable -Destination (Join-Path $stageDirectory 'vclogg2.exe')
Copy-Item -LiteralPath (Join-Path $repositoryRoot 'README.md') -Destination $stageDirectory
Copy-Item -LiteralPath (Join-Path $repositoryRoot 'LICENSE') -Destination $stageDirectory

$expectedPackageEntries = @('LICENSE', 'README.md', 'vclogg2.exe') | Sort-Object
$actualPackageEntries = @(Get-ChildItem -LiteralPath $stageDirectory -Force | ForEach-Object Name) | Sort-Object
$packageDifference = @(
    Compare-Object -ReferenceObject $expectedPackageEntries -DifferenceObject $actualPackageEntries
)
if ($packageDifference.Count -ne 0) {
    throw 'Windows 用户分发包必须且只能包含 LICENSE、README.md 与 vclogg2.exe。'
}

$archiveName = "vclogg2-$version-windows-x86_64.zip"
$archivePath = Join-Path $platformDirectory $archiveName
if (Test-Path -LiteralPath $archivePath) {
    Remove-Item -LiteralPath $archivePath -Force
}
Compress-Archive -Path (Join-Path $stageDirectory '*') -DestinationPath $archivePath -CompressionLevel Optimal

$releaseSymbols = Join-Path $repositoryRoot 'target\release\vclogg2.pdb'
$symbolsArchiveName = "vclogg2-$version-windows-x86_64-symbols.zip"
$symbolsArchivePath = Join-Path $platformDirectory $symbolsArchiveName
if (Test-Path -LiteralPath $symbolsArchivePath) {
    Remove-Item -LiteralPath $symbolsArchivePath -Force
}
Compress-Archive -LiteralPath $releaseSymbols -DestinationPath $symbolsArchivePath -CompressionLevel Optimal

$chunkSize = 1024 * 1024
$stream = [System.IO.File]::OpenRead($archivePath)
$chunkHashes = [System.Collections.Generic.List[string]]::new()
try {
    $buffer = New-Object byte[] $chunkSize
    while (($read = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
        $sha = [System.Security.Cryptography.SHA256]::Create()
        try {
            $hex = [BitConverter]::ToString($sha.ComputeHash($buffer, 0, $read))
            $chunkHashes.Add($hex.Replace('-', '').ToLowerInvariant())
        } finally {
            $sha.Dispose()
        }
    }
} finally {
    $stream.Dispose()
}

$blockmapName = "vclogg2-$version-windows-x86_64.blockmap.json"
$blockmapPath = Join-Path $platformDirectory $blockmapName
$blockmapJson = [ordered]@{
    schemaVersion = 1
    algorithm = 'sha256'
    chunkSize = $chunkSize
    file = $archiveName
    chunks = $chunkHashes
} | ConvertTo-Json -Depth 4
[System.IO.File]::WriteAllText(
    $blockmapPath,
    $blockmapJson + [Environment]::NewLine,
    [System.Text.UTF8Encoding]::new($false)
)

$archiveInfo = Get-Item -LiteralPath $archivePath
$archiveHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
$manifestPath = Join-Path $platformDirectory 'latest.json'
$manifestJson = [ordered]@{
    schemaVersion = 1
    product = 'VCLogg2'
    version = $version
    platform = 'windows'
    architecture = 'x86_64'
    artifact = $archiveName
    sha256 = $archiveHash
    size = $archiveInfo.Length
    blockmap = $blockmapName
} | ConvertTo-Json
[System.IO.File]::WriteAllText(
    $manifestPath,
    $manifestJson + [Environment]::NewLine,
    [System.Text.UTF8Encoding]::new($false)
)

Write-Output "便携目录：$stageDirectory"
Write-Output "发布压缩包：$archivePath"
Write-Output "调试符号包（不向用户分发）：$symbolsArchivePath"
Write-Output "更新清单：$manifestPath"
Write-Output "分块清单：$blockmapPath"
Pop-Location
