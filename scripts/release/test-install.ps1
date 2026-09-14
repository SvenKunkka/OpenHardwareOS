# Behavior tests using local download fixtures; no fixture executable is run.
# Works with PowerShell 5.1 on Windows and PowerShell 7 on other platforms.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot '../install.ps1')

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ohm-installer-test-' + [Guid]::NewGuid().ToString('N'))
$priorLocalAppData = $env:LOCALAPPDATA
$priorPath = $env:PATH
$script:fixture = Join-Path $testRoot 'downloads'
$script:failAsset = ''
$script:desktopCalls = 0
$script:desktopFailure = $false
$script:passed = 0
$script:downloadCalls = 0
$version = 'v0.1.0'
$cliName = "ohm-cli-$version-windows-x86_64.zip"
$desktopName = "OpenHardwareOS-$version-windows-x86_64-setup.exe"

function Require([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Require-Failure([scriptblock]$Action, [string]$Expected) {
    $failure = $null
    try { & $Action } catch { $failure = $_.ToString() }
    Require ($null -ne $failure) "Expected failure containing: $Expected"
    Require ($failure -like "*$Expected*") "Wrong failure: $failure"
}
function Test-Case([string]$Name, [scriptblock]$Action) {
    & $Action
    $script:passed += 1
    Write-Host "PASS $Name"
}

# Check the actual platform guard before replacing it for portable fixture tests.
Test-Case 'Reject non-Windows or unsupported Windows architecture' {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
        Require-Failure { Assert-WindowsX64 } 'requires Windows x64'
    } else {
        $savedNative = $env:PROCESSOR_ARCHITECTURE
        $savedWow64 = $env:PROCESSOR_ARCHITEW6432
        try {
            $env:PROCESSOR_ARCHITECTURE = 'ARM64'
            $env:PROCESSOR_ARCHITEW6432 = ''
            Require-Failure { Assert-WindowsX64 } 'Unsupported architecture'
        } finally {
            $env:PROCESSOR_ARCHITECTURE = $savedNative
            $env:PROCESSOR_ARCHITEW6432 = $savedWow64
        }
    }
}
function Assert-WindowsX64 { }
function Receive-ReleaseFile([string]$Uri, [string]$Destination) {
    $script:downloadCalls += 1
    Require ($Uri.StartsWith("https://github.com/SvenKunkka/OpenHardwareOS/releases/download/$script:version/", [StringComparison]::Ordinal)) 'Download escaped the requested release URL.'
    $name = $Uri.Substring($Uri.LastIndexOf('/') + 1)
    if ($name -eq $script:failAsset) { throw 'Injected download failure' }
    Copy-Item -LiteralPath (Join-Path $script:fixture $name) -Destination $Destination
}
function Invoke-DesktopInstaller([string]$Path) {
    $script:desktopCalls += 1
    Require ((Get-Content -LiteralPath $Path -Raw) -eq 'desktop-fixture') 'Unexpected desktop contents.'
    if ($script:desktopFailure) { throw 'Injected desktop failure' }
}
function Update-Manifest {
    $lines = @(Get-ChildItem $script:fixture -File | Where-Object Name -ne 'SHA256SUMS' | Sort-Object Name | ForEach-Object {
        '{0}  {1}' -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $_.Name
    })
    Set-Content (Join-Path $script:fixture 'SHA256SUMS') $lines -Encoding ASCII
}
function Set-Fixture([string]$BinaryText = 'cli-fixture', [string]$MetadataVersion = $script:version, [bool]$Traversal = $false) {
    if (Test-Path $script:fixture) { Remove-Item $script:fixture -Recurse -Force }
    New-Item $script:fixture -ItemType Directory -Force | Out-Null
    $zip = [IO.Compression.ZipFile]::Open((Join-Path $script:fixture $cliName), [IO.Compression.ZipArchiveMode]::Create)
    try {
        $contents = @{
            'ohm-cli.exe' = $BinaryText
            'LICENSE' = 'fixture licence'
            'THIRD_PARTY_NOTICES.txt' = 'fixture dependency notices'
            'release.json' = (@{ version = $MetadataVersion; platform = 'windows-x86_64'; repository = 'SvenKunkka/OpenHardwareOS' } | ConvertTo-Json)
        }
        if ($Traversal) { $contents.Remove('LICENSE'); $contents['../escaped.txt'] = 'must never be extracted' }
        foreach ($name in $contents.Keys) {
            $entry = $zip.CreateEntry($name)
            $writer = [IO.StreamWriter]::new($entry.Open())
            try { $writer.Write($contents[$name]) } finally { $writer.Dispose() }
        }
    } finally { $zip.Dispose() }
    [IO.File]::WriteAllText((Join-Path $script:fixture $desktopName), 'desktop-fixture')
    Update-Manifest
}
function Installed-Text { Get-Content (Join-Path $env:LOCALAPPDATA 'OpenHardwareOS/cli/ohm-cli.exe') -Raw }

try {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $env:LOCALAPPDATA = Join-Path $testRoot 'local-app-data'
    New-Item $testRoot -ItemType Directory | Out-Null
    Test-Case 'Missing version is rejected before any download or installation' {
        Require-Failure { Install-OpenHardwareOS '' $false $false } 'requires an explicit -Version'
        Require ($script:downloadCalls -eq 0) 'Missing version attempted a download.'
        Require (-not (Test-Path (Join-Path $env:LOCALAPPDATA 'OpenHardwareOS/cli'))) 'Missing version created an installation.'
    }
    Test-Case 'Latest and prerelease suffixes are rejected' {
        foreach ($invalidVersion in @('latest', 'v0.2.0-rc.1')) {
            Require-Failure { Install-OpenHardwareOS $invalidVersion $false $false } 'Expected version vMAJOR.MINOR.PATCH'
        }
        Require ($script:downloadCalls -eq 0) 'Invalid version attempted a download.'
    }
    Test-Case 'Verified CLI install preserves PATH and never invokes executable' {
        Set-Fixture
        Install-OpenHardwareOS $version $false $false
        Require ((Installed-Text) -eq 'cli-fixture') 'CLI payload did not install.'
        Require ($script:desktopCalls -eq 0) 'Default installation invoked desktop.'
        Require ($env:PATH -eq $priorPath) 'Installer changed PATH.'
    }
    Test-Case 'Download failure preserves existing installation' {
        $script:failAsset = $cliName
        try { Require-Failure { Install-OpenHardwareOS $version $false $false } 'Injected download failure' }
        finally { $script:failAsset = '' }
        Require ((Installed-Text) -eq 'cli-fixture') 'Failed download changed installed files.'
    }
    Test-Case 'Bad checksum aborts before desktop execution or CLI replacement' {
        Set-Fixture 'new-fixture'
        Add-Content (Join-Path $script:fixture $desktopName) 'corruption'
        Require-Failure { Install-OpenHardwareOS $version $true $false } 'SHA256 mismatch'
        Require ($script:desktopCalls -eq 0) 'Bad checksum reached installer execution.'
        Require ((Installed-Text) -eq 'cli-fixture') 'Bad checksum replaced CLI.'
    }
    Test-Case 'Duplicate manifest entry fails closed' {
        Set-Fixture
        $line = Get-Content (Join-Path $script:fixture 'SHA256SUMS') | Where-Object { $_.EndsWith($cliName) }
        Add-Content (Join-Path $script:fixture 'SHA256SUMS') $line
        Require-Failure { Install-OpenHardwareOS $version $false $false } 'exactly one SHA256'
    }
    Test-Case 'Checksum-valid archive with traversal never extracts' {
        Set-Fixture 'evil' 'v0.1.0' $true
        Require-Failure { Install-OpenHardwareOS $version $false $false } 'Invalid or missing CLI archive entry'
        Require (-not (Test-Path (Join-Path $env:LOCALAPPDATA 'OpenHardwareOS/escaped.txt'))) 'Traversal wrote outside staging.'
        Require ((Installed-Text) -eq 'cli-fixture') 'Malformed archive replaced CLI.'
    }
    Test-Case 'Package version mismatch is rejected' {
        Set-Fixture 'wrong-version' 'v9.9.9'
        Require-Failure { Install-OpenHardwareOS $version $false $false } 'metadata does not match'
        Require ((Installed-Text) -eq 'cli-fixture') 'Wrong-version package replaced CLI.'
    }
    Test-Case 'Desktop installer failure preserves existing CLI' {
        Set-Fixture 'new-fixture'
        $script:desktopFailure = $true
        try { Require-Failure { Install-OpenHardwareOS $version $true $false } 'Injected desktop failure' }
        finally { $script:desktopFailure = $false }
        Require ((Installed-Text) -eq 'cli-fixture') 'Failed desktop install replaced CLI.'
    }
    Test-Case 'Successful update uses verified desktop and replaces CLI' {
        Set-Fixture 'new-fixture'
        Install-OpenHardwareOS $version $true $false
        Require ((Installed-Text) -eq 'new-fixture') 'Update did not replace CLI.'
        Require ($script:desktopCalls -eq 2) 'Unexpected desktop invocation count.'
        Require (@(Get-ChildItem (Join-Path $env:LOCALAPPDATA 'OpenHardwareOS') -Directory).Count -eq 1) 'Temporary install or backup folder remains.'
    }
    Test-Case 'Explicit second version selects its own assets and package metadata' {
        $script:version = 'v0.2.0'
        $script:cliName = "ohm-cli-$script:version-windows-x86_64.zip"
        $script:desktopName = "OpenHardwareOS-$script:version-windows-x86_64-setup.exe"
        Set-Fixture 'second-version-fixture'
        Install-OpenHardwareOS $version $false $false
        Require ((Installed-Text) -eq 'second-version-fixture') 'Requested second version did not replace the CLI.'
        $installedMetadata = Get-Content (Join-Path $env:LOCALAPPDATA 'OpenHardwareOS/cli/release.json') -Raw | ConvertFrom-Json
        Require ($installedMetadata.version -ceq $version) 'Installed metadata does not match the explicit second version.'
    }
    Test-Case 'Uninstall preserves unrelated configuration and rejects unmanaged directory' {
        $config = Join-Path $env:LOCALAPPDATA 'OpenHardwareOS/config.json'
        Set-Content $config 'preserve'
        Install-OpenHardwareOS '' $false $true
        Require (Test-Path $config) 'Uninstall deleted unrelated configuration.'
        $unmanaged = Join-Path $env:LOCALAPPDATA 'OpenHardwareOS/cli'
        New-Item $unmanaged -ItemType Directory | Out-Null
        Set-Content (Join-Path $unmanaged 'user.txt') 'preserve'
        Require-Failure { Install-OpenHardwareOS $version $false $true } 'Unrecognized CLI directory'
        Require-Failure { Install-OpenHardwareOS $version $false $false } 'not managed by this installer'
        Require (Test-Path (Join-Path $unmanaged 'user.txt')) 'Installer changed unmanaged data.'
    }
    Write-Host "Installer behavior tests: $script:passed passed. Fixture executables were never run."
} finally {
    $env:LOCALAPPDATA = $priorLocalAppData
    if (Test-Path $testRoot) { Remove-Item $testRoot -Recurse -Force }
}
