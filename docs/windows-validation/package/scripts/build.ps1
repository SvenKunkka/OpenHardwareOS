<#
.SYNOPSIS
    Builds and tests OpenHardwareOS from this package, capturing evidence.

.DESCRIPTION
    Runs the pre-check first and refuses to continue if it fails. Then, in order:
    frontend install, type check, frontend tests, frontend production build, the Rust
    workspace build (all targets), the Rust test suite, and the NSIS installer.

    Every step writes its full output to .\evidence\ and stops the run at the first
    non-zero exit — a build that "mostly worked" produces evidence nobody can trust,
    and continuing past a failure buries the first, most useful error.

    It never launches the application, never writes a fan value, and installs
    nothing. Installing the bundle is a separate, deliberate, administrator step:
    see docs\windows-validation\checklist.md §7.2.

.PARAMETER Toolchain
    rustup toolchain to build with, passed through to the pre-check, e.g. `1.98.0`.
    Use the same value for both so the checked compiler is the building compiler.

.PARAMETER SkipBundle
    Stop after the test suite. Useful when the point of the run is the Rust evidence
    rather than the installer.

.EXAMPLE
    PS> .\scripts\build.ps1
    PS> .\scripts\build.ps1 -Toolchain 1.98.0
    PS> .\scripts\build.ps1 -SkipBundle
#>
#Requires -Version 7.0
[CmdletBinding()]
param(
    [string]$Toolchain = '',
    [switch]$SkipBundle
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$evidence = Join-Path $root 'evidence'
New-Item -ItemType Directory -Force -Path $evidence | Out-Null

$script:StepIndex = 0
$script:Summary = New-Object System.Collections.Generic.List[string]

Write-Host ''
Write-Host 'OpenHardwareOS — Windows acceptance build'
Write-Host ("package : {0}" -f $root)
Write-Host ("evidence: {0}" -f $evidence)
Write-Host ("started : {0}" -f (Get-Date -Format 'u'))
Write-Host ''

# --- Pre-check (a failed run must not start) ----------------------------------
$precheck = Join-Path $PSScriptRoot 'precheck.ps1'
$precheckArgs = @{}
if ($Toolchain) { $precheckArgs['Toolchain'] = $Toolchain }
& $precheck @precheckArgs
if ($LASTEXITCODE -ne 0) {
    Write-Host 'Build not started: the pre-check failed. See its output above.' -ForegroundColor Red
    exit 1
}

function Invoke-Step {
    param(
        [string]$Name,
        [string]$Command,
        [string[]]$Arguments,
        [string]$WorkingDirectory = $root
    )
    $script:StepIndex++
    $log = Join-Path $evidence ("{0:d2}-{1}.log" -f $script:StepIndex, ($Name -replace '[^A-Za-z0-9]+', '-').ToLower())
    Write-Host ("[{0}] {1}" -f $script:StepIndex, $Name)
    Write-Host ("     {0} {1}" -f $Command, ($Arguments -join ' '))
    $started = Get-Date
    Push-Location $WorkingDirectory
    try {
        & $Command @Arguments *> $log
        $code = $LASTEXITCODE
    } finally {
        Pop-Location
    }
    $elapsed = [math]::Round(((Get-Date) - $started).TotalSeconds, 1)
    Add-Content -LiteralPath $log -Value ("`n[exit code: {0}] [seconds: {1}]" -f $code, $elapsed)

    if ($code -ne 0) {
        Write-Host ("     FAILED (exit {0}) after {1}s — last lines of {2}:" -f $code, $elapsed, (Split-Path -Leaf $log)) -ForegroundColor Red
        Get-Content -LiteralPath $log -Tail 25 | ForEach-Object { Write-Host "     $_" }
        $script:Summary.Add("FAIL {0} (exit {1}) -> {2}" -f $Name, $code, (Split-Path -Leaf $log))
        Write-Host ''
        Write-Host ("RUN STOPPED at step {0} ({1}). Full output: {2}" -f $script:StepIndex, $Name, $log) -ForegroundColor Red
        Write-Host 'Capture that log in the evidence bundle and report it. Do not re-run the remaining steps.' -ForegroundColor Red
        Write-Summary
        exit 1
    }
    Write-Host ("     ok ({0}s)" -f $elapsed) -ForegroundColor Green
    $script:Summary.Add("PASS {0} -> {1}" -f $Name, (Split-Path -Leaf $log))
}

function Write-Summary {
    $file = Join-Path $evidence 'build-summary.txt'
    $lines = @(
        "OpenHardwareOS build summary",
        "package    : $root",
        "finished   : $(Get-Date -Format 'u')",
        "toolchain  : $(if ($Toolchain) { $Toolchain } else { 'PATH rustc' })",
        ""
    ) + $script:Summary
    Set-Content -LiteralPath $file -Value $lines
    Write-Host ("summary written to {0}" -f $file)
}

$desktop = Join-Path $root 'apps/desktop'

# --- Frontend -----------------------------------------------------------------
Invoke-Step 'frontend install (npm ci)' 'npm' @('ci', '--no-audit', '--no-fund') $desktop
Invoke-Step 'frontend type check' 'npm' @('run', 'typecheck') $desktop
Invoke-Step 'frontend tests' 'npm' @('run', 'test') $desktop
Invoke-Step 'frontend build' 'npm' @('run', 'build') $desktop

# --- Rust ---------------------------------------------------------------------
$cargoArgs = if ($Toolchain) { @('run', $Toolchain, 'cargo') } else { @('cargo') }
$cargo = 'rustup'
if (-not $Toolchain) { $cargo = 'cargo' }
if ($Toolchain) {
    Invoke-Step 'rust workspace build (all targets)' $cargo @('run', $Toolchain, 'cargo', 'build', '--workspace', '--all-targets')
    Invoke-Step 'rust test suite' $cargo @('run', $Toolchain, 'cargo', 'test', '--workspace')
} else {
    Invoke-Step 'rust workspace build (all targets)' 'cargo' @('build', '--workspace', '--all-targets')
    Invoke-Step 'rust test suite' 'cargo' @('test', '--workspace')
}

# --- Installer ----------------------------------------------------------------
if ($SkipBundle) {
    Write-Host '[bundle] skipped by -SkipBundle'
    $script:Summary.Add('SKIP installer bundle (-SkipBundle)')
} else {
    Invoke-Step 'NSIS installer bundle (tauri build)' 'npx' @('tauri', 'build') $desktop
    # The bundle lands in the *workspace* target directory, which is why it is asked
    # for rather than assumed.
    $target = if ($Toolchain) {
        (& rustup run $Toolchain cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
    } else {
        (& cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
    }
    $bundleRoot = Join-Path $target 'release/bundle'
    if (Test-Path -LiteralPath $bundleRoot) {
        Write-Host ''
        Write-Host ("installer artefacts under {0}:" -f $bundleRoot)
        Get-ChildItem -LiteralPath $bundleRoot -Recurse -File | ForEach-Object {
            $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            Write-Host ("  {0}  {1} bytes  {2}" -f $hash, $_.Length, $_.FullName)
            $script:Summary.Add("ARTEFACT {0} sha256={1} bytes={2}" -f $_.Name, $hash, $_.Length)
        }
    } else {
        Write-Host ("no bundle directory at {0} — the build reported success without producing an installer" -f $bundleRoot) -ForegroundColor Red
        $script:Summary.Add("FAIL bundle directory missing at $bundleRoot")
        Write-Summary
        exit 1
    }
}

Write-Host ''
Write-Host 'BUILD COMPLETE' -ForegroundColor Green
Write-Summary
Write-Host ''
Write-Host 'Next: docs\windows-validation\checklist.md §7.2 (install, needs administrator),'
Write-Host 'then the rest of the checklist, and fill in result-template.md.'
exit 0
