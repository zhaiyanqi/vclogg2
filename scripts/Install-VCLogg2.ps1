param(
    [string]$InstallDirectory = (Join-Path $env:LOCALAPPDATA 'Programs\VCLogg2'),
    [switch]$Launch
)

$ErrorActionPreference = 'Stop'

$sourceExecutable = Join-Path $PSScriptRoot 'vclogg2.exe'
if (-not (Test-Path -LiteralPath $sourceExecutable -PathType Leaf)) {
    throw "安装包不完整，未找到：$sourceExecutable"
}

$resolvedInstallDirectory = [System.IO.Path]::GetFullPath($InstallDirectory)
if ([string]::IsNullOrWhiteSpace($resolvedInstallDirectory)) {
    throw '安装目录不能为空。'
}

New-Item -ItemType Directory -Path $resolvedInstallDirectory -Force | Out-Null
$installedExecutable = Join-Path $resolvedInstallDirectory 'vclogg2.exe'
Copy-Item -LiteralPath $sourceExecutable -Destination $installedExecutable -Force

$installedSymbols = Join-Path $resolvedInstallDirectory 'vclogg2.pdb'
if (Test-Path -LiteralPath $installedSymbols -PathType Leaf) {
    Remove-Item -LiteralPath $installedSymbols -Force
}

$readme = Join-Path $PSScriptRoot 'README.md'
if (Test-Path -LiteralPath $readme -PathType Leaf) {
    Copy-Item -LiteralPath $readme -Destination (Join-Path $resolvedInstallDirectory 'README.md') -Force
}

$startMenuDirectory = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
New-Item -ItemType Directory -Path $startMenuDirectory -Force | Out-Null
$shortcutPath = Join-Path $startMenuDirectory 'VCLogg2.lnk'
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = $installedExecutable
$shortcut.WorkingDirectory = $resolvedInstallDirectory
$shortcut.Description = 'VCLogg2 large log viewer'
$shortcut.Save()

$supportedExtensions = @('.log', '.txt', '.out', '.trace', '.csv', '.json')
$classesRoot = 'HKCU:\Software\Classes'
$applicationKey = Join-Path $classesRoot 'Applications\vclogg2.exe'
$applicationCommandKey = Join-Path $applicationKey 'shell\open\command'
$applicationIconKey = Join-Path $applicationKey 'DefaultIcon'
$supportedTypesKey = Join-Path $applicationKey 'SupportedTypes'
$progId = 'VCLogg2.LogFile'
$progIdKey = Join-Path $classesRoot $progId
$progIdCommandKey = Join-Path $progIdKey 'shell\open\command'
$progIdIconKey = Join-Path $progIdKey 'DefaultIcon'
$capabilitiesKey = 'HKCU:\Software\VCLogg2\Capabilities'
$fileAssociationsKey = Join-Path $capabilitiesKey 'FileAssociations'
$registeredApplicationsKey = 'HKCU:\Software\RegisteredApplications'
$openCommand = '"{0}" "%1"' -f $installedExecutable
$iconValue = '"{0}",0' -f $installedExecutable

New-Item -Path $applicationCommandKey -Force | Out-Null
New-Item -Path $applicationIconKey -Force | Out-Null
New-Item -Path $supportedTypesKey -Force | Out-Null
Set-Item -LiteralPath $applicationCommandKey -Value $openCommand
Set-Item -LiteralPath $applicationIconKey -Value $iconValue
New-ItemProperty -LiteralPath $applicationKey -Name 'FriendlyAppName' -Value 'VCLogg2' -PropertyType String -Force | Out-Null
foreach ($extension in $supportedExtensions) {
    New-ItemProperty -LiteralPath $supportedTypesKey -Name $extension -Value '' -PropertyType String -Force | Out-Null
}

New-Item -Path $progIdCommandKey -Force | Out-Null
New-Item -Path $progIdIconKey -Force | Out-Null
Set-Item -LiteralPath $progIdKey -Value 'VCLogg2 Log File'
Set-Item -LiteralPath $progIdCommandKey -Value $openCommand
Set-Item -LiteralPath $progIdIconKey -Value $iconValue

New-Item -Path $fileAssociationsKey -Force | Out-Null
New-Item -Path $registeredApplicationsKey -Force | Out-Null
New-ItemProperty -LiteralPath $capabilitiesKey -Name 'ApplicationName' -Value 'VCLogg2' -PropertyType String -Force | Out-Null
New-ItemProperty -LiteralPath $capabilitiesKey -Name 'ApplicationDescription' -Value 'Large log file viewer' -PropertyType String -Force | Out-Null
New-ItemProperty -LiteralPath $capabilitiesKey -Name 'ApplicationIcon' -Value $iconValue -PropertyType String -Force | Out-Null
New-ItemProperty -LiteralPath $registeredApplicationsKey -Name 'VCLogg2' -Value 'Software\VCLogg2\Capabilities' -PropertyType String -Force | Out-Null
foreach ($extension in $supportedExtensions) {
    $openWithProgIdsKey = Join-Path $classesRoot "$extension\OpenWithProgids"
    New-Item -Path $openWithProgIdsKey -Force | Out-Null
    New-ItemProperty -LiteralPath $openWithProgIdsKey -Name $progId -Value '' -PropertyType String -Force | Out-Null
    New-ItemProperty -LiteralPath $fileAssociationsKey -Name $extension -Value $progId -PropertyType String -Force | Out-Null
}

if (-not ('VCLogg2Installer.ShellChange' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace VCLogg2Installer {
    public static class ShellChange {
        [DllImport("shell32.dll")]
        public static extern void SHChangeNotify(uint eventId, uint flags, IntPtr item1, IntPtr item2);
    }
}
'@
}
[VCLogg2Installer.ShellChange]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)

Write-Output "VCLogg2 已安装到：$resolvedInstallDirectory"
Write-Output "开始菜单快捷方式：$shortcutPath"
Write-Output '已注册到 Windows“打开方式”列表；不会自动更改任何文件类型的默认应用。'

if ($Launch) {
    Start-Process -FilePath $installedExecutable
}
