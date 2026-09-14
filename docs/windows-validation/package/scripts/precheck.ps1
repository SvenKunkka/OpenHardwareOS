<#
.SYNOPSIS
    Refuses to let a Windows acceptance run start on a machine that cannot produce
    trustworthy evidence.

.DESCRIPTION
    Every check here answers one question: would a failure later in this run be a
    finding about OpenHardwareOS, or about the machine? The second kind wastes a
    session and, worse, gets reported as the first. So this script checks the ground
    first and stops on the first problem.

    It writes nothing except a temporary file used to prove the package directory is
    writable (created and deleted immediately). It does not build, does not install
    anything and does not touch hardware.

.PARAMETER AllowNonWindows
    Continue on a non-Windows host. The result is then explicitly NOT Windows
    acceptance evidence — it is a way for a contributor to sanity-check a checkout.
    Any output from such a run must not be presented as a Windows result.

.PARAMETER Toolchain
    A rustup toolchain name to check and build with, e.g. `1.98.0`. When given, every
    Rust check runs through `rustup run <name>`, so the compiler that is checked is
    exactly the compiler the build will use. Without it, whatever `rustc` is on PATH
    is checked — and on a machine with more than one Rust installed that is a real
    source of wrong conclusions, because a `clippy` from a different release than
    `rustc` cannot lint this workspace at all.

.EXAMPLE
    PS> .\scripts\precheck.ps1
    PS> .\scripts\precheck.ps1 -Toolchain 1.98.0
#>
#Requires -Version 7.0
[CmdletBinding()]
param(
    [switch]$AllowNonWindows,
    [string]$Toolchain = ''
)

$ErrorActionPreference = 'Stop'
$script:PackageRoot = Split-Path -Parent $PSScriptRoot
$script:Failures = New-Object System.Collections.Generic.List[string]
$script:Notes = New-Object System.Collections.Generic.List[string]

function Add-Result {
    param([string]$Name, [bool]$Ok, [string]$Detail)
    $mark = if ($Ok) { 'PASS' } else { 'FAIL' }
    Write-Host ("  [{0}] {1,-28} {2}" -f $mark, $Name, $Detail)
    if (-not $Ok) { $script:Failures.Add("$Name`: $Detail") }
}

function Add-Note {
    param([string]$Text)
    $script:Notes.Add($Text)
}

# Resolve a Rust tool, preferring PATH and falling back to rustup's own directory.
# `rustup` installs itself into `%USERPROFILE%\.cargo\bin`, which is on PATH in a
# normal install but not in every session — and "rustup is not recognized" is a
# confusing way to tell an operator that their Rust is fine but their PATH is not.
function Resolve-Tool {
    param([string]$Name)
    $found = Get-Command $Name -ErrorAction SilentlyContinue
    if ($found) { return $found.Source }
    $suffix = if ($IsWindows) { '.exe' } else { '' }
    $fallback = Join-Path $HOME ".cargo/bin/$Name$suffix"
    if (Test-Path -LiteralPath $fallback) { return $fallback }
    return $null
}

function Invoke-RustTool {
    param([string]$Tool, [string[]]$Arguments)
    if ($Toolchain) {
        $rustup = Resolve-Tool 'rustup'
        if (-not $rustup) {
            throw "the toolchain '$Toolchain' was requested, but rustup is on neither PATH nor $HOME/.cargo/bin. Install rustup (https://rustup.rs), or drop -Toolchain."
        }
        return (& $rustup run $Toolchain $Tool @Arguments 2>&1 | Out-String).Trim()
    }
    $resolved = Resolve-Tool $Tool
    if (-not $resolved) {
        throw "$Tool is on neither PATH nor $HOME/.cargo/bin."
    }
    return (& $resolved @Arguments 2>&1 | Out-String).Trim()
}

function Get-VersionNumbers {
    param([string]$Text)
    if ($Text -match '(\d+)\.(\d+)(?:\.(\d+))?') {
        return @([int]$Matches[1], [int]$Matches[2], [int]($(if ($Matches[3]) { $Matches[3] } else { 0 })))
    }
    return $null
}

Write-Host ''
Write-Host 'OpenHardwareOS — Windows acceptance pre-check'
Write-Host ("package: {0}" -f $script:PackageRoot)
Write-Host ''

# --- 1. Host ------------------------------------------------------------------
if ($IsWindows -or $env:OS -eq 'Windows_NT') {
    Add-Result 'host operating system' $true ("Windows {0} ({1})" -f [System.Environment]::OSVersion.Version, $env:PROCESSOR_ARCHITECTURE)
} elseif ($AllowNonWindows) {
    Add-Result 'host operating system' $true ("{0} — allowed by -AllowNonWindows, so this run is NOT Windows evidence" -f [System.Runtime.InteropServices.RuntimeInformation]::OSDescription)
} else {
    Add-Result 'host operating system' $false 'this is not Windows. Windows-specific paths (WMI, \\.\pipe, NVML, NSIS) cannot be exercised here. Re-run on Windows, or use -AllowNonWindows only to sanity-check a checkout.'
}

# --- 2. The package itself ----------------------------------------------------
$manifest = Join-Path $script:PackageRoot 'MANIFEST.sha256'
if (Test-Path -LiteralPath $manifest) {
    $verify = Join-Path $PSScriptRoot 'verify-package.ps1'
    if (Test-Path -LiteralPath $verify) {
        $output = & $verify -Quiet 2>&1 | Out-String
        $ok = $LASTEXITCODE -eq 0
        $detail = if ($ok) { 'every file matches MANIFEST.sha256' } else { "contents differ from the manifest: $($output.Trim())" }
        Add-Result 'package integrity' $ok $detail
    } else {
        Add-Result 'package integrity' $false 'scripts\verify-package.ps1 is missing, so the manifest cannot be checked'
    }
} else {
    Add-Result 'package integrity' $false 'MANIFEST.sha256 is missing — this is not an assembled package'
}

# --- 3. Write access ----------------------------------------------------------
$probe = Join-Path $script:PackageRoot ('.precheck-{0}.tmp' -f [guid]::NewGuid().ToString('N').Substring(0, 8))
try {
    Set-Content -LiteralPath $probe -Value 'probe' -NoNewline
    Remove-Item -LiteralPath $probe -Force
    Add-Result 'package directory writable' $true 'evidence output can be written next to the source'
} catch {
    Add-Result 'package directory writable' $false "cannot write to $($script:PackageRoot): $($_.Exception.Message)"
}

# --- 4. Rust toolchain --------------------------------------------------------
$rustcVersion = $null
try {
    $rustcVersion = Invoke-RustTool 'rustc' @('--version')
    $numbers = Get-VersionNumbers $rustcVersion
    # The workspace declares rust-version = "1.95"; a compiler below that cannot
    # build it, and the failure would look like a project defect.
    if ($null -eq $numbers) {
        Add-Result 'rustc version parseable' $false "could not read a version from: $rustcVersion"
    } elseif ($numbers[0] -gt 1 -or ($numbers[0] -eq 1 -and $numbers[1] -ge 95)) {
        Add-Result 'rustc >= 1.95 (workspace MSRV)' $true $rustcVersion
    } else {
        Add-Result 'rustc >= 1.95 (workspace MSRV)' $false "$rustcVersion is older than the workspace rust-version = 1.95"
    }
} catch {
    Add-Result 'rustc available' $false $_.Exception.Message
}

$cargoVersion = $null
try {
    $cargoVersion = Invoke-RustTool 'cargo' @('--version')
    Add-Result 'cargo available' $true $cargoVersion
} catch {
    Add-Result 'cargo available' $false $_.Exception.Message
}

# clippy must come from the same release as rustc. A mismatched pair does not lint,
# and it says so unclearly: clippy-driver 0.1.92 beside rustc 1.98.0 makes cargo
# apply the rust-version gate and abort before reading a line of code.
try {
    $clippyVersion = Invoke-RustTool 'cargo' @('clippy', '--version')
    # The two tools abbreviate the same commit differently — `rustc 1.98.0 (88d9e12ae)`
    # against `clippy 0.1.98 (88d9e12ae1)` — so compare the common prefix rather than
    # the whole string, or a correctly matched pair is reported as mismatched.
    $rustcHash = if ($rustcVersion -match '\(([0-9a-f]+)[ )]') { $Matches[1] } else { '' }
    $clippyHash = if ($clippyVersion -match '\(([0-9a-f]+)[ )]') { $Matches[1] } else { '' }
    $common = [Math]::Min($rustcHash.Length, $clippyHash.Length)
    $sameRelease = $rustcHash -and $clippyHash -and $common -ge 9 -and
                   ($rustcHash.Substring(0, $common) -eq $clippyHash.Substring(0, $common))
    if ($sameRelease) {
        Add-Result 'clippy matches rustc' $true $clippyVersion
    } else {
        Add-Result 'clippy matches rustc' $false "$rustcVersion and $clippyVersion come from different releases; install a matching pair (rustup component add clippy) before linting"
    }
} catch {
    Add-Note 'clippy is not installed: the build still works, but `cargo clippy` cannot be run as evidence.'
}

# --- 5. Node and npm ----------------------------------------------------------
try {
    $node = Resolve-Tool 'node'
    if (-not $node) { throw 'node is on neither PATH nor in the usual place.' }
    $nodeVersion = (& $node --version 2>&1 | Out-String).Trim()
    $numbers = Get-VersionNumbers $nodeVersion
    # apps/desktop/package.json: ^20.19.0 || >=22.12.0
    $ok = $false
    if ($numbers) {
        $ok = ($numbers[0] -eq 20 -and $numbers[1] -ge 19) -or ($numbers[0] -gt 22) -or
              ($numbers[0] -eq 22 -and ($numbers[1] -gt 12 -or ($numbers[1] -eq 12 -and $numbers[2] -ge 0)))
    }
    Add-Result 'node satisfies package.json' $ok $nodeVersion
} catch {
    Add-Result 'node available' $false $_.Exception.Message
}
try {
    $npm = Resolve-Tool 'npm'
    if (-not $npm) { throw 'npm is on neither PATH nor in the usual place.' }
    $npmVersion = (& $npm --version 2>&1 | Out-String).Trim()
    Add-Result 'npm available' $true $npmVersion
} catch {
    Add-Result 'npm available' $false $_.Exception.Message
}

# --- 6. Disk space ------------------------------------------------------------
# A full workspace build, npm's cache and the NSIS bundle need room. This is a check
# about the machine the build happens on, so it is stated where it means something.
if ($IsWindows -or $env:OS -eq 'Windows_NT') {
    try {
        $qualifier = (Resolve-Path -LiteralPath $script:PackageRoot).Path.Substring(0, 3)
        $freeGb = [math]::Round((Get-PSDrive -Name $qualifier.TrimEnd(':')).Free / 1GB, 1)
        Add-Result 'free disk space >= 8 GB' ($freeGb -ge 8) ("$freeGb GB free on $qualifier")
    } catch {
        Add-Note "could not read free space: $($_.Exception.Message)"
    }
} else {
    Add-Note 'free disk space not checked: this is not Windows, and the build happens on Windows'
}

# --- Report -------------------------------------------------------------------
Write-Host ''
if ($script:Notes.Count -gt 0) {
    Write-Host 'Notes (not failures):'
    foreach ($note in $script:Notes) { Write-Host "  - $note" }
    Write-Host ''
}

if ($script:Failures.Count -gt 0) {
    Write-Host ("PRE-CHECK FAILED — {0} problem(s). Do not continue: a run started now would report machine problems as project findings." -f $script:Failures.Count)
    foreach ($failure in $script:Failures) { Write-Host "  * $failure" }
    Write-Host ''
    Write-Host 'Fix the machine, then run this script again. Do not edit the scripts to get past this.'
    exit 1
}

Write-Host 'PRE-CHECK PASSED — the machine can produce evidence that means something.'
if (-not ($IsWindows -or $env:OS -eq 'Windows_NT')) {
    Write-Host 'REMINDER: this host is not Windows, so nothing produced from here is Windows acceptance evidence.'
}
Write-Host ''
Write-Host 'Next:'
Write-Host '  1. .\scripts\verify-package.ps1'
Write-Host '  2. .\docs\windows-validation\collect.ps1 -DryRun     (read-only; then run it for real)'
Write-Host '  3. .\scripts\build.ps1'
exit 0
