[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

function Resolve-NodeExecutable {
  $nodeCommand = Get-Command node.exe -ErrorAction SilentlyContinue
  if ($nodeCommand) {
    return $nodeCommand.Source
  }
  if ($env:npm_node_execpath -and (Test-Path -LiteralPath $env:npm_node_execpath)) {
    return (Resolve-Path -LiteralPath $env:npm_node_execpath).Path
  }
  $pnpmCommand = Get-Command pnpm.cmd -ErrorAction SilentlyContinue
  if (-not $pnpmCommand) {
    $pnpmCommand = Get-Command pnpm -ErrorAction SilentlyContinue
  }
  if (-not $pnpmCommand) {
    throw 'Neither Node.js nor pnpm is available on PATH.'
  }
  $pnpmDirectory = Split-Path -Parent $pnpmCommand.Source
  foreach ($candidate in @(
    (Join-Path $pnpmDirectory 'node.exe'),
    (Join-Path $pnpmDirectory '..\..\node\bin\node.exe')
  )) {
    if (Test-Path -LiteralPath $candidate) {
      return (Resolve-Path -LiteralPath $candidate).Path
    }
  }
  throw 'Node.js is required, but no executable could be resolved from PATH or the pnpm launcher.'
}

function Resolve-GoExecutable {
  $goCommand = Get-Command go.exe -ErrorAction SilentlyContinue
  if ($goCommand) {
    return $goCommand.Source
  }
  $candidates = @()
  foreach ($scope in @('User', 'Machine')) {
    $configuredPath = [Environment]::GetEnvironmentVariable('Path', $scope)
    if ($configuredPath) {
      $candidates += $configuredPath -split ';' | Where-Object { $_ } | ForEach-Object { Join-Path $_ 'go.exe' }
    }
  }
  $candidates += @('D:\env\go\bin\go.exe', (Join-Path $env:ProgramFiles 'Go\bin\go.exe'))
  foreach ($candidate in $candidates | Select-Object -Unique) {
    if (Test-Path -LiteralPath $candidate) {
      return (Resolve-Path -LiteralPath $candidate).Path
    }
  }
  throw 'Go 1.24 or newer is required, but go.exe was not found on the current or configured Windows PATH.'
}

function Invoke-VerificationStep {
  param([string]$Label, [scriptblock]$Command)
  Write-Host "==> $Label"
  $global:LASTEXITCODE = 0
  & $Command
  if ($LASTEXITCODE -ne 0) {
    throw "$Label failed with exit code $LASTEXITCODE."
  }
}

$nodeExecutable = Resolve-NodeExecutable
$nodeDirectory = Split-Path -Parent $nodeExecutable
if (($env:PATH -split ';') -notcontains $nodeDirectory) {
  $env:PATH = "$nodeDirectory;$env:PATH"
}
$goExecutable = Resolve-GoExecutable
$env:GOCACHE = Join-Path $root 'tmp\go-cache'
$env:GOPATH = Join-Path $root 'tmp\go-path'
New-Item -ItemType Directory -Force -Path $env:GOCACHE, $env:GOPATH | Out-Null
Write-Host "Using Node.js: $nodeExecutable"
Write-Host "Using Go: $goExecutable"

Push-Location $root
try {
  Invoke-VerificationStep 'Administrator console typecheck' {
    & $nodeExecutable (Join-Path $root 'node_modules\vue-tsc\bin\vue-tsc.js') --noEmit -p (Join-Path $root 'admin-web\tsconfig.json')
  }
  Invoke-VerificationStep 'Embedded administrator console assets' {
    $verificationOutput = Join-Path $root 'tmp\admin-web-verification'
    $embeddedOutput = Join-Path $root 'internal\app\admin'
    $previousOutput = $env:VCLOGG_ADMIN_OUT_DIR
    try {
      $env:VCLOGG_ADMIN_OUT_DIR = $verificationOutput
      & $nodeExecutable (Join-Path $root 'node_modules\vite\bin\vite.js') build --config (Join-Path $root 'admin-web\vite.config.ts')
      if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    } finally {
      $env:VCLOGG_ADMIN_OUT_DIR = $previousOutput
    }
    $expectedFiles = Get-ChildItem -LiteralPath $verificationOutput -Recurse -File | ForEach-Object {
      [PSCustomObject]@{ Relative = $_.FullName.Substring($verificationOutput.Length + 1); Hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
    }
    $embeddedFiles = Get-ChildItem -LiteralPath $embeddedOutput -Recurse -File | ForEach-Object {
      [PSCustomObject]@{ Relative = $_.FullName.Substring($embeddedOutput.Length + 1); Hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
    }
    if (Compare-Object $expectedFiles $embeddedFiles -Property Relative, Hash) {
      throw 'Embedded administrator console assets are stale. Run pnpm run build:admin and commit the result.'
    }
    if (Get-ChildItem -LiteralPath $embeddedOutput -Recurse -File | Select-String -Pattern '(?:src|href)=["'']https?://' -CaseSensitive:$false) {
      throw 'Embedded administrator console must not load external assets.'
    }
  }
  Invoke-VerificationStep 'Go formatting' {
    $gofmtCommand = Join-Path (Split-Path -Parent $goExecutable) 'gofmt.exe'
    $goSourceRoots = @(
      (Join-Path $root 'cmd'),
      (Join-Path $root 'internal')
    )
    $goFiles = Get-ChildItem -LiteralPath $goSourceRoots -Recurse -Filter '*.go' -File | Select-Object -ExpandProperty FullName
    $unformatted = & $gofmtCommand -l $goFiles
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    if ($unformatted) { throw "Unformatted Go files: $($unformatted -join ', ')" }
  }
  Invoke-VerificationStep 'Go vet' { & $goExecutable -C $root vet ./... }
  Invoke-VerificationStep 'Go server tests' { & $goExecutable -C $root test ./... }
  Invoke-VerificationStep 'Go host build' {
    $output = Join-Path $root 'tmp\server-verification'
    New-Item -ItemType Directory -Force -Path $output | Out-Null
    & $goExecutable -C $root build -o (Join-Path $output 'vclogg-server-host.exe') ./cmd/vclogg-server
  }
  Invoke-VerificationStep 'Go CGO-free Windows/Linux builds' {
    $output = Join-Path $root 'tmp\server-verification'
    New-Item -ItemType Directory -Force -Path $output | Out-Null
    $previousGoos = $env:GOOS
    $previousGoarch = $env:GOARCH
    $previousCgo = $env:CGO_ENABLED
    try {
      $env:GOARCH = 'amd64'
      $env:CGO_ENABLED = '0'
      $env:GOOS = 'windows'
      & $goExecutable -C $root build -o (Join-Path $output 'vclogg-server-windows-amd64.exe') ./cmd/vclogg-server
      if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
      $env:GOOS = 'linux'
      & $goExecutable -C $root build -o (Join-Path $output 'vclogg-server-linux-amd64') ./cmd/vclogg-server
    } finally {
      $env:GOOS = $previousGoos
      $env:GOARCH = $previousGoarch
      $env:CGO_ENABLED = $previousCgo
    }
  }
} finally {
  Pop-Location
}
