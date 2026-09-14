[CmdletBinding()]
param([Parameter(Mandatory = $true)][ValidatePattern('^v\d+\.\d+\.\d+$')][string]$Version)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Push-Location $repo
try {
    $metadata = & cargo metadata --locked --format-version 1 --no-deps | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Could not determine Cargo output directory.' }
    $build = Join-Path $metadata.target_directory 'x86_64-pc-windows-msvc/release'
    $cli = Join-Path $build 'ohm-cli.exe'
    $installers = @(Get-ChildItem (Join-Path $build 'bundle/nsis') -Filter '*-setup.exe' -File)
    if ($installers.Count -ne 1) { throw "Expected exactly one NSIS installer, found $($installers.Count)." }
    if (-not (Test-Path $cli -PathType Leaf)) { throw 'The real Windows CLI build is missing.' }
    $bytes = [IO.File]::ReadAllBytes($cli)
    if ($bytes.Length -lt 64 -or $bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) { throw 'CLI is not a PE binary.' }
    $pe = [BitConverter]::ToInt32($bytes, 60)
    if ($pe -lt 0 -or $pe + 6 -gt $bytes.Length -or [BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550 -or [BitConverter]::ToUInt16($bytes, $pe + 4) -ne 0x8664) {
        throw 'CLI is not a Windows x86_64 executable.'
    }
    $notices = Join-Path $repo 'artifacts/THIRD_PARTY_NOTICES.txt'
    if (-not (Test-Path $notices -PathType Leaf) -or (Get-Item $notices).Length -eq 0) { throw 'Dependency notices are required.' }
    $output = Join-Path $repo 'artifacts/release'
    if (Test-Path $output) { throw "Output already exists; use a clean release build: $output" }
    $stage = Join-Path $repo 'artifacts/cli-package'
    if (Test-Path $stage) { throw "Staging directory already exists: $stage" }
    New-Item $output, $stage -ItemType Directory | Out-Null
    Copy-Item $cli (Join-Path $stage 'ohm-cli.exe')
    Copy-Item (Join-Path $repo 'LICENSE') $stage
    Copy-Item $notices $stage
    $sha = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read release source commit.' }
    @{
        version = $Version
        source_commit = $sha
        platform = 'windows-x86_64'
        repository = 'SvenKunkka/OpenHardwareOS'
        rust = (& rustc --version)
        workflow_run = $env:GITHUB_RUN_ID
    } | ConvertTo-Json | Set-Content (Join-Path $stage 'release.json') -Encoding UTF8
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath (Join-Path $output "ohm-cli-$Version-windows-x86_64.zip")
    Copy-Item $installers[0].FullName (Join-Path $output "OpenHardwareOS-$Version-windows-x86_64-setup.exe")
    Copy-Item (Join-Path $repo 'scripts/install.ps1') $output
    Copy-Item (Join-Path $repo 'LICENSE') $output
    Copy-Item $notices $output
    Copy-Item (Join-Path $stage 'release.json') $output
    $sums = @(Get-ChildItem $output -File | Sort-Object Name | ForEach-Object {
        '{0}  {1}' -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $_.Name
    })
    [IO.File]::WriteAllLines((Join-Path $output 'SHA256SUMS'), $sums, [Text.UTF8Encoding]::new($false))
    Get-ChildItem $output | Select-Object Name, Length
    Write-Host "Source commit: $sha"
} finally {
    Pop-Location
}
