<#
.SYNOPSIS
    Exercises the control flow of the Windows acceptance entry points with test doubles.

.DESCRIPTION
    WHAT THIS IS
    A regression harness for the *scripts* in `scripts\`: what each one does, in what
    order, in which directory, with which environment, and what it does when something
    goes wrong. Every external tool is a **test double** — a fake rustup, cargo, rustc,
    node, npm and npx generated into a temporary directory and put on this process's
    PATH — so no real compiler, no network and no Windows machine is involved.

    WHAT THIS IS NOT
    This is **not evidence that a Windows build succeeds**, and it is not Windows
    acceptance evidence of any kind. It validates script control flow with doubles only.
    Every claim about WMI, NSIS, the registry, NVML, LibreHardwareMonitor or a real fan
    needs a real Windows machine and the ordered checklist; a green run here says
    nothing about any of them. The doubles are also not the real CLI tools: they only
    imitate the argument shapes and exit codes the scripts depend on.

    WHAT IT COVERS
    The fifteen-plus cases below exist because each one is a defect that was real:
    a build that wrote its output into the read-only package (so every later
    verification failed), a second run blocked by the first run's leftovers, a
    `cargo metadata` call that answered about the caller's directory, "BUILD COMPLETE"
    printed for a bundle directory that merely existed, a stale installer passing as
    this run's artefact, a toolchain that reached some Rust invocations and not the
    Tauri build, a tool resolved by bare name, a native command's non-zero exit ignored,
    and a disk-space check that read `/Us` out of a POSIX path and called it a drive.

    USAGE
      pwsh -NoProfile -File docs\windows-validation\package\scripts\tests\run-script-tests.ps1
      pwsh -NoProfile -File ...\run-script-tests.ps1 -Filter installer -KeepTemp

    The harness writes only inside one temporary directory it creates, prints a PASS/FAIL
    summary, and exits non-zero if any assertion fails. On failure the temporary
    directory is kept (and its path printed) so the captured output can be read; pass
    -KeepTemp to keep it either way.

.PARAMETER ScriptsDir
    The directory holding the entry points under test. Default: the assembled package's
    `scripts\` directory when this harness is run from inside one, otherwise the
    `scripts\` directory beside this file.

.PARAMETER Filter
    Only run cases whose name contains this substring.

.PARAMETER KeepTemp
    Keep the temporary directory even when every case passes.
#>
#Requires -Version 7.0
[CmdletBinding()]
param(
    [string]$ScriptsDir = '',
    [string]$Filter = '',
    [switch]$KeepTemp
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

# ---------------------------------------------------------------------------
# Where the scripts under test live
# ---------------------------------------------------------------------------
$script:ScriptsDirFrom = ''
if ([string]::IsNullOrWhiteSpace($ScriptsDir)) {
    # Inside an assembled package (this file ships in the snapshot under
    # docs\windows-validation\package\scripts\tests\), test what the package actually
    # ships in scripts\ — which is the copy an operator runs, and not necessarily the
    # same bytes as the snapshot's copy if the package was assembled from a dirty tree.
    $probe = $PSScriptRoot
    while ($probe) {
        if (Test-Path -LiteralPath (Join-Path $probe 'MANIFEST.sha256') -PathType Leaf) {
            $candidate = Join-Path $probe 'scripts'
            if (Test-Path -LiteralPath (Join-Path $candidate 'build.ps1') -PathType Leaf) {
                $ScriptsDir = $candidate
                $script:ScriptsDirFrom = "the assembled package at $probe"
                break
            }
        }
        $parent = Split-Path -Parent $probe
        if (-not $parent -or $parent -eq $probe) { break }
        $probe = $parent
    }
}
if ([string]::IsNullOrWhiteSpace($ScriptsDir)) {
    $ScriptsDir = (Resolve-Path -LiteralPath (Split-Path -Parent $PSScriptRoot)).Path
    $script:ScriptsDirFrom = "$PSScriptRoot (the scripts directory beside this harness)"
}
$script:ScriptsDir = (Resolve-Path -LiteralPath $ScriptsDir).Path.TrimEnd([char]'\', [char]'/')

foreach ($required in @('_tools.ps1', 'precheck.ps1', 'build.ps1', 'verify-package.ps1', 'verify-package.sh')) {
    if (-not (Test-Path -LiteralPath (Join-Path $script:ScriptsDir $required) -PathType Leaf)) {
        throw "$($script:ScriptsDir) has no $required — point -ScriptsDir at the directory holding the entry points"
    }
}

$script:Pwsh = Join-Path $PSHOME $(if ($IsWindows) { 'pwsh.exe' } else { 'pwsh' })
if (-not (Test-Path -LiteralPath $script:Pwsh -PathType Leaf)) {
    throw "cannot find the PowerShell executable at $script:Pwsh"
}

# ---------------------------------------------------------------------------
# Temporary root: everything the harness writes lives here, and only here
# ---------------------------------------------------------------------------
$TempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('ohm-script-tests-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force -Path $TempRoot | Out-Null
$script:TempRoot = (Resolve-Path -LiteralPath $TempRoot).Path
$script:LogDir = Join-Path $script:TempRoot 'logs'
New-Item -ItemType Directory -Force -Path $script:LogDir | Out-Null

Write-Host ''
Write-Host '================================================================================'
Write-Host ' OpenHardwareOS — acceptance-script control-flow tests (TEST DOUBLES ONLY)'
Write-Host '================================================================================'
Write-Host ''
Write-Host ' These tests exercise the scripts with fake rustup/cargo/rustc/node/npm/npx and a'
Write-Host ' fake Tauri build. They are NOT evidence that a Windows build succeeds: that needs'
Write-Host ' a real Windows machine, and no output from here may be reported as Windows'
Write-Host ' acceptance evidence.'
Write-Host ''
Write-Host (" scripts under test : {0}" -f $script:ScriptsDir)
Write-Host ("   from             : {0}" -f $script:ScriptsDirFrom)
Write-Host (" host               : {0} (PowerShell {1})" -f [System.Runtime.InteropServices.RuntimeInformation]::OSDescription, $PSVersionTable.PSVersion)
Write-Host (" temporary root     : {0}" -f $script:TempRoot)
if ($Filter) { Write-Host (" filter             : {0}" -f $Filter) }
Write-Host ''

# ---------------------------------------------------------------------------
# Test doubles: a fake toolchain, on this process's PATH only
# ---------------------------------------------------------------------------
$script:ImplDir = Join-Path $script:TempRoot 'doubles/impl'
$script:RustBinDir = Join-Path $script:TempRoot 'doubles/rustup-bin'   # stands in for %CARGO_HOME%\bin
$script:NodeBinDir = Join-Path $script:TempRoot 'doubles/node-bin'
$script:DecoyDir = Join-Path $script:TempRoot 'doubles/decoy'
foreach ($dir in @($script:ImplDir, $script:RustBinDir, $script:NodeBinDir, $script:DecoyDir)) {
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
}

function Set-UnixExecutable {
    param([Parameter(Mandatory)][string]$Path)
    if ($IsWindows) { return }
    try {
        $mode = [System.IO.UnixFileMode]::UserRead -bor [System.IO.UnixFileMode]::UserWrite -bor
                [System.IO.UnixFileMode]::UserExecute -bor [System.IO.UnixFileMode]::GroupRead -bor
                [System.IO.UnixFileMode]::GroupExecute -bor [System.IO.UnixFileMode]::OtherRead -bor
                [System.IO.UnixFileMode]::OtherExecute
        [System.IO.File]::SetUnixFileMode($Path, $mode)
    } catch {
        throw "cannot make $Path executable: $($_.Exception.Message)"
    }
}

function New-DoubleWrapper {
    <#
    .SYNOPSIS
        One executable in a tool directory, which runs the matching PowerShell double.
    .DESCRIPTION
        On Windows the wrapper is a .cmd (that is what a PATH lookup finds there); on
        Unix it is an extension-less executable shell script. Both hand their arguments
        straight to the shared double implementation, so the doubles behave the same on
        both platforms.
    #>
    param(
        [Parameter(Mandatory)][string]$Directory,
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][string]$Implementation
    )
    $impl = Join-Path $script:ImplDir "$Implementation.ps1"
    if ($IsWindows) {
        $path = Join-Path $Directory "$Name.cmd"
        $content = @(
            '@echo off',
            "`"$script:Pwsh`" -NoProfile -File `"$impl`" %*"
        ) -join "`r`n"
        Set-Content -LiteralPath $path -Value $content -NoNewline
    } else {
        $path = Join-Path $Directory $Name
        $content = @(
            '#!/bin/sh',
            "exec `"$script:Pwsh`" -NoProfile -File `"$impl`" `"`$@`""
        ) -join "`n"
        Set-Content -LiteralPath $path -Value ($content + "`n") -NoNewline
        Set-UnixExecutable -Path $path
    }
    return $path
}

function Write-DoubleImplementations {
    $common = @'
# Shared behaviour for the test doubles. Generated by run-script-tests.ps1 into a
# temporary directory; nothing here is part of the shipped package.
function Record-Invocation {
    param([string]$Tool, [string[]]$Arguments = @())
    if (-not $env:OHM_CALL_LOG) { return }
    $cargoSeen = ''
    $found = Get-Command -Name cargo -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($found) { $cargoSeen = $found.Source }
    $line = "{0}|args={1}|RUSTUP_TOOLCHAIN={2}|CARGO_TARGET_DIR={3}|cargo-on-PATH={4}|cwd={5}" -f `
        $Tool, ($Arguments -join ' '), $env:RUSTUP_TOOLCHAIN, $env:CARGO_TARGET_DIR, $cargoSeen, (Get-Location).Path
    Add-Content -LiteralPath $env:OHM_CALL_LOG -Value $line
}
function Get-FlagValue {
    param([string[]]$Arguments, [string]$Name)
    for ($i = 0; $i -lt $Arguments.Count - 1; $i++) {
        if ($Arguments[$i] -eq $Name) { return $Arguments[$i + 1] }
    }
    return ''
}
function Assert-ToolchainInherited {
    param([string]$Tool)
    $expected = $env:OHM_REQUIRE_TOOLCHAIN
    if (-not $expected) { return }
    if ($env:RUSTUP_TOOLCHAIN -ne $expected) {
        Write-Output "FAKE ${Tool}: -Toolchain $expected was requested but RUSTUP_TOOLCHAIN is '$($env:RUSTUP_TOOLCHAIN)' in this process, so the toolchain did not reach it"
        Record-Invocation -Tool "$Tool-TOOLCHAIN-MISSING" -Arguments @()
        exit 8
    }
}
function Assert-NotRequestedToFail {
    param([string]$Tool)
    $fail = @($env:OHM_FAKE_FAIL -split ',') | Where-Object { $_.Trim() -ne '' }
    if ($fail -contains $Tool) {
        Write-Output "FAKE ${Tool}: failing on purpose (OHM_FAKE_FAIL)"
        Record-Invocation -Tool "$Tool-FAILING" -Arguments @()
        exit 3
    }
}
'@
    Set-Content -LiteralPath (Join-Path $script:ImplDir '_common.ps1') -Value $common

    $rustup = @'
# A broken double must fail loudly, not print errors and carry on pretending to be a
# tool that worked.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_common.ps1')
$tool = 'rustup'
Record-Invocation -Tool $tool -Arguments @($args)
Assert-NotRequestedToFail -Tool $tool
Assert-ToolchainInherited -Tool $tool
if ($args.Count -eq 0) { 'rustup 1.28.0 (test double)'; exit 0 }
switch ($args[0]) {
    '--version' { 'rustup 1.28.0 (test double)'; exit 0 }
    'toolchain' {
        if ($args.Count -ge 2 -and $args[1] -eq 'list') {
            $names = if ($env:OHM_FAKE_TOOLCHAINS) { $env:OHM_FAKE_TOOLCHAINS } else { '1.98.0' }
            foreach ($name in @($names -split ',')) {
                $trimmed = $name.Trim()
                if ($trimmed) { "$trimmed-x86_64-pc-windows-msvc (active, default)" }
            }
        }
        exit 0
    }
    'run' {
        if ($args.Count -lt 3) { 'rustup run: expected a toolchain and a tool'; exit 2 }
        $toolchain = $args[1]
        $inner = $args[2]
        $rest = if ($args.Count -gt 3) { @($args[3..($args.Count - 1)]) } else { @() }
        if ($env:OHM_REQUIRE_TOOLCHAIN -and $toolchain -ne $env:OHM_REQUIRE_TOOLCHAIN) {
            "FAKE rustup: asked to run '$toolchain' but -Toolchain $($env:OHM_REQUIRE_TOOLCHAIN) was requested"
            exit 8
        }
        Record-Invocation -Tool "rustup-run-$inner" -Arguments $rest
        & (Join-Path $PSScriptRoot '_pwsh.ps1') $inner $rest
        exit $LASTEXITCODE
    }
    default { exit 0 }
}
'@
    Set-Content -LiteralPath (Join-Path $script:ImplDir 'rustup.ps1') -Value $rustup

    # A tiny dispatcher, so `rustup run <toolchain> <tool>` reaches the same double the
    # tool would have reached on a real rustup installation.
    $dispatcher = @'
$tool = $args[0]
$rest = if ($args.Count -gt 1) { @($args[1..($args.Count - 1)]) } else { @() }
$pwsh = Join-Path $PSHOME $(if ($IsWindows) { 'pwsh.exe' } else { 'pwsh' })
& $pwsh -NoProfile -File (Join-Path $PSScriptRoot "$tool.ps1") @rest
exit $LASTEXITCODE
'@
    Set-Content -LiteralPath (Join-Path $script:ImplDir '_pwsh.ps1') -Value $dispatcher

    $cargo = @'
# A broken double must fail loudly, not print errors and carry on pretending to be a
# tool that worked.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_common.ps1')
$tool = 'cargo'
Record-Invocation -Tool $tool -Arguments @($args)
Assert-NotRequestedToFail -Tool $tool
Assert-ToolchainInherited -Tool $tool
if ($args.Count -gt 0 -and $args[0] -eq '--version') { 'cargo 1.98.0 (797e8a9bc 2026-08-05)'; exit 0 }
if ($args.Count -gt 0 -and $args[0] -eq 'clippy') {
    $version = if ($env:OHM_FAKE_CLIPPY_VERSION) { $env:OHM_FAKE_CLIPPY_VERSION } else { '0.1.98' }
    $hash = if ($env:OHM_FAKE_CLIPPY_HASH) { $env:OHM_FAKE_CLIPPY_HASH } else { '88d9e12ae1' }
    "clippy $version ($hash 2026-08-18)"
    exit 0
}
if ($args.Count -gt 0 -and $args[0] -eq 'metadata') {
    $manifest = Get-FlagValue -Arguments @($args) -Name '--manifest-path'
    if (-not $manifest) {
        # This is the defect the harness exists for: metadata asked without a bound
        # manifest answers about whatever directory the caller happens to be in.
        Write-Output 'FAKE cargo: metadata was called without --manifest-path, so the answer would be about the caller''s directory'
        exit 4
    }
    $target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path (Split-Path -Parent $manifest) 'target' }
    Write-Output 'warning: this is a test double, not cargo (proving the JSON is found inside noise)'
    [pscustomobject]@{
        packages          = @()
        workspace_members = @()
        target_directory  = $target
        version           = 1
        manifest_path     = $manifest
    } | ConvertTo-Json -Compress
    exit 0
}
# build / test: stand in for real output, in the target directory the script chose.
if ($env:CARGO_TARGET_DIR) {
    $release = Join-Path $env:CARGO_TARGET_DIR 'release'
    New-Item -ItemType Directory -Force -Path $release | Out-Null
    Set-Content -LiteralPath (Join-Path $release '.fake-cargo-output') -Value "cargo $($args -join ' ')"
} else {
    Write-Output 'FAKE cargo: CARGO_TARGET_DIR is not set, so this double cannot tell the script where its output went'
}
exit 0
'@
    Set-Content -LiteralPath (Join-Path $script:ImplDir 'cargo.ps1') -Value $cargo

    $rustc = @'
# A broken double must fail loudly, not print errors and carry on pretending to be a
# tool that worked.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_common.ps1')
$tool = 'rustc'
Record-Invocation -Tool $tool -Arguments @($args)
Assert-NotRequestedToFail -Tool $tool
Assert-ToolchainInherited -Tool $tool
$version = if ($env:OHM_FAKE_RUSTC_VERSION) { $env:OHM_FAKE_RUSTC_VERSION } else { '1.98.0' }
$hash = if ($env:OHM_FAKE_RUSTC_HASH) { $env:OHM_FAKE_RUSTC_HASH } else { '88d9e12ae' }
"rustc $version ($hash 2026-08-18)"
exit 0
'@
    Set-Content -LiteralPath (Join-Path $script:ImplDir 'rustc.ps1') -Value $rustc

    $node = @'
# A broken double must fail loudly, not print errors and carry on pretending to be a
# tool that worked.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_common.ps1')
Record-Invocation -Tool 'node' -Arguments @($args)
Assert-NotRequestedToFail -Tool 'node'
$version = if ($env:OHM_FAKE_NODE_VERSION) { $env:OHM_FAKE_NODE_VERSION } else { 'v22.12.0' }
$version
exit 0
'@
    Set-Content -LiteralPath (Join-Path $script:ImplDir 'node.ps1') -Value $node

    $npm = @'
# A broken double must fail loudly, not print errors and carry on pretending to be a
# tool that worked.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_common.ps1')
$tool = 'npm'
Record-Invocation -Tool $tool -Arguments @($args)
Assert-NotRequestedToFail -Tool $tool
if ($args.Count -gt 0 -and $args[0] -eq '--version') {
    $version = if ($env:OHM_FAKE_NPM_VERSION) { $env:OHM_FAKE_NPM_VERSION } else { '10.9.0' }
    $version
    exit 0
}
if ($args.Count -gt 0 -and $args[0] -eq 'ci') {
    # Real npm ci writes node_modules *here* — in the work directory's copy, which is
    # exactly what the harness checks: never inside the package.
    $modules = Join-Path (Get-Location).Path 'node_modules'
    New-Item -ItemType Directory -Force -Path $modules | Out-Null
    Set-Content -LiteralPath (Join-Path $modules '.fake-install') -Value 'npm ci'
    exit 0
}
if ($args.Count -gt 1 -and $args[0] -eq 'run' -and $args[1] -eq 'build') {
    $dist = Join-Path (Get-Location).Path 'dist'
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    Set-Content -LiteralPath (Join-Path $dist 'index.html') -Value '<!doctype html><title>fake frontend build</title>'
}
exit 0
'@
    Set-Content -LiteralPath (Join-Path $script:ImplDir 'npm.ps1') -Value $npm

    $npx = @'
# A broken double must fail loudly, not print errors and carry on pretending to be a
# tool that worked.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_common.ps1')
$tool = 'npx'
Record-Invocation -Tool $tool -Arguments @($args)
Assert-NotRequestedToFail -Tool $tool
Assert-ToolchainInherited -Tool $tool
if ($args.Count -gt 0 -and $args[0] -eq 'tauri') {
    $mode = if ($env:OHM_FAKE_TAURI) { $env:OHM_FAKE_TAURI } else { 'ok' }
    if ($mode -eq 'fail') { 'FAKE npx: the tauri build failed on purpose'; exit 3 }
    if ($mode -eq 'touchpackage') {
        if (-not $env:OHM_FAKE_PACKAGE) { 'FAKE npx: OHM_FAKE_PACKAGE is not set'; exit 5 }
        Set-Content -LiteralPath (Join-Path $env:OHM_FAKE_PACKAGE '.build-touched-the-package.tmp') -Value 'a build that wrote into the read-only package'
        exit 0
    }
    if ($mode -eq 'nobundle') { exit 0 }
    if (-not $env:CARGO_TARGET_DIR) {
        'FAKE npx: CARGO_TARGET_DIR is not set, so the script did not tell this build where its output belongs'
        exit 7
    }
    $confPath = Join-Path (Get-Location).Path 'src-tauri/tauri.conf.json'
    if (-not (Test-Path -LiteralPath $confPath)) {
        "FAKE npx: no tauri.conf.json at $confPath (the build must run in the app directory)"
        exit 6
    }
    $conf = Get-Content -LiteralPath $confPath -Raw | ConvertFrom-Json
    $arch = switch ($env:PROCESSOR_ARCHITECTURE) { 'AMD64' { 'x64' } 'ARM64' { 'arm64' } 'x86' { 'x86' } default { 'unknown' } }
    $version = if ($mode -eq 'wrongversion') { '0.2.0' } else { $conf.version }
    $name = "$($conf.productName)_${version}_$arch-setup.exe"
    $bundle = Join-Path (Join-Path $env:CARGO_TARGET_DIR 'release/bundle') 'nsis'
    New-Item -ItemType Directory -Force -Path $bundle | Out-Null
    if ($mode -eq 'empty') {
        Set-Content -LiteralPath (Join-Path $bundle $name) -Value '' -NoNewline
    } else {
        # A nonce, so two runs never produce identical bytes: a repeated run must produce
        # its own artefact rather than pass because the previous one is still there.
        Set-Content -LiteralPath (Join-Path $bundle $name) -Value ("fake NSIS installer, nonce {0}" -f [guid]::NewGuid().ToString('N'))
    }
    exit 0
}
exit 0
'@
    Set-Content -LiteralPath (Join-Path $script:ImplDir 'npx.ps1') -Value $npx

    # Decoys: a cargo and a rustc that must NEVER be reached. They exist so that a script
    # which fails to pin PATH to rustup's own directory fails visibly here instead of
    # silently building with the wrong toolchain.
    $decoy = @'
# A broken double must fail loudly, not print errors and carry on pretending to be a
# tool that worked.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_common.ps1')
$tool = Split-Path -Leaf $PSCommandPath
Record-Invocation -Tool "DECOY-$tool" -Arguments @($args)
"FAKE DECOY $tool: this cargo/rustc is not the rustup proxy: something resolved a Rust tool from PATH without the -Toolchain pin"
exit 9
'@
    Set-Content -LiteralPath (Join-Path $script:ImplDir 'decoy.ps1') -Value $decoy
}

Write-DoubleImplementations
foreach ($name in @('rustup', 'cargo', 'rustc')) { [void](New-DoubleWrapper -Directory $script:RustBinDir -Name $name -Implementation $name) }
foreach ($name in @('node', 'npm', 'npx')) { [void](New-DoubleWrapper -Directory $script:NodeBinDir -Name $name -Implementation $name) }
foreach ($name in @('cargo', 'rustc')) { [void](New-DoubleWrapper -Directory $script:DecoyDir -Name $name -Implementation 'decoy') }

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------
function Get-Sha256 {
    param([Parameter(Mandatory)][string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-RelativeFiles {
    param([Parameter(Mandatory)][string]$Root)
    $full = (Resolve-Path -LiteralPath $Root).Path.TrimEnd([char]'\', [char]'/')
    $prefix = $full + [System.IO.Path]::DirectorySeparatorChar
    return @(Get-ChildItem -LiteralPath $full -Recurse -File -Force | ForEach-Object {
        ($_.FullName.Substring($prefix.Length)) -replace '\\', '/'
    } | Sort-Object)
}

function Write-FakeManifest {
    param([Parameter(Mandatory)][string]$Root)
    $root = (Resolve-Path -LiteralPath $Root).Path.TrimEnd([char]'\', [char]'/')
    $lines = foreach ($file in (Get-ChildItem -LiteralPath $root -Recurse -File -Force |
                Where-Object { $_.Name -ne 'MANIFEST.sha256' } | Sort-Object FullName)) {
        $relative = ($file.FullName.Substring($root.Length + 1)) -replace '\\', '/'
        "{0}  {1}" -f (Get-Sha256 -Path $file.FullName), $relative
    }
    # LF endings and no trailing blank line: both verifiers (PowerShell and the shell
    # twin) read this file, and a CR would make the shell twin see a 65-character hash.
    Set-Content -LiteralPath (Join-Path $root 'MANIFEST.sha256') -Value (@($lines) -join "`n") -NoNewline
}

function New-FakePackage {
    <#
    .SYNOPSIS
        A miniature package: the real entry points, plus the few source files the
        scripts read (a workspace manifest, the Tauri config) and one source file the
        tamper cases can change.
    #>
    param([Parameter(Mandatory)][string]$Root)
    New-Item -ItemType Directory -Force -Path $Root | Out-Null
    foreach ($dir in @('scripts', 'apps/desktop/src-tauri', 'crates/ohm-core/src', 'docs/windows-validation')) {
        New-Item -ItemType Directory -Force -Path (Join-Path $Root $dir) | Out-Null
    }
    foreach ($file in @('_tools.ps1', 'precheck.ps1', 'build.ps1', 'verify-package.ps1', 'verify-package.sh')) {
        Copy-Item -LiteralPath (Join-Path $script:ScriptsDir $file) -Destination (Join-Path $Root "scripts/$file") -Force
    }
    Set-UnixExecutable -Path (Join-Path $Root 'scripts/verify-package.sh')
    Set-Content -LiteralPath (Join-Path $Root 'EVIDENCE.md') -Value "# Evidence index (test fixture)`n`nBuilt by run-script-tests.ps1; not a real package."
    Set-Content -LiteralPath (Join-Path $Root 'README-ACCEPTANCE.md') -Value '# Acceptance README (test fixture)'
    Set-Content -LiteralPath (Join-Path $Root 'Cargo.toml') -Value "[workspace]`nresolver = `"2`"`nmembers = []`n"
    Set-Content -LiteralPath (Join-Path $Root 'apps/desktop/package.json') -Value '{ "name": "openhardwareos-desktop", "version": "0.1.0", "private": true }'
    Set-Content -LiteralPath (Join-Path $Root 'apps/desktop/src-tauri/tauri.conf.json') -Value '{ "productName": "OpenHardwareOS", "version": "0.1.0", "bundle": { "active": true, "targets": ["nsis"] } }'
    Set-Content -LiteralPath (Join-Path $Root 'crates/ohm-core/src/lib.rs') -Value "// A source file, so the tamper cases have something to change.`npub fn version() -> &'static str { `"0.1.0`" }`n"
    Set-Content -LiteralPath (Join-Path $Root 'docs/windows-validation/collect.ps1') -Value '# placeholder, so the fixture has a docs tree'
    Write-FakeManifest -Root $Root
    return (Resolve-Path -LiteralPath $Root).Path
}

function Get-WorkDirFor {
    param([Parameter(Mandatory)][string]$PackageRoot)
    $full = (Resolve-Path -LiteralPath $PackageRoot).Path.TrimEnd([char]'\', [char]'/')
    return (Join-Path (Split-Path -Parent $full) ((Split-Path -Leaf $full) + '-build'))
}

function Get-ExpectedInstallerName {
    param([string]$Arch = 'x64')
    return "OpenHardwareOS_0.1.0_$Arch-setup.exe"
}

function New-CaseRoot {
    param([Parameter(Mandatory)][string]$Name)
    $dir = Join-Path $script:TempRoot "cases/$Name"
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    return $dir
}

# ---------------------------------------------------------------------------
# Assertions and the PASS/FAIL summary
# ---------------------------------------------------------------------------
$script:Cases = New-Object System.Collections.Generic.List[object]
$script:Current = $null
$script:CheckCount = 0
$script:FailureCount = 0

function Start-Case {
    param([string]$Name, [string]$Defect)
    $script:Current = [pscustomobject]@{
        Name    = $Name
        Defect  = $Defect
        Checks  = New-Object System.Collections.Generic.List[object]
    }
    Write-Host ''
    Write-Host ("--- {0}" -f $Name)
    Write-Host ("    catches: {0}" -f $Defect)
}

function Assert-That {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][bool]$Condition,
        [string]$Detail = ''
    )
    $script:CheckCount++
    if ($Condition) {
        Write-Host ("    [PASS] {0}" -f $Name)
    } else {
        $script:FailureCount++
        Write-Host ("    [FAIL] {0}" -f $Name) -ForegroundColor Red
        if ($Detail) { Write-Host ("           {0}" -f ($Detail -replace "`n", "`n           ")) -ForegroundColor Red }
    }
    if ($script:Current) {
        $script:Current.Checks.Add([pscustomobject]@{ Name = $Name; Ok = $Condition; Detail = $Detail })
    }
}

function End-Case {
    if (-not $script:Current) { return }
    $failed = @($script:Current.Checks | Where-Object { -not $_.Ok })
    $mark = if ($failed.Count -eq 0) { 'PASS' } else { 'FAIL' }
    Write-Host ("    => {0}: {1} ({2} check(s))" -f $mark, $script:Current.Name, $script:Current.Checks.Count)
    $script:Cases.Add($script:Current)
    $script:Current = $null
}

function Invoke-Case {
    param([string]$Name, [string]$Defect, [scriptblock]$Body)
    if ($Filter -and ($Name -notlike "*$Filter*")) { return }
    Start-Case -Name $Name -Defect $Defect
    try {
        & $Body
    } catch {
        Assert-That 'the case ran to the end without an exception' $false $_.Exception.Message
    }
    End-Case
}

function Get-Tail {
    param([string]$Text, [int]$Lines = 12)
    $all = @($Text -split "`n")
    if ($all.Count -le $Lines) { return $Text }
    return "(last $Lines of $($all.Count) lines)`n" + (($all | Select-Object -Last $Lines) -join "`n")
}

function Test-OutputHas {
    param([string]$Output, [string]$Pattern)
    return [bool]($Output -match [regex]::Escape($Pattern))
}

function Get-OutputLine {
    param([string]$Output, [string]$Pattern)
    return (@($Output -split "`n" | Where-Object { $_ -match [regex]::Escape($Pattern) }) | Select-Object -First 1)
}

# ---------------------------------------------------------------------------
# Environment control: every case starts from the same base, so nothing leaks
# ---------------------------------------------------------------------------
$script:EnvKeys = @(
    'PATH', 'HOME', 'USERPROFILE', 'CARGO_HOME', 'CARGO_TARGET_DIR', 'RUSTUP_TOOLCHAIN',
    'PROCESSOR_ARCHITECTURE', 'OS', 'OHM_CALL_LOG', 'OHM_FAKE_PACKAGE', 'OHM_FAKE_TAURI',
    'OHM_FAKE_FAIL', 'OHM_FAKE_TOOLCHAINS', 'OHM_FAKE_RUSTC_VERSION', 'OHM_FAKE_RUSTC_HASH',
    'OHM_FAKE_CLIPPY_VERSION', 'OHM_FAKE_CLIPPY_HASH', 'OHM_FAKE_NODE_VERSION',
    'OHM_FAKE_NPM_VERSION', 'OHM_REQUIRE_TOOLCHAIN'
)
$script:EnvOriginal = @{}
foreach ($key in $script:EnvKeys) {
    $script:EnvOriginal[$key] = [pscustomobject]@{
        Had   = [bool](Test-Path -LiteralPath "Env:$key")
        Value = [System.Environment]::GetEnvironmentVariable($key)
    }
}

function Set-CaseEnvironment {
    <#
    .SYNOPSIS
        A hermetic environment for one case: the developer's own PATH, HOME, CARGO_HOME
        and RUSTUP_TOOLCHAIN are replaced, not augmented, and only the doubles are
        reachable. PATH holds the doubles alone so that a missing double fails loudly
        instead of quietly running the real tool.
    #>
    param(
        [Parameter(Mandatory)][string]$PackageRoot,
        [Parameter(Mandatory)][string[]]$PathDirectories,
        [hashtable]$Extra = @{}
    )
    foreach ($key in $script:EnvKeys) {
        $original = $script:EnvOriginal[$key]
        if ($original.Had) { Set-Item -LiteralPath "Env:$key" -Value $original.Value }
        else { Remove-Item -LiteralPath "Env:$key" -ErrorAction SilentlyContinue }
    }
    if (-not $IsWindows) {
        # The host check reads $env:OS as a fallback for $IsWindows. On a non-Windows
        # host it must be absent, or the pre-check would be talked into claiming Windows.
        Remove-Item -LiteralPath 'Env:OS' -ErrorAction SilentlyContinue
    }
    $env:PATH = ($PathDirectories -join [System.IO.Path]::PathSeparator)
    $env:OHM_CALL_LOG = Join-Path $script:LogDir ("calls-{0}.log" -f [guid]::NewGuid().ToString('N').Substring(0, 6))
    $env:OHM_FAKE_PACKAGE = $PackageRoot
    $env:PROCESSOR_ARCHITECTURE = 'AMD64'
    foreach ($key in $Extra.Keys) {
        if ($null -eq $Extra[$key]) { Remove-Item -LiteralPath "Env:$key" -ErrorAction SilentlyContinue }
        else { Set-Item -LiteralPath "Env:$key" -Value $Extra[$key] }
    }
}

function Get-CallLog {
    if (-not $env:OHM_CALL_LOG -or -not (Test-Path -LiteralPath $env:OHM_CALL_LOG)) { return @() }
    return @(Get-Content -LiteralPath $env:OHM_CALL_LOG)
}

function Invoke-UnderTest {
    <#
    .SYNOPSIS
        Runs one entry point as its own process, from an explicit working directory.
    #>
    param(
        [Parameter(Mandatory)][string]$Script,
        [string[]]$Arguments = @(),
        [Parameter(Mandatory)][string]$PackageRoot,
        [Parameter(Mandatory)][string]$WorkingDirectory,
        [Parameter(Mandatory)][string]$Label
    )
    $path = Join-Path (Join-Path $PackageRoot 'scripts') $Script
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "no $Script under $PackageRoot\scripts" }
    if (-not (Test-Path -LiteralPath $WorkingDirectory -PathType Container)) {
        New-Item -ItemType Directory -Force -Path $WorkingDirectory | Out-Null
    }
    Push-Location -LiteralPath $WorkingDirectory
    try {
        $output = & $script:Pwsh -NoProfile -NonInteractive -File $path @Arguments 2>&1 | Out-String
        $code = $LASTEXITCODE
    } finally {
        Pop-Location
    }
    $text = ($output -replace "`r`n", "`n").Trim()
    Set-Content -LiteralPath (Join-Path $script:LogDir "$Label.log") -Value (
        @("$script:Pwsh -NoProfile -File $path $($Arguments -join ' ')",
          "cwd      : $WorkingDirectory",
          "exit code: $code",
          '') + @($text -split "`n"))
    return [pscustomobject]@{ ExitCode = $code; Output = $text; LogPath = (Join-Path $script:LogDir "$Label.log") }
}

function Get-BashPath {
    $candidates = @('/bin/bash', '/usr/bin/bash')
    if ($IsWindows) {
        $candidates = @()
        if ($env:ProgramFiles) { $candidates += (Join-Path $env:ProgramFiles 'Git/bin/bash.exe') }
        if ($env:LOCALAPPDATA) { $candidates += (Join-Path $env:LOCALAPPDATA 'Programs/Git/bin/bash.exe') }
    }
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    return $null
}

$shellTools = @($script:RustBinDir, $script:NodeBinDir)

# ===========================================================================
# Case 1 — a normal first run
# ===========================================================================
Invoke-Case -Name 'normal-first-run' -Defect 'a first run that does not produce, or does not record, the installer it claims to have built' -Body {
    $pkg = New-FakePackage -Root (Join-Path (New-CaseRoot 'normal') 'OpenHardwareOS-cafe001-windows-acceptance')
    $work = Get-WorkDirFor -PackageRoot $pkg
    $before = Get-RelativeFiles -Root $pkg
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles' ; CARGO_TARGET_DIR = $null ; RUSTUP_TOOLCHAIN = $null }

    $pre = Invoke-UnderTest -Script 'precheck.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case01-precheck'
    Assert-That 'precheck exits 0 on a healthy machine' ($pre.ExitCode -eq 0) "exit $($pre.ExitCode)`n$(Get-Tail $pre.Output)"
    Assert-That 'precheck reports PASSED' (Test-OutputHas $pre.Output 'PRE-CHECK PASSED') $pre.Output
    Assert-That 'precheck confirms clippy matches rustc (the check is not weakened)' (Test-OutputHas $pre.Output 'clippy matches rustc') $pre.Output
    Assert-That 'precheck resolves every tool to an absolute path' ($pre.Output -notmatch 'NOT FOUND') $pre.Output

    $build = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case01-build'
    Assert-That 'build exits 0' ($build.ExitCode -eq 0) "exit $($build.ExitCode)`n$(Get-Tail $build.Output)"
    Assert-That 'build prints BUILD COMPLETE' (Test-OutputHas $build.Output 'BUILD COMPLETE') $build.Output
    Assert-That 'build prints the absolute installer path' (Test-OutputHas $build.Output (Join-Path $work 'target')) $build.Output

    $installer = Join-Path $work ('target/release/bundle/nsis/' + (Get-ExpectedInstallerName))
    Assert-That 'the expected installer exists on disk' (Test-Path -LiteralPath $installer -PathType Leaf) $installer
    Assert-That 'the installer is non-empty' ((Get-Item -LiteralPath $installer).Length -gt 0)

    $record = Join-Path (Get-ChildItem -LiteralPath (Join-Path $work 'evidence') -Directory | Select-Object -First 1).FullName '91-installer.txt'
    Assert-That 'the run records the installer in 91-installer.txt' (Test-Path -LiteralPath $record -PathType Leaf) $record
    $recordText = Get-Content -LiteralPath $record -Raw
    $installerHash = Get-Sha256 -Path $installer
    Assert-That 'the recorded artefact hash matches the file on disk' (Test-OutputHas $recordText $installerHash) $recordText
    Assert-That 'the recorded artefact path is absolute' (Test-OutputHas $recordText $installer) $recordText
    Assert-That 'the recorded artefact size matches' (Test-OutputHas $recordText ("bytes       : $((Get-Item -LiteralPath $installer).Length)")) $recordText

    Assert-That 'the work directory holds the source copy' (Test-Path -LiteralPath (Join-Path $work 'source/Cargo.toml') -PathType Leaf)
    Assert-That 'the frontend install landed in the work copy, not the package' `
        ((Test-Path -LiteralPath (Join-Path $work 'source/apps/desktop/node_modules') -PathType Container) -and
         -not (Test-Path -LiteralPath (Join-Path $pkg 'apps/desktop/node_modules')))
    Assert-That 'cargo''s output landed in the work directory''s target dir' (Test-Path -LiteralPath (Join-Path $work 'target/release/.fake-cargo-output') -PathType Leaf)

    Assert-That 'the package is byte-for-byte unchanged after a full run' `
        (((Get-RelativeFiles -Root $pkg) -join '|') -eq ($before -join '|')) `
        ("before: $($before -join ', ')`nafter : $((Get-RelativeFiles -Root $pkg) -join ', ')")
    Assert-That 'no evidence\target\node_modules\dist was created inside the package' `
        (-not (Test-Path -LiteralPath (Join-Path $pkg 'evidence')) -and -not (Test-Path -LiteralPath (Join-Path $pkg 'target')) -and
         -not (Test-Path -LiteralPath (Join-Path $pkg 'node_modules')) -and -not (Test-Path -LiteralPath (Join-Path $pkg 'dist')))

    $verify = Invoke-UnderTest -Script 'verify-package.ps1' -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case01-verify-after'
    Assert-That 'the package still verifies against its manifest after the build' ($verify.ExitCode -eq 0) $verify.Output

    $evidenceRoot = Join-Path $work 'evidence'
    $runDir = @(Get-ChildItem -LiteralPath $evidenceRoot -Directory)[0].FullName
    Assert-That 'the run wrote an evidence folder with its environment' (Test-Path -LiteralPath (Join-Path $runDir '00-environment.txt') -PathType Leaf)
    Assert-That 'the run wrote its pre-check log' (Test-Path -LiteralPath (Join-Path $runDir '00-precheck.log') -PathType Leaf)
    Assert-That 'the run wrote a step log for the Tauri bundle' (@(Get-ChildItem -LiteralPath $runDir -File -Filter '*nsis*').Count -ge 1)
    Assert-That 'the run wrote a build summary' (Test-Path -LiteralPath (Join-Path $runDir 'build-summary.txt') -PathType Leaf)
}

# ===========================================================================
# Case 2 — a second run over the same package
# ===========================================================================
Invoke-Case -Name 'repeat-run' -Defect 'a second run blocked by the first run''s output, or a copy that is not reset' -Body {
    $pkg = New-FakePackage -Root (Join-Path (New-CaseRoot 'repeat') 'OpenHardwareOS-cafe002-windows-acceptance')
    $work = Get-WorkDirFor -PackageRoot $pkg
    $before = Get-RelativeFiles -Root $pkg
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles' }

    $first = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case02-build-1'
    Assert-That 'the first run exits 0' ($first.ExitCode -eq 0) "exit $($first.ExitCode)`n$(Get-Tail $first.Output)"
    $firstRunDir = @(Get-ChildItem -LiteralPath (Join-Path $work 'evidence') -Directory)[0].FullName
    $installer = Join-Path $work ('target/release/bundle/nsis/' + (Get-ExpectedInstallerName))
    $firstHash = Get-Sha256 -Path $installer

    # Leave something behind in the copy and in the target dir: the leftovers of run 1
    # must not be read as the source of run 2, and must not block it either.
    Set-Content -LiteralPath (Join-Path $work 'source/leftover-from-run-1.txt') -Value 'the first run left this in the copy'
    Start-Sleep -Milliseconds 1100

    $second = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case02-build-2'
    Assert-That 'the second run exits 0 (not blocked by the first run''s output)' ($second.ExitCode -eq 0) "exit $($second.ExitCode)`n$(Get-Tail $second.Output)"
    Assert-That 'the second run prints BUILD COMPLETE' (Test-OutputHas $second.Output 'BUILD COMPLETE') $second.Output
    Assert-That 'the second run noticed the installer already there before it started' (Test-OutputHas $second.Output 'before this run:') $second.Output
    Assert-That 'the second run produced its own artefact (hash differs from run 1)' ((Get-Sha256 -Path $installer) -ne $firstHash)
    Assert-That 'the copy was reset: run 1''s leftover is gone' (-not (Test-Path -LiteralPath (Join-Path $work 'source/leftover-from-run-1.txt')))
    Assert-That 'the copy holds exactly the package''s files' `
        (((Get-RelativeFiles -Root (Join-Path $work 'source')) -join '|') -eq ((Get-RelativeFiles -Root $pkg) -join '|'))
    Assert-That 'run 1''s evidence folder is still there' (Test-Path -LiteralPath (Join-Path $firstRunDir 'build-summary.txt') -PathType Leaf)
    Assert-That 'each run gets its own evidence folder' (@(Get-ChildItem -LiteralPath (Join-Path $work 'evidence') -Directory).Count -ge 2)
    Assert-That 'the package is still byte-for-byte unchanged' (((Get-RelativeFiles -Root $pkg) -join '|') -eq ($before -join '|'))
}

# ===========================================================================
# Case 3 — one byte changed in a package source file
# ===========================================================================
Invoke-Case -Name 'tampered-source-file' -Defect 'source tampering that is not reported file by file, or a build that starts anyway' -Body {
    $pkg = New-FakePackage -Root (Join-Path (New-CaseRoot 'tamper') 'OpenHardwareOS-cafe003-windows-acceptance')
    $work = Get-WorkDirFor -PackageRoot $pkg
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles' }

    $victim = Join-Path $pkg 'crates/ohm-core/src/lib.rs'
    Add-Content -LiteralPath $victim -Value '// one line appended: one byte-level change to a source file'

    $verify = Invoke-UnderTest -Script 'verify-package.ps1' -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case03-verify'
    Assert-That 'verification exits non-zero' ($verify.ExitCode -ne 0) $verify.Output
    Assert-That 'verification names the changed file' (Test-OutputHas $verify.Output 'changed: crates/ohm-core/src/lib.rs') $verify.Output

    $build = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case03-build'
    Assert-That 'the build refuses to start' ($build.ExitCode -ne 0) "exit $($build.ExitCode)"
    Assert-That 'the build names the changed file' (Test-OutputHas $build.Output 'crates/ohm-core/src/lib.rs') $build.Output
    Assert-That 'the build prints no BUILD COMPLETE' (-not (Test-OutputHas $build.Output 'BUILD COMPLETE')) $build.Output
    Assert-That 'nothing was copied into the work directory' (-not (Test-Path -LiteralPath (Join-Path $work 'source')))
}

# ===========================================================================
# Case 4 — an extra file in the package (including a name the old verifier excused)
# ===========================================================================
Invoke-Case -Name 'unknown-file-in-package' -Defect 'an unknown file that verification ignores — a blanket exemption, or a hidden file' -Body {
    $pkg = New-FakePackage -Root (Join-Path (New-CaseRoot 'unknown') 'OpenHardwareOS-cafe004-windows-acceptance')
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles' }

    New-Item -ItemType Directory -Force -Path (Join-Path $pkg 'windows-validation-TESTHOST-20260101-000000') | Out-Null
    Set-Content -LiteralPath (Join-Path $pkg 'windows-validation-TESTHOST-20260101-000000/environment.txt') -Value 'a collector bundle left inside the package'
    Set-Content -LiteralPath (Join-Path $pkg '.precheck-deadbeef.tmp') -Value 'the old write probe name, no longer excused'

    $verify = Invoke-UnderTest -Script 'verify-package.ps1' -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case04-verify'
    Assert-That 'verification exits non-zero' ($verify.ExitCode -ne 0) $verify.Output
    Assert-That 'verification names the collector bundle file' (Test-OutputHas $verify.Output 'not in the manifest: windows-validation-TESTHOST-20260101-000000/environment.txt') $verify.Output
    Assert-That 'verification names the .precheck-*.tmp file too (no exemption list)' (Test-OutputHas $verify.Output 'not in the manifest: .precheck-deadbeef.tmp') $verify.Output

    $bash = Get-BashPath
    if ($bash) {
        Push-Location -LiteralPath $pkg
        try {
            $shOutput = & $bash (Join-Path $pkg 'scripts/verify-package.sh') 2>&1 | Out-String
            $shCode = $LASTEXITCODE
        } finally { Pop-Location }
        Assert-That 'the shell twin exits non-zero' ($shCode -ne 0) $shOutput
        Assert-That 'the shell twin names the unknown file' (Test-OutputHas $shOutput 'not in the manifest: .precheck-deadbeef.tmp') $shOutput
    } else {
        Write-Host '    [SKIP] the shell twin was not run: no bash found on this host'
    }

    $build = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case04-build'
    Assert-That 'the build refuses to start on an unknown file' ($build.ExitCode -ne 0) "exit $($build.ExitCode)"
    Assert-That 'the build names the unknown file' (Test-OutputHas $build.Output '.precheck-deadbeef.tmp') $build.Output
}

# ===========================================================================
# Case 5 — a build that writes into the package
# ===========================================================================
Invoke-Case -Name 'build-writes-into-package' -Defect 'a build that writes into the read-only package without the run failing' -Body {
    $pkg = New-FakePackage -Root (Join-Path (New-CaseRoot 'writeback') 'OpenHardwareOS-cafe005-windows-acceptance')
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles'; OHM_FAKE_TAURI = 'touchpackage' }

    $build = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case05-build'
    Assert-That 'the build exits non-zero' ($build.ExitCode -ne 0) "exit $($build.ExitCode)"
    Assert-That 'the build reports the package as modified by this run' (Test-OutputHas $build.Output 'PACKAGE MODIFIED BY THIS RUN') $build.Output
    Assert-That 'the build names the file it put in the package' (Test-OutputHas $build.Output '.build-touched-the-package.tmp') $build.Output
    Assert-That 'the build prints no BUILD COMPLETE' (-not (Test-OutputHas $build.Output 'BUILD COMPLETE')) $build.Output

    $verify = Invoke-UnderTest -Script 'verify-package.ps1' -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case05-verify'
    Assert-That 'the package no longer verifies afterwards' ($verify.ExitCode -ne 0) $verify.Output
    Assert-That 'verification names the offending file' (Test-OutputHas $verify.Output 'not in the manifest: .build-touched-the-package.tmp') $verify.Output
}

# ===========================================================================
# Case 6 — paths with spaces and non-ASCII characters
# ===========================================================================
Invoke-Case -Name 'path-with-spaces-and-non-ascii' -Defect 'quoting: a package path with a space or a non-ASCII character' -Body {
    $roots = @(
        (Join-Path (New-CaseRoot 'paths') 'a folder with spaces/OpenHardwareOS-cafe006-windows-acceptance'),
        (Join-Path (New-CaseRoot 'paths') 'paket-日本語-ünïcode/OpenHardwareOS-cafe006-windows-acceptance')
    )
    foreach ($root in $roots) {
        $pkg = New-FakePackage -Root $root
        $work = Get-WorkDirFor -PackageRoot $pkg
        Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles' }
        $label = Split-Path -Leaf (Split-Path -Parent $pkg)
        $pre = Invoke-UnderTest -Script 'precheck.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label "case06-precheck-$label"
        Assert-That "precheck exits 0 for '$($pkg.Substring($script:TempRoot.Length))'" ($pre.ExitCode -eq 0) "exit $($pre.ExitCode)`n$(Get-Tail $pre.Output)"
        $build = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label "case06-build-$label"
        Assert-That "build exits 0 for '$($pkg.Substring($script:TempRoot.Length))'" ($build.ExitCode -eq 0) "exit $($build.ExitCode)`n$(Get-Tail $build.Output)"
        $installer = Join-Path $work ('target/release/bundle/nsis/' + (Get-ExpectedInstallerName))
        Assert-That "the installer is where the run says it is, under a path with spaces/non-ASCII" (Test-Path -LiteralPath $installer -PathType Leaf) $installer
    }
}

# ===========================================================================
# Case 7 — run from an unrelated current directory
# ===========================================================================
Invoke-Case -Name 'unrelated-current-directory' -Defect 'cargo metadata answering about the caller''s directory instead of the work copy' -Body {
    $root = New-CaseRoot 'elsewhere'
    $pkg = New-FakePackage -Root (Join-Path $root 'OpenHardwareOS-cafe007-windows-acceptance')
    $work = Get-WorkDirFor -PackageRoot $pkg
    $elsewhere = Join-Path $root 'some-other-project'
    New-Item -ItemType Directory -Force -Path $elsewhere | Out-Null
    Set-Content -LiteralPath (Join-Path $elsewhere 'Cargo.toml') -Value "[workspace]`nresolver = `"2`"`nmembers = []`n"
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles' }

    $pre = Invoke-UnderTest -Script 'precheck.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $elsewhere -Label 'case07-precheck'
    Assert-That 'precheck works from an unrelated directory' ($pre.ExitCode -eq 0) "exit $($pre.ExitCode)`n$(Get-Tail $pre.Output)"

    $build = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $elsewhere -Label 'case07-build'
    Assert-That 'build works from an unrelated directory' ($build.ExitCode -eq 0) "exit $($build.ExitCode)`n$(Get-Tail $build.Output)"
    Assert-That 'build prints BUILD COMPLETE' (Test-OutputHas $build.Output 'BUILD COMPLETE') $build.Output

    $runDir = @(Get-ChildItem -LiteralPath (Join-Path $work 'evidence') -Directory | Select-Object -Last 1).FullName
    $record = Get-Content -LiteralPath (Join-Path $runDir '91-installer.txt') -Raw
    Assert-That 'the installer is under the work directory, not the caller''s directory' `
        ((Test-OutputHas $record $work) -and -not (Test-OutputHas $record $elsewhere)) $record
    Assert-That 'nothing was written into the unrelated directory' `
        (-not (Test-Path -LiteralPath (Join-Path $elsewhere 'target')) -and -not (Test-Path -LiteralPath (Join-Path $elsewhere 'evidence')))
    Assert-That 'the metadata call was bound to the work copy''s manifest' `
        (@(Get-CallLog | Where-Object { $_ -like 'cargo|args=metadata*' -and $_ -like "*$work*source*Cargo.toml*" }).Count -ge 1) ((Get-CallLog) -join "`n")
}

# ===========================================================================
# Case 8 — rustup is not on PATH, but is in the fallback location
# ===========================================================================
Invoke-Case -Name 'toolchain-from-fallback-location' -Defect 'a resolver that only looks at PATH, or that falls back to a bare tool name' -Body {
    $root = New-CaseRoot 'fallback'
    $pkg = New-FakePackage -Root (Join-Path $root 'OpenHardwareOS-cafe008-windows-acceptance')
    $fakeHome = Join-Path $root 'fakehome'
    $fakeCargoBin = Join-Path $fakeHome '.cargo/bin'
    New-Item -ItemType Directory -Force -Path $fakeCargoBin | Out-Null
    foreach ($name in @('rustup', 'cargo', 'rustc')) { [void](New-DoubleWrapper -Directory $fakeCargoBin -Name $name -Implementation $name) }

    # Only the node tools are on PATH: the Rust tools exist solely in the fallback
    # directory, which is what a shell without `%USERPROFILE%\.cargo\bin` on PATH looks
    # like — the situation the resolver's fallback exists for.
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories @($script:NodeBinDir) `
        -Extra @{ HOME = $fakeHome; USERPROFILE = $fakeHome; CARGO_HOME = $null }

    $homeProbe = & $script:Pwsh -NoProfile -NonInteractive -Command '$HOME'
    $homeFollowsEnvironment = (($homeProbe | Out-String).Trim() -eq $fakeHome)
    Assert-That 'this host''s PowerShell takes $HOME from the environment (needed for the fallback test)' $homeFollowsEnvironment `
        "the child reported HOME='$((($homeProbe | Out-String).Trim()))', expected '$fakeHome'"

    $pre = Invoke-UnderTest -Script 'precheck.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case08-precheck'
    Assert-That 'precheck exits 0 with rustup only in the fallback directory' ($pre.ExitCode -eq 0) "exit $($pre.ExitCode)`n$(Get-Tail $pre.Output)"
    Assert-That 'precheck names the fallback path it resolved rustup from' (Test-OutputHas $pre.Output "fallback directory $fakeCargoBin") $pre.Output
    Assert-That 'no tool is reported as NOT FOUND' ($pre.Output -notmatch 'NOT FOUND') $pre.Output

    $build = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case08-build'
    Assert-That 'build exits 0 with the tools in the fallback directory' ($build.ExitCode -eq 0) "exit $($build.ExitCode)`n$(Get-Tail $build.Output)"

    # The same check with CARGO_HOME instead of HOME, for a machine that sets it.
    $cargoHome = Join-Path $root 'cargohome'
    $cargoHomeBin = Join-Path $cargoHome 'bin'
    New-Item -ItemType Directory -Force -Path $cargoHomeBin | Out-Null
    foreach ($name in @('rustup', 'cargo', 'rustc')) { [void](New-DoubleWrapper -Directory $cargoHomeBin -Name $name -Implementation $name) }
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories @($script:NodeBinDir) -Extra @{ CARGO_HOME = $cargoHome }
    $pre2 = Invoke-UnderTest -Script 'precheck.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case08-precheck-cargohome'
    Assert-That 'precheck exits 0 with the tools under CARGO_HOME' ($pre2.ExitCode -eq 0) "exit $($pre2.ExitCode)`n$(Get-Tail $pre2.Output)"
    Assert-That 'precheck names the CARGO_HOME fallback' (Test-OutputHas $pre2.Output "fallback directory $cargoHomeBin") $pre2.Output
}

# ===========================================================================
# Case 9 — -Toolchain must reach the Tauri build too
# ===========================================================================
Invoke-Case -Name 'toolchain-propagation' -Defect '-Toolchain reaching some Rust invocations but not the Tauri build' -Body {
    $root = New-CaseRoot 'toolchain'
    $pkg = New-FakePackage -Root (Join-Path $root 'OpenHardwareOS-cafe009-windows-acceptance')
    $cargoHome = Join-Path $root 'cargohome'
    $cargoBin = Join-Path $cargoHome 'bin'
    New-Item -ItemType Directory -Force -Path $cargoBin | Out-Null
    foreach ($name in @('rustup', 'cargo', 'rustc')) { [void](New-DoubleWrapper -Directory $cargoBin -Name $name -Implementation $name) }

    # The decoy comes first on PATH: it is not the rustup proxy, so anything that
    # resolves a Rust tool without the pin lands on it and exits 9.
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories @($script:DecoyDir, $script:NodeBinDir) `
        -Extra @{ CARGO_HOME = $cargoHome; OHM_REQUIRE_TOOLCHAIN = '1.98.0' }

    $pre = Invoke-UnderTest -Script 'precheck.ps1' -Arguments @('-AllowNonWindows', '-Toolchain', '1.98.0') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case09-precheck'
    Assert-That 'precheck exits 0 with -Toolchain' ($pre.ExitCode -eq 0) "exit $($pre.ExitCode)`n$(Get-Tail $pre.Output)"
    Assert-That 'precheck confirms the pin reaches child processes' (Test-OutputHas $pre.Output 'the cargo a child process finds is the rustup proxy') $pre.Output
    Assert-That 'precheck confirms the toolchain is installed' (Test-OutputHas $pre.Output "toolchain '1.98.0' is installed") $pre.Output
    Assert-That 'precheck never reached the decoy cargo' (@(Get-CallLog | Where-Object { $_ -like 'DECOY*' }).Count -eq 0) ((Get-CallLog) -join "`n")

    $build = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows', '-Toolchain', '1.98.0') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case09-build'
    Assert-That 'build exits 0 with -Toolchain' ($build.ExitCode -eq 0) "exit $($build.ExitCode)`n$(Get-Tail $build.Output)"
    Assert-That 'build prints BUILD COMPLETE' (Test-OutputHas $build.Output 'BUILD COMPLETE') $build.Output
    Assert-That 'build never reached the decoy cargo' (@(Get-CallLog | Where-Object { $_ -like 'DECOY*' }).Count -eq 0) ((Get-CallLog) -join "`n")

    $calls = Get-CallLog
    $npxCall = Get-OutputLine -Output ($calls -join "`n") 'npx|args=tauri build'
    Assert-That 'the Tauri build observed RUSTUP_TOOLCHAIN=1.98.0' ($npxCall -and $npxCall -like '*RUSTUP_TOOLCHAIN=1.98.0*') $npxCall
    Assert-That 'the Tauri build saw the rustup proxy cargo on its PATH' ($npxCall -and $npxCall -like "*cargo-on-PATH=$cargoBin*") $npxCall
    Assert-That 'the Rust steps ran through rustup run 1.98.0' (@($calls | Where-Object { $_ -like 'rustup-run-cargo|*' }).Count -ge 2) ($calls -join "`n")
    Assert-That 'every cargo invocation carried RUSTUP_TOOLCHAIN=1.98.0' `
        (@($calls | Where-Object { $_ -like 'cargo|*' -and $_ -notlike '*RUSTUP_TOOLCHAIN=1.98.0*' }).Count -eq 0) (($calls | Where-Object { $_ -like 'cargo|*' }) -join "`n")

    # The instrument checks itself: with no -Toolchain the doubles' guard must bite, so a
    # green result above cannot be an artefact of a guard that never fires.
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories @($cargoBin, $script:NodeBinDir, $script:DecoyDir) `
        -Extra @{ CARGO_HOME = $cargoHome; OHM_REQUIRE_TOOLCHAIN = '1.98.0' }
    $unpinned = Invoke-UnderTest -Script 'precheck.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case09-precheck-unpinned'
    Assert-That 'without -Toolchain the doubles'' toolchain guard fails the run (the guard is real)' `
        (($unpinned.ExitCode -ne 0) -and (Test-OutputHas $unpinned.Output 'RUSTUP_TOOLCHAIN is')) "exit $($unpinned.ExitCode)`n$(Get-Tail $unpinned.Output)"

    # And a toolchain rustup does not have must be reported, not built with.
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories @($script:DecoyDir, $script:NodeBinDir) `
        -Extra @{ CARGO_HOME = $cargoHome; OHM_FAKE_TOOLCHAINS = 'stable'; OHM_REQUIRE_TOOLCHAIN = $null }
    $missing = Invoke-UnderTest -Script 'precheck.ps1' -Arguments @('-AllowNonWindows', '-Toolchain', '1.98.0') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case09-precheck-absent'
    Assert-That 'a toolchain rustup does not have is reported' `
        (($missing.ExitCode -ne 0) -and (Test-OutputHas $missing.Output "toolchain '1.98.0' is installed")) "exit $($missing.ExitCode)`n$(Get-Tail $missing.Output)"
}

# ===========================================================================
# Case 10 — no installer at all
# ===========================================================================
Invoke-Case -Name 'missing-installer' -Defect '"BUILD COMPLETE" printed because a bundle directory merely exists' -Body {
    $pkg = New-FakePackage -Root (Join-Path (New-CaseRoot 'missing') 'OpenHardwareOS-cafe010-windows-acceptance')
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles'; OHM_FAKE_TAURI = 'nobundle' }
    $build = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case10-build'
    Assert-That 'the run fails' ($build.ExitCode -ne 0) "exit $($build.ExitCode)"
    Assert-That 'the run says INSTALLER MISSING' (Test-OutputHas $build.Output 'INSTALLER MISSING') $build.Output
    Assert-That 'the message names the installer it expected' (Test-OutputHas $build.Output (Get-ExpectedInstallerName)) $build.Output
    Assert-That 'no BUILD COMPLETE is printed' (-not (Test-OutputHas $build.Output 'BUILD COMPLETE')) $build.Output
}

# ===========================================================================
# Case 11 — a stale installer left over from an earlier build
# ===========================================================================
Invoke-Case -Name 'stale-installer' -Defect 'a leftover installer passing as the artefact of this run' -Body {
    $pkg = New-FakePackage -Root (Join-Path (New-CaseRoot 'stale') 'OpenHardwareOS-cafe011-windows-acceptance')
    $work = Get-WorkDirFor -PackageRoot $pkg
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles'; OHM_FAKE_TAURI = 'nobundle' }

    # A leftover from an earlier build: right name, right place, old timestamp.
    $stale = Join-Path $work ('target/release/bundle/nsis/' + (Get-ExpectedInstallerName))
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $stale) | Out-Null
    Set-Content -LiteralPath $stale -Value 'an installer from a previous build'
    (Get-Item -LiteralPath $stale).LastWriteTimeUtc = (Get-Date).ToUniversalTime().AddHours(-3)

    $build = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case11-build'
    Assert-That 'the run fails' ($build.ExitCode -ne 0) "exit $($build.ExitCode)"
    Assert-That 'the run says INSTALLER STALE' (Test-OutputHas $build.Output 'INSTALLER STALE') $build.Output
    Assert-That 'the message explains that a leftover is not this run''s artefact' (Test-OutputHas $build.Output 'leftover from an earlier build') $build.Output
    Assert-That 'no BUILD COMPLETE is printed' (-not (Test-OutputHas $build.Output 'BUILD COMPLETE')) $build.Output
}

# ===========================================================================
# Case 12 — an installer for another version, and an empty one
# ===========================================================================
Invoke-Case -Name 'wrong-or-empty-installer' -Defect 'an installer belonging to another version, or a zero-byte file, passing verification' -Body {
    $pkg = New-FakePackage -Root (Join-Path (New-CaseRoot 'wrong') 'OpenHardwareOS-cafe012-windows-acceptance')
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles'; OHM_FAKE_TAURI = 'wrongversion' }
    $build = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case12-build-wrongversion'
    Assert-That 'an installer for another version fails the run' ($build.ExitCode -ne 0) "exit $($build.ExitCode)"
    Assert-That 'the run says INSTALLER MISMATCH' (Test-OutputHas $build.Output 'INSTALLER MISMATCH') $build.Output
    Assert-That 'the message names the file it found instead' (Test-OutputHas $build.Output 'OpenHardwareOS_0.2.0_x64-setup.exe') $build.Output
    Assert-That 'no BUILD COMPLETE is printed' (-not (Test-OutputHas $build.Output 'BUILD COMPLETE')) $build.Output

    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles'; OHM_FAKE_TAURI = 'empty' }
    Remove-Item -LiteralPath (Get-WorkDirFor -PackageRoot $pkg) -Recurse -Force -ErrorAction SilentlyContinue
    $empty = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case12-build-empty'
    Assert-That 'a zero-byte installer fails the run' ($empty.ExitCode -ne 0) "exit $($empty.ExitCode)"
    Assert-That 'the run says INSTALLER EMPTY' (Test-OutputHas $empty.Output 'INSTALLER EMPTY') $empty.Output
    Assert-That 'no BUILD COMPLETE is printed for an empty installer' (-not (Test-OutputHas $empty.Output 'BUILD COMPLETE')) $empty.Output
}

# ===========================================================================
# Case 13 — the installer name follows the host's architecture
# ===========================================================================
Invoke-Case -Name 'installer-name-follows-host-architecture' -Defect 'a hard-coded installer name that does not follow tauri.conf.json and the host architecture' -Body {
    $pkg = New-FakePackage -Root (Join-Path (New-CaseRoot 'arch') 'OpenHardwareOS-cafe013-windows-acceptance')
    $work = Get-WorkDirFor -PackageRoot $pkg
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles'; PROCESSOR_ARCHITECTURE = 'ARM64' }
    $build = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case13-build'
    Assert-That 'the run exits 0 when the build produces the arm64 artefact' ($build.ExitCode -eq 0) "exit $($build.ExitCode)`n$(Get-Tail $build.Output)"
    $arm = Join-Path $work ('target/release/bundle/nsis/' + (Get-ExpectedInstallerName -Arch 'arm64'))
    Assert-That 'the expected arm64 installer exists' (Test-Path -LiteralPath $arm -PathType Leaf) $arm
    $record = @(Get-ChildItem -LiteralPath (Join-Path $work 'evidence') -Directory | Select-Object -Last 1).FullName
    Assert-That 'the run records the arm64 installer it expected' (Test-OutputHas (Get-Content -LiteralPath (Join-Path $record '91-installer.txt') -Raw) (Get-ExpectedInstallerName -Arch 'arm64'))
}

# ===========================================================================
# Case 14 — native commands that fail must stop the run
# ===========================================================================
Invoke-Case -Name 'failing-native-command-aborts' -Defect 'a native command''s non-zero exit being ignored ($ErrorActionPreference does not cover native commands)' -Body {
    $pkg = New-FakePackage -Root (Join-Path (New-CaseRoot 'nativefail') 'OpenHardwareOS-cafe014-windows-acceptance')

    # (a) a frontend step fails: the build must stop there, not carry on to Rust.
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles'; OHM_FAKE_FAIL = 'npm' }
    $build = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case14-build-npm'
    Assert-That 'a failing npm aborts the build' ($build.ExitCode -ne 0) "exit $($build.ExitCode)"
    Assert-That 'the run says RUN STOPPED' (Test-OutputHas $build.Output 'RUN STOPPED at step') $build.Output
    Assert-That 'the run stops at the frontend step' (Test-OutputHas $build.Output 'frontend install (npm ci)') $build.Output
    Assert-That 'the Rust steps never ran' (@(Get-CallLog | Where-Object { $_ -like 'rustup-run-cargo*' -or $_ -like 'cargo|args=build*' }).Count -eq 0) ((Get-CallLog) -join "`n")
    Assert-That 'no BUILD COMPLETE is printed' (-not (Test-OutputHas $build.Output 'BUILD COMPLETE')) $build.Output

    # (b) the bundle step fails: no installer, no success.
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles'; OHM_FAKE_FAIL = 'npx' }
    $npxFail = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case14-build-npx'
    Assert-That 'a failing tauri build aborts the run' ($npxFail.ExitCode -ne 0) "exit $($npxFail.ExitCode)"
    Assert-That 'the run stops at the bundle step' (Test-OutputHas $npxFail.Output 'RUN STOPPED at step 7') $npxFail.Output
    Assert-That 'no BUILD COMPLETE is printed when the bundle fails' (-not (Test-OutputHas $npxFail.Output 'BUILD COMPLETE')) $npxFail.Output

    # (c) the pre-check must not report a tool as available when it exits non-zero.
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles'; OHM_FAKE_FAIL = 'npm' }
    $pre = Invoke-UnderTest -Script 'precheck.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case14-precheck-npm'
    Assert-That 'a non-zero npm --version fails the pre-check' ($pre.ExitCode -ne 0) "exit $($pre.ExitCode)`n$(Get-Tail $pre.Output)"
    Assert-That 'the pre-check names npm as the failing tool' `
        ((Test-OutputHas $pre.Output '[FAIL] npm available') -or (Test-OutputHas $pre.Output 'npm available')) $pre.Output
    Assert-That 'the pre-check quotes the failing exit code' (Test-OutputHas $pre.Output 'exit code 3') $pre.Output

    # (d) a failing rustc, likewise.
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles'; OHM_FAKE_FAIL = 'rustc' }
    $preRustc = Invoke-UnderTest -Script 'precheck.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case14-precheck-rustc'
    Assert-That 'a non-zero rustc --version fails the pre-check' ($preRustc.ExitCode -ne 0) "exit $($preRustc.ExitCode)`n$(Get-Tail $preRustc.Output)"
    Assert-That 'the pre-check reports rustc as unavailable' (Test-OutputHas $preRustc.Output 'rustc available') $preRustc.Output
}

# ===========================================================================
# Case 15 — the pre-check gates the build
# ===========================================================================
Invoke-Case -Name 'precheck-gate-blocks-build' -Defect 'a build that starts although the pre-check failed' -Body {
    $pkg = New-FakePackage -Root (Join-Path (New-CaseRoot 'gate') 'OpenHardwareOS-cafe015-windows-acceptance')
    $work = Get-WorkDirFor -PackageRoot $pkg
    # Break the toolchain so the pre-check fails whatever the host is.
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles'; OHM_FAKE_FAIL = 'rustup,cargo,rustc' }
    $build = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case15-build'
    Assert-That 'the build exits non-zero' ($build.ExitCode -ne 0) "exit $($build.ExitCode)"
    Assert-That 'the build says it did not start' (Test-OutputHas $build.Output 'BUILD NOT STARTED') $build.Output
    Assert-That 'the build prints no BUILD COMPLETE' (-not (Test-OutputHas $build.Output 'BUILD COMPLETE')) $build.Output
    Assert-That 'no source was copied' (-not (Test-Path -LiteralPath (Join-Path $work 'source')))

    # On Windows the host check would pass, so this is the non-Windows half of the gate:
    # nothing but -AllowNonWindows may talk the pre-check into accepting this host.
    if (-not $IsWindows) {
        Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles' }
        $hostGate = Invoke-UnderTest -Script 'build.ps1' -Arguments @() -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case15-build-hostgate'
        Assert-That 'without -AllowNonWindows the build refuses on a non-Windows host' (($hostGate.ExitCode -ne 0) -and (Test-OutputHas $hostGate.Output 'host operating system')) "exit $($hostGate.ExitCode)`n$(Get-Tail $hostGate.Output)"
    } else {
        Write-Host '    [SKIP] the non-Windows half of the gate does not apply on Windows'
    }
}

# ===========================================================================
# Case 16 — the clippy/rustc pairing check is not weakened
# ===========================================================================
Invoke-Case -Name 'clippy-mismatch-still-fails' -Defect 'the clippy/rustc release check being weakened so a mismatched pair passes' -Body {
    $pkg = New-FakePackage -Root (Join-Path (New-CaseRoot 'clippy') 'OpenHardwareOS-cafe016-windows-acceptance')
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles'; OHM_FAKE_CLIPPY_HASH = 'deadbeef01' }
    $pre = Invoke-UnderTest -Script 'precheck.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case16-precheck'
    Assert-That 'a mismatched clippy/rustc pair fails the pre-check' ($pre.ExitCode -ne 0) "exit $($pre.ExitCode)`n$(Get-Tail $pre.Output)"
    Assert-That 'the pre-check reports the mismatch' (Test-OutputHas $pre.Output 'clippy matches rustc') $pre.Output
    Assert-That 'the message says the pair comes from different releases' (Test-OutputHas $pre.Output 'different releases') $pre.Output
}

# ===========================================================================
# Case 17 — the disk-space check and its drive qualifier
# ===========================================================================
Invoke-Case -Name 'disk-space-check-scope' -Defect 'the disk-space check reading /Us out of /Users/... as a drive, or running off Windows' -Body {
    $toolsPath = Join-Path $script:ScriptsDir '_tools.ps1'
    . $toolsPath
    Assert-That '_tools.ps1 defines Get-DriveQualifier (dot-sourced here as the scripts do)' ([bool](Get-Command -Name Get-DriveQualifier -ErrorAction SilentlyContinue))

    $qualifierCases = @(
        @{ Path = 'C:\Users\operator\OpenHardwareOS-build'; Expect = 'C:' },
        @{ Path = 'd:/build/ohm'; Expect = 'D:' },
        @{ Path = 'Z:\'; Expect = 'Z:' },
        @{ Path = '/Users/keychron/Documents/OpenHardwareOS'; Expect = $null },
        @{ Path = '/tmp/ohm-build'; Expect = $null },
        @{ Path = '\\server\share\ohm-build'; Expect = $null },
        @{ Path = 'relative\path'; Expect = $null },
        @{ Path = 'C:'; Expect = $null }
    )
    foreach ($case in $qualifierCases) {
        $actual = Get-DriveQualifier -Path $case.Path
        Assert-That ("qualifier of '{0}' is {1}" -f $case.Path, $(if ($case.Expect) { $case.Expect } else { '$null' })) `
            ($actual -eq $case.Expect) "got '$actual'"
    }

    $pkg = New-FakePackage -Root (Join-Path (New-CaseRoot 'disk') 'OpenHardwareOS-cafe017-windows-acceptance')
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles' }
    $pre = Invoke-UnderTest -Script 'precheck.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case17-precheck'
    if (-not $IsWindows) {
        Assert-That 'the free-space result line does not appear on a non-Windows host' (-not (Test-OutputHas $pre.Output 'free disk space >= 8 GB')) $pre.Output
        Assert-That 'the run says why the check was skipped' (Test-OutputHas $pre.Output 'free disk space not checked') $pre.Output
        Assert-That 'the note says the path has no drive qualifier' (Test-OutputHas $pre.Output 'no drive qualifier') $pre.Output

        # $env:OS is faked: the gate must be $IsWindows, not the environment variable, or
        # a non-Windows path would be parsed as a drive.
        Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles'; OS = 'Windows_NT' }
        $faked = Invoke-UnderTest -Script 'precheck.ps1' -Arguments @('-AllowNonWindows') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case17-precheck-fakedos'
        Assert-That 'with OS=Windows_NT faked, the free-space check still does not run here' (-not (Test-OutputHas $faked.Output 'free disk space >= 8 GB')) $faked.Output
        Assert-That 'and the note still says the host is not Windows' (Test-OutputHas $faked.Output 'free disk space not checked') $faked.Output
    } else {
        Write-Host '    [NOTE] this is Windows, so the skip-the-check half does not apply; the qualifier unit cases above are the portable part'
        Assert-That 'on Windows the check reports a result or says why it could not' ((Test-OutputHas $pre.Output 'free disk space >= 8 GB') -or (Test-OutputHas $pre.Output 'could not read free space')) $pre.Output
    }
}

# ===========================================================================
# Case 18 — -SkipBundle makes no claim about an installer
# ===========================================================================
Invoke-Case -Name 'skip-bundle' -Defect 'a run that skips the bundle yet still claims an installer' -Body {
    $pkg = New-FakePackage -Root (Join-Path (New-CaseRoot 'skipbundle') 'OpenHardwareOS-cafe018-windows-acceptance')
    $work = Get-WorkDirFor -PackageRoot $pkg
    Set-CaseEnvironment -PackageRoot $pkg -PathDirectories $shellTools -Extra @{ CARGO_HOME = $script:TempRoot + '/doubles'; OHM_FAKE_TAURI = 'nobundle' }
    $build = Invoke-UnderTest -Script 'build.ps1' -Arguments @('-AllowNonWindows', '-SkipBundle') -PackageRoot $pkg -WorkingDirectory $pkg -Label 'case18-build'
    Assert-That 'a skipped bundle still exits 0' ($build.ExitCode -eq 0) "exit $($build.ExitCode)`n$(Get-Tail $build.Output)"
    Assert-That 'the run says the bundle was skipped' (Test-OutputHas $build.Output 'skipped by -SkipBundle') $build.Output
    $runDir = @(Get-ChildItem -LiteralPath (Join-Path $work 'evidence') -Directory | Select-Object -Last 1).FullName
    Assert-That 'no installer record is written' (-not (Test-Path -LiteralPath (Join-Path $runDir '91-installer.txt')))
    Assert-That 'the summary records that no claim is made about an installer' (Test-OutputHas (Get-Content -LiteralPath (Join-Path $runDir 'build-summary.txt') -Raw) 'makes no claim about an installer')
}

# ===========================================================================
# Summary
# ===========================================================================
Write-Host ''
Write-Host '================================================================================'
Write-Host ' SCRIPT TEST SUMMARY'
Write-Host '================================================================================'
$caseFailures = 0
foreach ($case in $script:Cases) {
    $failed = @($case.Checks | Where-Object { -not $_.Ok })
    if ($failed.Count -eq 0) {
        Write-Host ("  PASS  {0,-42} {1} check(s)" -f $case.Name, $case.Checks.Count)
    } else {
        $caseFailures++
        Write-Host ("  FAIL  {0,-42} {1} of {2} check(s) failed" -f $case.Name, $failed.Count, $case.Checks.Count) -ForegroundColor Red
        foreach ($check in $failed) {
            Write-Host ("          - {0}" -f $check.Name) -ForegroundColor Red
            if ($check.Detail) { Write-Host ("              {0}" -f ($check.Detail -replace "`n", ' | ')) -ForegroundColor Red }
        }
    }
}
Write-Host ''
Write-Host ("cases: {0}   checks: {1}   failures: {2}" -f $script:Cases.Count, $script:CheckCount, $script:FailureCount)
Write-Host ''
Write-Host ' These tests exercised script control flow with test doubles. They are NOT'
Write-Host ' evidence that a Windows build succeeds: that needs a real Windows machine.'
Write-Host ''

if ($script:FailureCount -gt 0 -or $script:Cases.Count -eq 0) {
    Write-Host 'RESULT: FAIL' -ForegroundColor Red
    Write-Host ("Captured output kept in: {0}" -f $script:LogDir)
    Write-Host ("Temporary directory kept: {0}" -f $script:TempRoot)
    exit 1
}

Write-Host 'RESULT: PASS' -ForegroundColor Green
if ($KeepTemp) {
    Write-Host ("Temporary directory kept (-KeepTemp): {0}" -f $script:TempRoot)
} else {
    Remove-Item -LiteralPath $script:TempRoot -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host 'Temporary directory removed (nothing was written outside it).'
}
exit 0
