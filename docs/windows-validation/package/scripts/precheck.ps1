<#
.SYNOPSIS
    Refuses to let a Windows acceptance run start on a machine that cannot produce
    trustworthy evidence.

.DESCRIPTION
    Every check here answers one question: would a failure later in this run be a
    finding about OpenHardwareOS, or about the machine? The second kind wastes a
    session and, worse, gets reported as the first. So this script checks the ground
    first and stops on the first problem.

    It writes nothing inside the package. The build happens in a *work directory*
    outside it (default: a sibling of the package called `<package>-build`, overridable
    with -WorkDir) — so the package stays exactly the tree the manifest describes, and
    a second run is not blocked by the first run's output. This check creates that work
    directory if it is missing, writes one probe file inside it to prove the build's
    output location is writable, deletes the probe file, and removes the directory
    again if this run created it. A probe file inside the package would itself be a file
    the manifest does not describe if the run were interrupted — which is exactly what
    the package verifier treats as a failure.

    It does not build, does not install anything and does not touch hardware.

.PARAMETER AllowNonWindows
    Continue on a non-Windows host. The result is then explicitly NOT Windows
    acceptance evidence — it is a way for a contributor to sanity-check a checkout.
    Any output from such a run must not be presented as a Windows result.

.PARAMETER Toolchain
    A rustup toolchain name to check and build with, e.g. `1.98.0`. When given, every
    Rust check runs through `rustup run <name>`, and RUSTUP_TOOLCHAIN is set in this
    process so that a child process this script does not launch itself — the cargo
    inside `npx tauri build` — uses the same toolchain. Without it, whatever `rustc` is
    on PATH is checked — and on a machine with more than one Rust installed that is a
    real source of wrong conclusions, because a `clippy` from a different release than
    `rustc` cannot lint this workspace at all.

    Nothing is written to the User or Machine environment: the variable is set in this
    process only and disappears when the process exits.

.PARAMETER WorkDir
    Where the build will write, and where this check proves writability. Default: the
    package's parent directory plus `<package-directory-name>-build`, i.e. outside the
    package. Pass the same value to build.ps1.

.EXAMPLE
    PS> .\scripts\precheck.ps1
    PS> .\scripts\precheck.ps1 -Toolchain 1.98.0
    PS> .\scripts\precheck.ps1 -WorkDir D:\ohm-build
#>
#Requires -Version 7.0
[CmdletBinding()]
param(
    [switch]$AllowNonWindows,
    [string]$Toolchain = '',
    [string]$WorkDir = ''
)

$ErrorActionPreference = 'Stop'
# A native command that exits non-zero does not throw, whatever $ErrorActionPreference
# says. Every native call below reads $LASTEXITCODE explicitly (through
# Invoke-NativeCommand in _tools.ps1), and this preference is set to $false so that a
# tool writing to stderr is never mistaken for a tool failing.
$PSNativeCommandUseErrorActionPreference = $false

. (Join-Path $PSScriptRoot '_tools.ps1')

$script:PackageRoot = (Resolve-Path -LiteralPath (Split-Path -Parent $PSScriptRoot)).Path.TrimEnd([char]'\', [char]'/')
if ([string]::IsNullOrWhiteSpace($WorkDir)) {
    $WorkDir = Join-Path (Split-Path -Parent $script:PackageRoot) ((Split-Path -Leaf $script:PackageRoot) + '-build')
}
$script:WorkDir = [System.IO.Path]::GetFullPath($WorkDir).TrimEnd([char]'\', [char]'/')
$script:Failures = New-Object System.Collections.Generic.List[string]
$script:Notes = New-Object System.Collections.Generic.List[string]

function Add-Result {
    param([string]$Name, [bool]$Ok, [string]$Detail)
    $mark = if ($Ok) { 'PASS' } else { 'FAIL' }
    Write-Host ("  [{0}] {1,-46} {2}" -f $mark, $Name, $Detail)
    if (-not $Ok) { $script:Failures.Add("$Name`: $Detail") }
}

function Add-Note {
    param([string]$Text)
    $script:Notes.Add($Text)
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
Write-Host ("package : {0}" -f $script:PackageRoot)
Write-Host ("work    : {0}" -f $script:WorkDir)
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
        $result = Invoke-PowerShellScript -Path $verify -Arguments @('-Quiet') -WorkingDirectory $script:PackageRoot
        $ok = $result.ExitCode -eq 0
        $detail = if ($ok) { 'every file matches MANIFEST.sha256' } else { "contents differ from the manifest: $($result.Output.Trim())" }
        Add-Result 'package integrity' $ok $detail
        if (-not $ok) {
            Add-Note 'the package has to stay byte-for-byte the tree the manifest describes, which is why the build never writes inside it. If the unexpected files are a collector bundle, move it out of the package: .\docs\windows-validation\collect.ps1 -OutputRoot <work directory>\evidence. If they are anything else, the package has been modified and this run cannot be acceptance evidence.'
        }
    } else {
        Add-Result 'package integrity' $false 'scripts\verify-package.ps1 is missing, so the manifest cannot be checked'
    }
} else {
    Add-Result 'package integrity' $false 'MANIFEST.sha256 is missing — this is not an assembled package'
}

# --- 3. The work directory is writable, and it is not the package -------------
$probe = Test-WritableDirectory -Path $script:WorkDir
Add-Result 'work directory writable' $probe.Ok $probe.Detail
if ($script:WorkDir -eq $script:PackageRoot) {
    Add-Result 'work directory separate from the package' $false 'the work directory is the package itself: the build would leave target\, node_modules\ and evidence\ inside it, and every later verification of this package would fail'
} else {
    Add-Result 'work directory separate from the package' $true "the build writes to $($script:WorkDir); the package stays read-only"
}

# --- 4. Which tools this run will actually use --------------------------------
Write-Host ''
Write-Host 'Tools this run will use (absolute paths — nothing is taken from a bare name):'
$resolution = Get-ToolResolutionTable
foreach ($row in $resolution) {
    $shown = if ($row.Path) { $row.Path } else { 'NOT FOUND' }
    $origin = if ($row.Origin) { $row.Origin } else { '-' }
    Write-Host ("  {0,-7} {1,-60} {2}" -f $row.Name, $shown, $origin)
}
$scope = Set-ToolchainScope -Toolchain $Toolchain
if ($Toolchain) {
    Write-Host ("  toolchain: {0}" -f $scope.Detail)
    $pinned = $false
    $cargoNow = Get-ToolResolution -Name 'cargo' -FallbackDirectories (Get-ToolFallbackDirectories -Name 'cargo')
    if ($scope.PinnedPath -and $cargoNow.Path -and $scope.CargoBin) {
        $parent = (Split-Path -Parent $cargoNow.Path).TrimEnd([char]'\', [char]'/')
        $pinned = $parent -eq $scope.CargoBin.TrimEnd([char]'\', [char]'/')
    }
    $pinDetail = if ($pinned) {
        "the cargo a child process finds is the rustup proxy in $($scope.CargoBin), which honours RUSTUP_TOOLCHAIN"
    } elseif (-not $scope.PinnedPath) {
        $scope.Detail
    } else {
        "-Toolchain $Toolchain was requested, but the cargo a child process would find is $($cargoNow.Path), which is not the rustup proxy in $($scope.CargoBin). That cargo ignores RUSTUP_TOOLCHAIN, so `npx tauri build` would produce an installer from a toolchain nobody asked for. Put $($scope.CargoBin) before it on PATH."
    }
    Add-Result 'requested toolchain pinned for child processes' $pinned $pinDetail
}
Write-Host ''

# --- 5. Rust toolchain --------------------------------------------------------
# The version probes run with the package directory as their (read-only) working
# directory: `--version` reads nothing from the project, and nothing should run inside
# the work directory, which this check may have created and removed again.
$rustcVersion = $null
try {
    $invocation = Get-RustCommand -Tool 'rustc' -Toolchain $Toolchain -Arguments @('--version')
    $rustcVersion = (Invoke-NativeCommand -FilePath $invocation.FilePath -Arguments $invocation.Arguments `
        -WorkingDirectory $script:PackageRoot -FailOnError -What $invocation.Display).Output
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

if ($Toolchain) {
    try {
        $invocation = Get-RustCommand -Tool 'rustup' -Arguments @('toolchain', 'list')
        $listed = (Invoke-NativeCommand -FilePath $invocation.FilePath -Arguments $invocation.Arguments `
            -WorkingDirectory $script:PackageRoot -FailOnError -What 'rustup toolchain list').Output
        $installed = [bool]($listed -match "(^|\s)$([regex]::Escape($Toolchain))")
        $match = @($listed -split "`n" | Where-Object { $_ -match [regex]::Escape($Toolchain) }) | Select-Object -First 1
        Add-Result "toolchain '$Toolchain' is installed" $installed $(
            if ($installed) { "rustup lists: $match" }
            else { "rustup does not list '$Toolchain'. Installed: $($listed -replace "`n", '; ')" }
        )
    } catch {
        Add-Result 'rustup available' $false $_.Exception.Message
    }
}

try {
    $invocation = Get-RustCommand -Tool 'cargo' -Toolchain $Toolchain -Arguments @('--version')
    $cargoVersion = (Invoke-NativeCommand -FilePath $invocation.FilePath -Arguments $invocation.Arguments `
        -WorkingDirectory $script:PackageRoot -FailOnError -What $invocation.Display).Output
    Add-Result 'cargo available' $true $cargoVersion
} catch {
    Add-Result 'cargo available' $false $_.Exception.Message
}

# clippy must come from the same release as rustc. A mismatched pair does not lint,
# and it says so unclearly: clippy-driver 0.1.92 beside rustc 1.98.0 makes cargo
# apply the rust-version gate and abort before reading a line of code.
try {
    $invocation = Get-RustCommand -Tool 'cargo' -Toolchain $Toolchain -Arguments @('clippy', '--version')
    $clippyVersion = (Invoke-NativeCommand -FilePath $invocation.FilePath -Arguments $invocation.Arguments `
        -WorkingDirectory $script:PackageRoot -FailOnError -What 'cargo clippy --version').Output
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

# --- 6. Node and npm ----------------------------------------------------------
try {
    $node = Get-ToolPath 'node'
    $nodeVersion = (Invoke-NativeCommand -FilePath $node -Arguments @('--version') `
        -WorkingDirectory $script:PackageRoot -FailOnError -What 'node --version').Output
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
    $npm = Get-ToolPath 'npm'
    $npmVersion = (Invoke-NativeCommand -FilePath $npm -Arguments @('--version') `
        -WorkingDirectory $script:PackageRoot -FailOnError -What 'npm --version').Output
    Add-Result 'npm available' $true $npmVersion
} catch {
    Add-Result 'npm available' $false $_.Exception.Message
}

# --- 7. Disk space ------------------------------------------------------------
# A full workspace build, npm's cache and the NSIS bundle need room. This is a check
# about the machine the build happens on, measured on the volume the build writes to —
# the work directory, not the package.
#
# Windows only, and the gate is $IsWindows rather than $env:OS: a non-Windows host has
# no drive letters, and reading the first three characters of '/Users/name/pkg' as a
# drive name is how this check once reported free space for the drive '/Us'. A path
# with no drive qualifier is now reported as such instead of being guessed at.
# The decision itself lives in Test-DiskSpace (_tools.ps1), with the platform and the
# drive reading passed in, so the harness can exercise it on a machine that has neither.
$disk = Test-DiskSpace -Path $script:WorkDir -IsWindowsHost $IsWindows -FreeSpaceProvider {
    param($qualifier)
    (Get-PSDrive -Name $qualifier.TrimEnd(':') -ErrorAction Stop).Free
}
switch ($disk.Kind) {
    'passed'     { Add-Result 'free disk space >= 8 GB' $true $disk.Detail }
    'short'      { Add-Result 'free disk space >= 8 GB' $false $disk.Detail }
    'unreadable' { Add-Note $disk.Detail }
    default      { Add-Note $disk.Detail }
}
if ($IsWindows) {
    $packageQualifier = Get-DriveQualifier -Path $script:PackageRoot
    if ($packageQualifier -and $packageQualifier -ne $disk.Qualifier) {
        Add-Note "the package is on $packageQualifier and the work directory is on $($disk.Qualifier)`: the build needs its space on the second, and copying the source crosses volumes."
    }
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
Write-Host 'Next, from the package root:'
Write-Host '  1. .\scripts\verify-package.ps1'
Write-Host ('  2. .\docs\windows-validation\collect.ps1 -DryRun -OutputRoot "{0}\evidence"   (then run it again without -DryRun)' -f $script:WorkDir)
Write-Host '     The collector writes a bundle folder into the directory it is given: give it one outside the package, or the package stops matching its own manifest.'
Write-Host ('  3. .\scripts\build.ps1   — this builds in {0}, never inside the package' -f $script:WorkDir)
exit 0
