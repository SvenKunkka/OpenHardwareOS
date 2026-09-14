# Exercise the actual Windows release ZIP and installed binary on the CI runner.
# Only the network transport is replaced, because release assets are not public
# until this check passes. Desktop installation and real hardware are excluded.
[CmdletBinding()]
param([Parameter(Mandatory = $true)][ValidatePattern('^v\d+\.\d+\.\d+$')][string]$ReleaseVersion)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot '../install.ps1')
Assert-WindowsX64
if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) { throw 'This smoke check requires an isolated CI RUNNER_TEMP.' }

$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$script:smokeAssets = Join-Path $repo 'artifacts/release'
$script:smokeVersion = $ReleaseVersion
function Receive-ReleaseFile([string]$Uri, [string]$Destination) {
    $base = "https://github.com/SvenKunkka/OpenHardwareOS/releases/download/$script:smokeVersion/"
    if (-not $Uri.StartsWith($base, [StringComparison]::Ordinal)) { throw "Unexpected release URL: $Uri" }
    $name = $Uri.Substring($base.Length)
    $allowed = @('SHA256SUMS', "ohm-cli-$script:smokeVersion-windows-x86_64.zip")
    if ($name -cnotin $allowed) { throw "Unexpected release asset: $name" }
    Copy-Item -LiteralPath (Join-Path $script:smokeAssets $name) -Destination $Destination
}

$isolation = Join-Path $env:RUNNER_TEMP ('ohm-release-smoke-' + [Guid]::NewGuid().ToString('N'))
$previousLocalAppData = $env:LOCALAPPDATA
$previousConfig = $env:OHM_CONFIG_DIR
try {
    $env:LOCALAPPDATA = Join-Path $isolation 'local-app-data'
    $env:OHM_CONFIG_DIR = Join-Path $isolation 'runtime-config'
    New-Item $env:OHM_CONFIG_DIR -ItemType Directory -Force | Out-Null
    $sentinel = Join-Path $env:OHM_CONFIG_DIR 'preserve-on-uninstall.txt'
    Set-Content $sentinel 'unrelated configuration must survive uninstall' -Encoding ASCII

    Install-OpenHardwareOS $ReleaseVersion $false $false
    $installed = Join-Path $env:LOCALAPPDATA 'OpenHardwareOS/cli/ohm-cli.exe'
    $actualVersion = (& $installed --version | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Installed CLI --version failed.' }
    $expectedVersion = 'ohm-cli ' + $ReleaseVersion.Substring(1)
    if ($actualVersion -cne $expectedVersion) { throw "Expected '$expectedVersion'; received '$actualVersion'." }
    Write-Host "Installed binary version verified: $actualVersion"
    & $installed demo --steps 10
    if ($LASTEXITCODE -ne 0) { throw 'Installed CLI simulated cooling demo failed.' }
    $configBefore = @(Get-ChildItem $env:OHM_CONFIG_DIR -File -Recurse | Sort-Object FullName | Get-FileHash -Algorithm SHA256 | Select-Object Path, Hash) | ConvertTo-Json
    Install-OpenHardwareOS $ReleaseVersion $false $true
    if (Test-Path -LiteralPath $installed) { throw 'CLI executable remains after uninstall.' }
    $configAfter = @(Get-ChildItem $env:OHM_CONFIG_DIR -File -Recurse | Sort-Object FullName | Get-FileHash -Algorithm SHA256 | Select-Object Path, Hash) | ConvertTo-Json
    if ($configBefore -cne $configAfter -or -not (Test-Path $sentinel)) { throw 'Uninstall changed the isolated runtime configuration.' }
    Write-Host 'PASS: actual Windows release ZIP installed, CLI version and simulated demo succeeded, uninstall preserved runtime configuration.'
    if ($env:GITHUB_STEP_SUMMARY) {
        "Actual Windows x64 release ZIP: install, $actualVersion, 10-step simulated demo, and uninstall preservation passed. No desktop installer or real hardware was exercised." >> $env:GITHUB_STEP_SUMMARY
    }
} finally {
    $env:LOCALAPPDATA = $previousLocalAppData
    $env:OHM_CONFIG_DIR = $previousConfig
    if (Test-Path $isolation) { Remove-Item $isolation -Recurse -Force }
}
