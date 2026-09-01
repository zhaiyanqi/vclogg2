param(
    [Parameter(Mandatory)]
    [string]$ExecutablePath,
    [Parameter(Mandatory)]
    [string]$CertificatePath,
    [Parameter(Mandatory)]
    [string]$TimestampUrl,
    [string]$CertificatePassword = $env:VCLOGG2_WINDOWS_SIGNING_CERTIFICATE_PASSWORD
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Resolve-SignTool {
    $command = Get-Command -Name 'signtool.exe' -CommandType Application -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return $command.Source
    }

    $programFilesX86 = [Environment]::GetFolderPath(
        [System.Environment+SpecialFolder]::ProgramFilesX86
    )
    if ([string]::IsNullOrWhiteSpace($programFilesX86)) {
        $programFilesX86 = ${env:ProgramFiles(x86)}
    }
    if ([string]::IsNullOrWhiteSpace($programFilesX86)) {
        throw 'Could not locate the 64-bit Windows SDK SignTool.'
    }

    $sdkBin = Join-Path $programFilesX86 'Windows Kits\10\bin'
    $candidates = @(
        Get-ChildItem -Path (Join-Path $sdkBin '*\x64\signtool.exe') `
            -File -ErrorAction SilentlyContinue
    )
    $directCandidate = Join-Path $sdkBin 'x64\signtool.exe'
    if (Test-Path -LiteralPath $directCandidate -PathType Leaf) {
        $candidates += Get-Item -LiteralPath $directCandidate
    }
    $candidate = $candidates |
        Sort-Object -Property FullName -Descending |
        Select-Object -First 1
    if ($null -eq $candidate) {
        throw "Could not locate SignTool under: $sdkBin"
    }
    return $candidate.FullName
}

$resolvedExecutable = [System.IO.Path]::GetFullPath($ExecutablePath)
$resolvedCertificate = [System.IO.Path]::GetFullPath($CertificatePath)
if (-not (Test-Path -LiteralPath $resolvedExecutable -PathType Leaf)) {
    throw "Executable to sign does not exist: $resolvedExecutable"
}
if (-not (Test-Path -LiteralPath $resolvedCertificate -PathType Leaf)) {
    throw "Signing certificate does not exist: $resolvedCertificate"
}
if ([System.IO.Path]::GetExtension($resolvedCertificate) -ne '.pfx') {
    throw 'The signing certificate must be a PKCS #12 .pfx file.'
}

$timestampUri = $null
if (-not [Uri]::TryCreate($TimestampUrl, [UriKind]::Absolute, [ref]$timestampUri) -or
    $timestampUri.Scheme -notin @('http', 'https')) {
    throw "The RFC 3161 timestamp URL must be an absolute HTTP(S) URL: $TimestampUrl"
}

$signTool = Resolve-SignTool
$signArguments = @(
    'sign',
    '/fd', 'SHA256',
    '/tr', $timestampUri.AbsoluteUri,
    '/td', 'SHA256',
    '/f', $resolvedCertificate,
    '/d', 'VCLogg2',
    '/du', 'https://github.com/zhaiyanqi/vclogg2'
)
if (-not [string]::IsNullOrEmpty($CertificatePassword)) {
    $signArguments += @('/p', $CertificatePassword)
}
$signArguments += $resolvedExecutable

try {
    & $signTool @signArguments
    if ($LASTEXITCODE -ne 0) {
        throw "SignTool failed to sign the executable (exit code $LASTEXITCODE)."
    }
} finally {
    $CertificatePassword = $null
    $signArguments = $null
}

& $signTool verify /pa /all /v $resolvedExecutable
if ($LASTEXITCODE -ne 0) {
    throw "SignTool failed to verify the Authenticode signature (exit code $LASTEXITCODE)."
}

$signature = Get-AuthenticodeSignature -LiteralPath $resolvedExecutable
if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
    throw "Authenticode signature is not valid: $($signature.StatusMessage)"
}
if ($null -eq $signature.SignerCertificate) {
    throw 'The signed executable does not contain a signer certificate.'
}
if ($null -eq $signature.TimeStamperCertificate) {
    throw 'The signed executable does not contain an RFC 3161 timestamp.'
}

Write-Output "Authenticode signer: $($signature.SignerCertificate.Subject)"
Write-Output "Signer thumbprint: $($signature.SignerCertificate.Thumbprint)"
Write-Output "Timestamp authority: $($signature.TimeStamperCertificate.Subject)"
