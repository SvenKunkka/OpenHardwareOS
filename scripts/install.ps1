<#
.SYNOPSIS
Installs the verified OpenHardwareOS Windows x64 CLI from a fixed GitHub release.
.EXAMPLE
.\install.ps1 -Version v0.1.0
.EXAMPLE
.\install.ps1 -Version v0.1.0 -Desktop
.EXAMPLE
.\install.ps1 -Uninstall
.NOTES
Requires Windows x64 and PowerShell 5.1+. CLI installation is per-user and does not
change PATH. -Desktop also runs the NSIS installer silently (Windows may request
administrator approval); it does not launch the application. Desktop removal is
through Windows Settings > Apps. No execution-policy or security settings change.
#>
[CmdletBinding()]
param(
    [ValidatePattern('^v\d+\.\d+\.\d+$')][string]$Version = 'v0.1.0',
    [switch]$Desktop,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Assert-WindowsX64 {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'This installer requires Windows x64.' }
    $architecture = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
    if ($architecture -ne 'AMD64') { throw "Unsupported architecture: $architecture. This release requires x64 Windows." }
}

function Receive-ReleaseFile([string]$Uri, [string]$Destination) {
    Invoke-WebRequest -UseBasicParsing -Uri $Uri -OutFile $Destination -ErrorAction Stop
}

function Assert-ReleaseChecksum([string]$Path, [string]$Name, [string[]]$Manifest) {
    $entries = @($Manifest | Where-Object { $_ -match ('^[0-9a-fA-F]{64}  ' + [Regex]::Escape($Name) + '$') })
    if ($entries.Count -ne 1) { throw "Expected exactly one SHA256 entry for $Name." }
    $expected = $entries[0].Substring(0, 64)
    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    if ($actual -ne $expected) { throw "SHA256 mismatch for $Name. Nothing from this download will be executed." }
}

function Expand-CliPackage([string]$Archive, [string]$Destination, [string]$ExpectedVersion) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        $required = @('ohm-cli.exe', 'LICENSE', 'THIRD_PARTY_NOTICES.txt', 'release.json')
        $names = @($zip.Entries | ForEach-Object { $_.FullName })
        if ($names.Count -ne $required.Count) { throw 'Unexpected CLI archive contents.' }
        foreach ($name in $required) {
            if (@($names | Where-Object { $_ -ceq $name }).Count -ne 1) { throw "Invalid or missing CLI archive entry: $name" }
        }
    } finally { $zip.Dispose() }
    [IO.Compression.ZipFile]::ExtractToDirectory($Archive, $Destination)
    $release = Get-Content -LiteralPath (Join-Path $Destination 'release.json') -Raw | ConvertFrom-Json
    if ($release.version -ne $ExpectedVersion -or $release.platform -ne 'windows-x86_64' -or $release.repository -ne 'SvenKunkka/OpenHardwareOS') {
        throw 'CLI package metadata does not match the requested release.'
    }
    if ((Get-Item -LiteralPath (Join-Path $Destination 'ohm-cli.exe')).Length -eq 0) { throw 'CLI executable is empty.' }
}

function Invoke-DesktopInstaller([string]$Path) {
    # /S is the Tauri NSIS silent switch. Do not pass /R (run after install).
    $process = Start-Process -FilePath $Path -ArgumentList '/S' -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Desktop installer failed (exit $($process.ExitCode))." }
}

function Install-OpenHardwareOS([string]$ReleaseVersion, [bool]$InstallDesktop, [bool]$RemoveCli) {
    Assert-WindowsX64
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) { throw 'LOCALAPPDATA is unavailable.' }
    $base = Join-Path $env:LOCALAPPDATA 'OpenHardwareOS'
    $destination = Join-Path $base 'cli'
    $markerName = '.openhardwareos-installer'
    if ($RemoveCli) {
        if ($InstallDesktop) { throw 'Use Windows Settings > Apps to uninstall the desktop application.' }
        if (-not (Test-Path -LiteralPath $destination)) { Write-Host 'CLI is not installed.'; return }
        if (-not (Test-Path -LiteralPath (Join-Path $destination $markerName) -PathType Leaf)) { throw 'Unrecognized CLI directory; refusing to remove it.' }
        Remove-Item -LiteralPath $destination -Recurse -Force
        Write-Host 'OpenHardwareOS CLI removed. Application configuration and desktop installation were preserved.'
        return
    }
    if ($ReleaseVersion -notmatch '^v\d+\.\d+\.\d+$') { throw 'Expected a version such as v0.1.0.' }
    if ((Test-Path -LiteralPath $destination) -and -not (Test-Path -LiteralPath (Join-Path $destination $markerName) -PathType Leaf)) {
        throw "Existing directory is not managed by this installer: $destination"
    }
    $downloadRoot = 'https://github.com/SvenKunkka/OpenHardwareOS/releases/download/' + $ReleaseVersion
    $cliName = "ohm-cli-$ReleaseVersion-windows-x86_64.zip"
    $desktopName = "OpenHardwareOS-$ReleaseVersion-windows-x86_64-setup.exe"
    $temporary = Join-Path ([IO.Path]::GetTempPath()) ('ohm-install-' + [Guid]::NewGuid().ToString('N'))
    $staging = Join-Path $base ('cli-staging-' + [Guid]::NewGuid().ToString('N'))
    $backup = Join-Path $base ('cli-backup-' + [Guid]::NewGuid().ToString('N'))
    $oldTls = [Net.ServicePointManager]::SecurityProtocol
    try {
        [Net.ServicePointManager]::SecurityProtocol = $oldTls -bor [Net.SecurityProtocolType]::Tls12
        New-Item $temporary -ItemType Directory | Out-Null
        $manifestPath = Join-Path $temporary 'SHA256SUMS'
        Receive-ReleaseFile "$downloadRoot/SHA256SUMS" $manifestPath
        $manifest = @(Get-Content -LiteralPath $manifestPath)
        $assets = @($cliName)
        if ($InstallDesktop) { $assets += $desktopName }
        foreach ($name in $assets) {
            $file = Join-Path $temporary $name
            Receive-ReleaseFile "$downloadRoot/$name" $file
            Assert-ReleaseChecksum $file $name $manifest
        }
        New-Item $base -ItemType Directory -Force | Out-Null
        Expand-CliPackage (Join-Path $temporary $cliName) $staging $ReleaseVersion
        Set-Content -LiteralPath (Join-Path $staging $markerName) -Value 'OpenHardwareOS CLI installer v1' -Encoding ASCII
        if ($InstallDesktop) {
            Write-Host 'Installing desktop application; Windows may request administrator approval.'
            Invoke-DesktopInstaller (Join-Path $temporary $desktopName)
        }
        if (Test-Path -LiteralPath $destination) { Move-Item -LiteralPath $destination -Destination $backup }
        try { Move-Item -LiteralPath $staging -Destination $destination }
        catch {
            if (Test-Path -LiteralPath $backup) { Move-Item -LiteralPath $backup -Destination $destination }
            throw
        }
        if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Recurse -Force }
        Write-Host "Installed CLI $ReleaseVersion to $destination"
        Write-Host ('Run: & "' + (Join-Path $destination 'ohm-cli.exe') + '" --help')
        Write-Host 'PATH was not changed; the absolute command works in this and new shells.'
        Write-Host 'Remove CLI: .\install.ps1 -Uninstall. Remove desktop: Windows Settings > Apps.'
    } finally {
        [Net.ServicePointManager]::SecurityProtocol = $oldTls
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Recurse -Force }
        if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
    }
}

# Dot-sourcing exposes the functions to isolated tests without installing.
if ($MyInvocation.InvocationName -ne '.') {
    try { Install-OpenHardwareOS $Version $Desktop.IsPresent $Uninstall.IsPresent }
    catch { Write-Error $_; exit 1 }
}
