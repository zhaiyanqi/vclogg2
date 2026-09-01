param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\dist'),
    [string]$SigningCertificatePath = $env:VCLOGG2_WINDOWS_SIGNING_CERTIFICATE_PATH,
    [string]$TimestampUrl = $env:VCLOGG2_WINDOWS_TIMESTAMP_URL,
    [ValidateSet('Pfx', 'PreSigned')]
    [string]$SigningMode = 'Pfx',
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$resolvedOutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)
if ($SigningMode -eq 'Pfx') {
    if ([string]::IsNullOrWhiteSpace($SigningCertificatePath)) {
        throw 'PFX 签名模式必须提供代码签名证书；请设置 VCLOGG2_WINDOWS_SIGNING_CERTIFICATE_PATH。'
    }
    if ([string]::IsNullOrWhiteSpace($TimestampUrl)) {
        throw 'PFX 签名模式必须提供 RFC 3161 时间戳地址；请设置 VCLOGG2_WINDOWS_TIMESTAMP_URL。'
    }
    if ([string]::IsNullOrEmpty($env:VCLOGG2_WINDOWS_SIGNING_CERTIFICATE_PASSWORD)) {
        throw 'PFX 签名模式必须提供受密码保护的证书；请设置 VCLOGG2_WINDOWS_SIGNING_CERTIFICATE_PASSWORD。'
    }
    $resolvedSigningCertificate = [System.IO.Path]::GetFullPath($SigningCertificatePath)
    if (-not (Test-Path -LiteralPath $resolvedSigningCertificate -PathType Leaf)) {
        throw "Windows 代码签名证书不存在：$resolvedSigningCertificate"
    }
} elseif ($SigningMode -eq 'PreSigned' -and -not $SkipBuild) {
    throw 'PreSigned 模式必须与 -SkipBuild 同时使用，避免重新编译覆盖外部签名结果。'
}

Push-Location -LiteralPath $repositoryRoot
$version = $env:VCLOGG2_BUILD_VERSION
if ([string]::IsNullOrWhiteSpace($version)) {
    $tag = & git describe --tags --exact-match --match 'v[0-9]*' HEAD 2>$null
    if ($LASTEXITCODE -eq 0) {
        $version = $tag
    } else {
        $commit = & git rev-parse --short=12 HEAD 2>$null
        if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($commit)) {
            $version = "0.0.0-dev+g$commit"
        } else {
            $version = '0.0.0'
        }
    }
}
$version = $version.Trim()
if ($version.StartsWith('v')) { $version = $version.Substring(1) }
$number = '(0|[1-9][0-9]*)'
$prereleaseIdentifier = '(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)'
$buildIdentifier = '[0-9A-Za-z-]+'
$semverPattern = "^$number\.$number\.$number(-$prereleaseIdentifier(\.$prereleaseIdentifier)*)?(\+$buildIdentifier(\.$buildIdentifier)*)?$"
if ($version -notmatch $semverPattern) {
    throw "构建版本必须是语义版本或带 v 前缀的语义版本：$version"
}
$env:VCLOGG2_BUILD_VERSION = $version

if (-not $SkipBuild) {
    & (Join-Path $PSScriptRoot 'build-release.ps1')
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

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
if (-not (Test-Path -LiteralPath $releaseExecutable -PathType Leaf)) {
    throw "Windows Release 可执行文件不存在：$releaseExecutable"
}
if ($SigningMode -eq 'Pfx') {
    & (Join-Path $PSScriptRoot 'sign-windows.ps1') `
        -ExecutablePath $releaseExecutable `
        -CertificatePath $resolvedSigningCertificate `
        -TimestampUrl $TimestampUrl
}

$releaseSignature = Get-AuthenticodeSignature -LiteralPath $releaseExecutable
if ($releaseSignature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
    $null -eq $releaseSignature.SignerCertificate -or
    $null -eq $releaseSignature.TimeStamperCertificate) {
    throw 'Windows Release 可执行文件必须包含有效且受信任的 Authenticode 签名和 RFC 3161 时间戳。'
}

$packagedExecutable = Join-Path $stageDirectory 'vclogg2.exe'
Copy-Item -LiteralPath $releaseExecutable -Destination $packagedExecutable
$packagedSignature = Get-AuthenticodeSignature -LiteralPath $packagedExecutable
if ($packagedSignature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
    $null -eq $packagedSignature.SignerCertificate -or
    $null -eq $packagedSignature.TimeStamperCertificate) {
    throw 'Windows 发布目录中的 vclogg2.exe 未保留有效的 Authenticode 签名和时间戳。'
}
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

Write-Output "便携目录：$stageDirectory"
Write-Output "发布压缩包：$archivePath"
Write-Output "调试符号包（不向用户分发）：$symbolsArchivePath"
Pop-Location
