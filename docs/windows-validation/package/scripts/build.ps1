<#
.SYNOPSIS
    Builds and tests OpenHardwareOS from this package, capturing evidence, without
    writing anything inside the package.

.DESCRIPTION
    Runs the pre-check first and refuses to continue if it fails. Then, in order:
    frontend install, type check, frontend tests, frontend production build, the Rust
    workspace build (all targets), the Rust test suite, and the NSIS installer.

    Everything happens in a *work directory* outside the package:

        <package-parent>\<package-name>-build\          (default; -WorkDir overrides)
            source\      a byte-for-byte copy of the package, made fresh on every run
            target\      CARGO_TARGET_DIR — cargo's output, kept between runs
            evidence\
                <stamp>\ this run's logs, environment, installer record and summary

    The package is treated as read-only and is verified twice: before its source is
    copied, and again after the build. If it gained, lost or changed a single file —
    including a build that wrote its output into the package — the run fails and names
    the file. That is what makes a second run over the same package possible at all:
    nothing this run does lands in the package, so the manifest still describes it.

    A run reports success only if the installer *this run* was supposed to produce is
    there. The expected name comes from apps/desktop/src-tauri/tauri.conf.json
    (productName, version, and the `nsis` bundle target), the file must be non-empty,
    and the pre-build state is recorded before the bundle step so that a stale file
    left by an earlier build cannot pass as this run's artefact. An empty bundle
    directory, a stale installer, and an installer for another product or version each
    fail with a message naming which of those happened.

.PARAMETER Toolchain
    rustup toolchain to build with, e.g. `1.98.0`. Every Rust invocation goes through
    `rustup run <name>`, and RUSTUP_TOOLCHAIN is set in this process so that the cargo
    inside `npx tauri build` uses the same toolchain — including the frontend build
    Tauri runs as its `beforeBuildCommand`. Nothing is written to the User or Machine
    environment, and no shell profile is touched.

.PARAMETER WorkDir
    Where the build happens. Default: the package's parent directory plus
    `<package-directory-name>-build`, i.e. outside the package. Pass the same value to
    precheck.ps1.

.PARAMETER SkipBundle
    Stop after the test suite. Useful when the point of the run is the Rust evidence
    rather than the installer. The package is still verified before and after.

.PARAMETER AllowNonWindows
    Forwarded to the pre-check, for a contributor sanity-checking a checkout on a
    non-Windows host. A run with this switch is explicitly NOT Windows acceptance
    evidence, and never use it on the Windows machine the session is about.

.EXAMPLE
    PS> .\scripts\build.ps1
    PS> .\scripts\build.ps1 -Toolchain 1.98.0
    PS> .\scripts\build.ps1 -SkipBundle
    PS> .\scripts\build.ps1 -WorkDir D:\ohm-build
#>
#Requires -Version 7.0
[CmdletBinding()]
param(
    [string]$Toolchain = '',
    [string]$WorkDir = '',
    [switch]$SkipBundle,
    [switch]$AllowNonWindows
)

$ErrorActionPreference = 'Stop'
# Native commands report failure through $LASTEXITCODE and never throw, whatever
# $ErrorActionPreference says. Every one of them is run through Invoke-NativeCommand
# (in _tools.ps1), which reads the exit code at the point it is set, and this
# preference is $false so a tool writing to stderr is not mistaken for a tool failing.
$PSNativeCommandUseErrorActionPreference = $false

. (Join-Path $PSScriptRoot '_tools.ps1')

$script:PackageRoot = (Resolve-Path -LiteralPath (Split-Path -Parent $PSScriptRoot)).Path.TrimEnd([char]'\', [char]'/')
if ([string]::IsNullOrWhiteSpace($WorkDir)) {
    $WorkDir = Join-Path (Split-Path -Parent $script:PackageRoot) ((Split-Path -Leaf $script:PackageRoot) + '-build')
}
$script:WorkDir = [System.IO.Path]::GetFullPath($WorkDir).TrimEnd([char]'\', [char]'/')
$script:SourceCopy = Join-Path $script:WorkDir 'source'
$script:TargetDir = Join-Path $script:WorkDir 'target'
$script:RunStamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$script:Evidence = Join-Path (Join-Path $script:WorkDir 'evidence') $script:RunStamp
$script:Started = Get-Date
$script:StepIndex = 0
$script:Summary = New-Object System.Collections.Generic.List[string]
$script:InstallerInfo = $null
$script:InstallerRecord = ''

if ($script:WorkDir -eq $script:PackageRoot) {
    Write-Host 'REFUSING TO START: -WorkDir points at the package itself.' -ForegroundColor Red
    Write-Host 'The build would leave target\, node_modules\ and evidence\ inside the package, and every later verification of the package would fail.'
    Write-Host 'Give -WorkDir a directory outside the package (the default is a sibling of it).'
    exit 1
}

New-Item -ItemType Directory -Force -Path $script:Evidence | Out-Null
New-Item -ItemType Directory -Force -Path $script:TargetDir | Out-Null

# --- The tools this run will use ----------------------------------------------
$resolutionRows = Get-ToolResolutionTable
$scope = Set-ToolchainScope -Toolchain $Toolchain
# Keep cargo's output out of the copied source: the copy is reset on every run, and a
# warm target directory is what makes a second run cheap instead of a full rebuild.
$env:CARGO_TARGET_DIR = $script:TargetDir

function Write-Summary {
    $lines = @(
        'OpenHardwareOS acceptance build — summary',
        "package    : $script:PackageRoot",
        "work dir   : $script:WorkDir",
        "evidence   : $script:Evidence",
        "finished   : $((Get-Date).ToUniversalTime().ToString('u'))",
        "toolchain  : $(if ($Toolchain) { $Toolchain } else { 'PATH resolution (no -Toolchain given)' })",
        ''
    ) + $script:Summary
    if ($script:InstallerRecord) { $lines += '', $script:InstallerRecord }
    Set-Content -LiteralPath (Join-Path $script:Evidence 'build-summary.txt') -Value $lines
}

function Write-Environment {
    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add('OpenHardwareOS acceptance build — environment of this run')
    $lines.Add('')
    $lines.Add("package           : $script:PackageRoot")
    $lines.Add("work dir          : $script:WorkDir")
    $lines.Add("source copy       : $script:SourceCopy")
    $lines.Add("target dir        : $script:TargetDir  (CARGO_TARGET_DIR)")
    $lines.Add("evidence          : $script:Evidence")
    $lines.Add("started (UTC)     : $($script:Started.ToUniversalTime().ToString('u'))")
    $lines.Add("host              : $([System.Runtime.InteropServices.RuntimeInformation]::OSDescription)")
    $lines.Add("powershell        : $($PSVersionTable.PSVersion) $($PSVersionTable.PSEdition) in $PSHOME")
    $lines.Add("toolchain         : $(if ($Toolchain) { $Toolchain } else { 'none requested' })")
    $lines.Add("RUSTUP_TOOLCHAIN  : $(if ($env:RUSTUP_TOOLCHAIN) { $env:RUSTUP_TOOLCHAIN } else { '(unset)' })  — this process and its children only")
    $lines.Add("CARGO_TARGET_DIR  : $env:CARGO_TARGET_DIR")
    $lines.Add("toolchain scope   : $($scope.Detail)")
    $lines.Add('')
    $lines.Add('Tools (absolute paths):')
    foreach ($row in $resolutionRows) {
        $shown = if ($row.Path) { $row.Path } else { 'NOT FOUND' }
        $origin = if ($row.Origin) { $row.Origin } else { '-' }
        $lines.Add(("  {0,-7} {1}" -f $row.Name, $shown))
        $lines.Add(("  {0,-7} found via {1}" -f '', $origin))
    }
    $lines.Add('')
    $lines.Add('Nothing in this run writes to the User or Machine environment, and no shell')
    $lines.Add('profile is touched. RUSTUP_TOOLCHAIN and PATH are set in the script process only,')
    $lines.Add('so that child processes inherit the pin and nothing survives the process.')
    Set-Content -LiteralPath (Join-Path $script:Evidence '00-environment.txt') -Value $lines
}

function Fail-Run {
    param(
        [Parameter(Mandatory)][string]$Message,
        [string[]]$Detail = @()
    )
    Write-Host ''
    Write-Host $Message -ForegroundColor Red
    foreach ($line in $Detail) { Write-Host "  $line" -ForegroundColor Red }
    $script:Summary.Add("FAIL $Message")
    Write-Summary
    Write-Host ''
    Write-Host 'No BUILD COMPLETE was printed: this run did not produce what it claims to verify.' -ForegroundColor Red
    Write-Host ("Evidence so far: {0}" -f $script:Evidence) -ForegroundColor Red
    exit 1
}

function Invoke-Step {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][string]$Command,
        [string[]]$Arguments = @(),
        [Parameter(Mandatory)][string]$WorkingDirectory
    )
    $script:StepIndex++
    $label = ($Name -replace '[^A-Za-z0-9]+', '-').ToLower().Trim('-')
    $log = Join-Path $script:Evidence ("step-{0:d2}-{1}.log" -f $script:StepIndex, $label)
    Write-Host ("[{0}] {1}" -f $script:StepIndex, $Name)
    Write-Host ("     {0} {1}" -f $Command, ($Arguments -join ' '))
    Write-Host ("     in  {0}" -f $WorkingDirectory)
    $started = Get-Date
    $result = Invoke-NativeCommand -FilePath $Command -Arguments $Arguments -WorkingDirectory $WorkingDirectory
    $elapsed = [math]::Round(((Get-Date) - $started).TotalSeconds, 1)
    $header = @(
        "command  : $Command $($Arguments -join ' ')",
        "cwd      : $WorkingDirectory",
        "started  : $($started.ToUniversalTime().ToString('u'))",
        "seconds  : $elapsed",
        "exit code: $($result.ExitCode)",
        ''
    )
    Set-Content -LiteralPath $log -Value ($header + @($result.Output -split "`n"))

    if ($result.ExitCode -ne 0) {
        Write-Host ("     FAILED (exit {0}) after {1}s — last lines of {2}:" -f $result.ExitCode, $elapsed, (Split-Path -Leaf $log)) -ForegroundColor Red
        @($result.Output -split "`n") | Select-Object -Last 25 | ForEach-Object { Write-Host "     $_" }
        Fail-Run ("RUN STOPPED at step {0} ({1}), exit {2}. A build that 'mostly worked' produces evidence nobody can trust, and continuing past a failure buries the first, most useful error." -f $script:StepIndex, $Name, $result.ExitCode) @(
            "Full output: $log",
            'Capture that log in the evidence bundle and report it. Do not re-run the remaining steps.'
        )
    }
    Write-Host ("     ok ({0}s)" -f $elapsed) -ForegroundColor Green
    $script:Summary.Add(("PASS step {0} {1} (exit 0, {2}s) -> {3}" -f $script:StepIndex, $Name, $elapsed, (Split-Path -Leaf $log)))
}

function Get-CargoMetadata {
    <#
    .SYNOPSIS
        cargo metadata, bound to the work copy's manifest and to an explicit directory.
    .DESCRIPTION
        Run without --manifest-path and without an explicit working directory, this
        answers about whatever project the caller's directory happens to hold: from
        another directory it fails, and from another project it silently reports *that*
        project's target directory, which is where the installer is then looked for.
    #>
    param(
        [Parameter(Mandatory)][string]$SourceCopy,
        [string]$Toolchain = ''
    )
    $manifestPath = Join-Path $SourceCopy 'Cargo.toml'
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "there is no Cargo.toml in the work copy ($manifestPath), so the bundle location cannot be derived"
    }
    $invocation = Get-RustCommand -Tool 'cargo' -Toolchain $Toolchain -Arguments @(
        'metadata', '--format-version', '1', '--no-deps', '--manifest-path', $manifestPath
    )
    $result = Invoke-NativeCommand -FilePath $invocation.FilePath -Arguments $invocation.Arguments `
        -WorkingDirectory $SourceCopy -FailOnError -What 'cargo metadata'
    # cargo prints warnings around the JSON, so take the JSON object itself rather than
    # assuming the whole stream parses.
    $text = $result.Output
    $start = $text.IndexOf('{')
    $end = $text.LastIndexOf('}')
    if ($start -lt 0 -or $end -le $start) {
        throw "cargo metadata exited 0 without printing JSON: $text"
    }
    $json = $text.Substring($start, $end - $start + 1)
    try {
        $meta = $json | ConvertFrom-Json
    } catch {
        throw "cargo metadata's output could not be parsed: $($_.Exception.Message)"
    }
    if (-not $meta.target_directory) {
        throw 'cargo metadata reported no target_directory, so the bundle location cannot be derived'
    }
    $evidence = @(
        "command : $($invocation.Display)",
        "cwd     : $SourceCopy",
        "exit    : $($result.ExitCode)",
        ''
    ) + @($json)
    Set-Content -LiteralPath (Join-Path $script:Evidence 'cargo-metadata.json') -Value $evidence
    return [pscustomobject]@{ TargetDirectory = $meta.target_directory; Command = $invocation.Display }
}

function Get-TauriBundle {
    <#
    .SYNOPSIS
        The installer this project's own Tauri configuration says to expect.
    #>
    param([Parameter(Mandatory)][string]$SourceCopy)
    $confPath = Join-Path $SourceCopy 'apps/desktop/src-tauri/tauri.conf.json'
    if (-not (Test-Path -LiteralPath $confPath -PathType Leaf)) {
        throw "apps/desktop/src-tauri/tauri.conf.json is missing from the work copy ($confPath), so the expected installer cannot be named"
    }
    try {
        $conf = Get-Content -LiteralPath $confPath -Raw | ConvertFrom-Json
    } catch {
        throw "apps/desktop/src-tauri/tauri.conf.json could not be parsed: $($_.Exception.Message)"
    }
    $targets = @($conf.bundle.targets)
    if ($targets -notcontains 'nsis') {
        throw ("apps/desktop/src-tauri/tauri.conf.json does not build an NSIS installer (bundle.targets = {0}). This script verifies an NSIS installer and will not report success for anything else." -f (($targets -join ', ')))
    }
    $product = [string]$conf.productName
    $version = [string]$conf.version
    if ([string]::IsNullOrWhiteSpace($product)) { throw 'apps/desktop/src-tauri/tauri.conf.json has no productName, so the installer file name cannot be derived' }
    if ([string]::IsNullOrWhiteSpace($version)) { throw 'apps/desktop/src-tauri/tauri.conf.json has no version, so the installer file name cannot be derived' }
    $arch = switch ($env:PROCESSOR_ARCHITECTURE) {
        'AMD64' { 'x64' }
        'ARM64' { 'arm64' }
        'x86' { 'x86' }
        default { '' }
    }
    if (-not $arch) {
        throw "cannot derive the installer's architecture token from PROCESSOR_ARCHITECTURE='$($env:PROCESSOR_ARCHITECTURE)'"
    }
    return [pscustomobject]@{
        ProductName  = $product
        Version      = $version
        Arch         = $arch
        ExpectedName = "${product}_${version}_${arch}-setup.exe"
        ConfigPath   = $confPath
    }
}

function Get-InstallerState {
    <#
    .SYNOPSIS
        The state of the expected installer before the bundle step runs.
    #>
    param(
        [Parameter(Mandatory)][string]$BundleRoot,
        [Parameter(Mandatory)][string]$ExpectedName
    )
    $path = Join-Path (Join-Path $BundleRoot 'nsis') $ExpectedName
    $exists = Test-Path -LiteralPath $path -PathType Leaf
    $length = 0
    $hash = ''
    $lastWrite = $null
    if ($exists) {
        $item = Get-Item -LiteralPath $path
        $length = $item.Length
        $hash = Get-FileSha256 -Path $path
        $lastWrite = $item.LastWriteTimeUtc
    }
    return [pscustomobject]@{
        Path            = $path
        ExpectedName    = $ExpectedName
        Exists          = $exists
        Length          = $length
        Sha256          = $hash
        LastWriteTimeUtc = $lastWrite
        CapturedAtUtc   = (Get-Date).ToUniversalTime()
    }
}

function Test-InstallerProduced {
    <#
    .SYNOPSIS
        Did *this run* produce the installer it was supposed to produce?
    .DESCRIPTION
        Four ways to fail, each with its own message: the bundle directory is absent,
        the expected file is absent, the file is empty, the file was already there
        before the bundle step and was not rewritten by it. `Get-ChildItem`'s result is
        the verdict; the artefact listing that follows is only evidence.
    #>
    param(
        [Parameter(Mandatory)]$PreState,
        [Parameter(Mandatory)][string]$BundleRoot,
        [Parameter(Mandatory)][string]$ExpectedName
    )
    if (-not (Test-Path -LiteralPath $BundleRoot -PathType Container)) {
        return [pscustomobject]@{ Ok = $false; Kind = 'missing'; Message = "INSTALLER MISSING: there is no bundle directory at $BundleRoot, so this run produced no installer — `npx tauri build` exited 0 without writing one." }
    }
    $found = @(Get-ChildItem -LiteralPath $BundleRoot -Recurse -File -Filter $ExpectedName -ErrorAction SilentlyContinue)
    if ($found.Count -eq 0) {
        $others = @(Get-ChildItem -LiteralPath $BundleRoot -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -like '*-setup.exe' })
        if ($others.Count -gt 0) {
            return [pscustomobject]@{
                Ok      = $false
                Kind    = 'mismatch'
                Message = ("INSTALLER MISMATCH: expected '{0}' under {1}, but the bundle holds {2}. An installer built by another project, or for another version, is not evidence that this run produced the installer this project declares in tauri.conf.json." -f `
                    $ExpectedName, $BundleRoot, (($others | ForEach-Object { $_.FullName }) -join '; '))
            }
        }
        return [pscustomobject]@{
            Ok      = $false
            Kind    = 'missing'
            Message = ("INSTALLER MISSING: '{0}' does not exist under {1}, so this run produced no installer. A bundle directory that merely exists is not an installer." -f $ExpectedName, $BundleRoot)
        }
    }
    $file = Get-Item -LiteralPath $found[0].FullName
    if ($file.Length -le 0) {
        return [pscustomobject]@{ Ok = $false; Kind = 'empty'; Message = ("INSTALLER EMPTY: '{0}' exists but is 0 bytes, which is not an installer." -f $file.FullName) }
    }
    if ($PreState.Exists -and $file.LastWriteTimeUtc -le $PreState.LastWriteTimeUtc) {
        return [pscustomobject]@{
            Ok      = $false
            Kind    = 'stale'
            Message = ("INSTALLER STALE: '{0}' was already there before the bundle step and was not rewritten by it (last write {1}, before the step recorded {2}). A leftover from an earlier build is not evidence that this run produced an installer. Delete that file — or the whole work directory — and run again." -f `
                $file.FullName, $file.LastWriteTimeUtc.ToString('u'), $PreState.CapturedAtUtc.ToString('u'))
        }
    }
    return [pscustomobject]@{ Ok = $true; Kind = 'ok'; File = $file; Message = '' }
}

function Assert-PackageUntouched {
    <#
    .SYNOPSIS
        The package must still be exactly the tree the manifest describes.
    .DESCRIPTION
        Two questions, because they fail for different reasons: did this run *add* or
        remove a file in the package (a build that wrote its output where the source
        lives), and does every file still match MANIFEST.sha256 (source tampering, or a
        tool that rewrote something in place).
    #>
    param(
        [Parameter(Mandatory)][string]$PackageRoot,
        [Parameter(Mandatory)][string[]]$Before,
        [Parameter(Mandatory)][string]$VerifyScript,
        [Parameter(Mandatory)][string]$LogPath
    )
    $after = Get-RelativeFileList -Root $PackageRoot
    $diff = Compare-FileLists -Before $Before -After $after
    if ($diff.Gained.Count -gt 0) {
        $detail = @($diff.Gained | ForEach-Object { "+ $_" })
        $detail += "Nothing may be written inside $PackageRoot. The build's output belongs in $script:WorkDir."
        Fail-Run ("PACKAGE MODIFIED BY THIS RUN — the build put {0} file(s) inside the source package, which is read-only by design. The package no longer matches its own manifest, so it can no longer be used as acceptance evidence." -f $diff.Gained.Count) $detail
    }
    if ($diff.Removed.Count -gt 0) {
        $detail = @($diff.Removed | ForEach-Object { "- $_" })
        Fail-Run ("PACKAGE MODIFIED BY THIS RUN — {0} file(s) that the package held before the build are gone." -f $diff.Removed.Count) $detail
    }
    $verify = Invoke-PowerShellScript -Path $VerifyScript -WorkingDirectory $PackageRoot -LogPath $LogPath
    Write-Host $verify.Output
    if ($verify.ExitCode -ne 0) {
        Fail-Run 'PACKAGE CHANGED DURING THE BUILD — the package no longer matches MANIFEST.sha256 (the files are named above). The run is not evidence: what was built is not what the manifest describes.' @("Verification log: $LogPath")
    }
    $script:Summary.Add(("PASS package unchanged by this run ({0} file(s) still match MANIFEST.sha256)" -f $after.Count))
}

# --- Start --------------------------------------------------------------------
Write-Host ''
Write-Host 'OpenHardwareOS — Windows acceptance build'
Write-Host ("package : {0}" -f $script:PackageRoot)
Write-Host ("work    : {0}" -f $script:WorkDir)
Write-Host ("copy    : {0}" -f $script:SourceCopy)
Write-Host ("target  : {0}" -f $script:TargetDir)
Write-Host ("evidence: {0}" -f $script:Evidence)
Write-Host ("started : {0}" -f $script:Started.ToUniversalTime().ToString('u'))
Write-Host ''
Write-Host 'Tools this run will use (absolute paths):'
foreach ($row in $resolutionRows) {
    $shown = if ($row.Path) { $row.Path } else { 'NOT FOUND' }
    Write-Host ("  {0,-7} {1}" -f $row.Name, $shown)
}
if ($Toolchain) { Write-Host ("  toolchain: {0}" -f $scope.Detail) }
Write-Host ''
Write-Environment

# --- Pre-check (a failed run must not start) ----------------------------------
$precheck = Join-Path $PSScriptRoot 'precheck.ps1'
$precheckArgs = @()
if ($Toolchain) { $precheckArgs += @('-Toolchain', $Toolchain) }
$precheckArgs += @('-WorkDir', $script:WorkDir)
if ($AllowNonWindows) { $precheckArgs += '-AllowNonWindows' }
Write-Host '[gate] pre-check (its own process; the build does not start unless it exits 0)'
$precheckResult = Invoke-PowerShellScript -Path $precheck -Arguments $precheckArgs `
    -WorkingDirectory $script:PackageRoot -LogPath (Join-Path $script:Evidence '00-precheck.log')
Write-Host $precheckResult.Output
$script:Summary.Add("PRE-CHECK exit $($precheckResult.ExitCode) -> 00-precheck.log")
if ($precheckResult.ExitCode -ne 0) {
    Fail-Run 'BUILD NOT STARTED — the pre-check failed. A run started now would report machine problems as project findings.' @(
        'See its output above and 00-precheck.log. Fix the machine, then run again; do not edit the scripts to get past this.'
    )
}

# --- The package is read-only: verify it before copying it --------------------
$verifyScript = Join-Path $PSScriptRoot 'verify-package.ps1'
Write-Host '[gate] verify the package before copying it into the work directory'
$verifyBefore = Invoke-PowerShellScript -Path $verifyScript -WorkingDirectory $script:PackageRoot `
    -LogPath (Join-Path $script:Evidence '01-package-verify-before.log')
Write-Host $verifyBefore.Output
if ($verifyBefore.ExitCode -ne 0) {
    Fail-Run 'BUILD NOT STARTED — the package does not match MANIFEST.sha256, so what would be built is not the tree the manifest describes.' @(
        'The names of the differing files are printed above and written to 01-package-verify-before.log.',
        'Do not edit the package to get past this. Re-extract it, if the difference is not yours.'
    )
}
$packageFilesBefore = Get-RelativeFileList -Root $script:PackageRoot
$script:Summary.Add("PASS package verified before the build ($($packageFilesBefore.Count) file(s))")

# --- Copy the source out of the package (fresh, every run) --------------------
Write-Host ''
Write-Host ("[copy] {0} -> {1}" -f $script:PackageRoot, $script:SourceCopy)
if (Test-Path -LiteralPath $script:SourceCopy) {
    Write-Host '       removing the previous run''s copy (the build never reads it, and a stale copy is not evidence)'
    Remove-Item -LiteralPath $script:SourceCopy -Recurse -Force
}
$copy = Copy-PackageToDirectory -Source $script:PackageRoot -Destination $script:SourceCopy -VerifyHashes
Write-Host ("       {0} file(s) copied, every one checked by SHA-256 against its source" -f $copy.FileCount)
$script:Summary.Add("PASS source copied to the work directory ($($copy.FileCount) file(s), hashes verified)")

$desktop = Join-Path $script:SourceCopy 'apps/desktop'
$npm = Get-ToolPath 'npm'

# --- Frontend -----------------------------------------------------------------
Invoke-Step 'frontend install (npm ci)' $npm @('ci', '--no-audit', '--no-fund') $desktop
Invoke-Step 'frontend type check' $npm @('run', 'typecheck') $desktop
Invoke-Step 'frontend tests' $npm @('run', 'test') $desktop
Invoke-Step 'frontend build' $npm @('run', 'build') $desktop

# --- Rust ---------------------------------------------------------------------
$cargoBuild = Get-RustCommand -Tool 'cargo' -Toolchain $Toolchain -Arguments @('build', '--workspace', '--all-targets')
Invoke-Step 'rust workspace build (all targets)' $cargoBuild.FilePath $cargoBuild.Arguments $script:SourceCopy
$cargoTest = Get-RustCommand -Tool 'cargo' -Toolchain $Toolchain -Arguments @('test', '--workspace')
Invoke-Step 'rust test suite' $cargoTest.FilePath $cargoTest.Arguments $script:SourceCopy

# --- Installer ----------------------------------------------------------------
if ($SkipBundle) {
    Write-Host '[bundle] skipped by -SkipBundle'
    $script:Summary.Add('SKIP installer bundle (-SkipBundle): this run makes no claim about an installer')
} else {
    # Where the bundle lands is asked for, not assumed: cargo metadata reports the
    # target directory, and it is asked in the work copy with an explicit manifest and
    # working directory so the answer cannot be another project's.
    $metadata = Get-CargoMetadata -SourceCopy $script:SourceCopy -Toolchain $Toolchain
    $bundleRoot = Join-Path $metadata.TargetDirectory 'release/bundle'
    $tauri = Get-TauriBundle -SourceCopy $script:SourceCopy
    Write-Host ''
    Write-Host '[bundle] expected installer, from apps/desktop/src-tauri/tauri.conf.json:'
    Write-Host ("         {0}  (productName {1}, version {2}, nsis target, {3})" -f $tauri.ExpectedName, $tauri.ProductName, $tauri.Version, $tauri.Arch)
    Write-Host ("         bundle root: {0}" -f $bundleRoot)
    # Record what is there *before* the build, so a file left by an earlier run cannot
    # pass as this run's artefact.
    $preState = Get-InstallerState -BundleRoot $bundleRoot -ExpectedName $tauri.ExpectedName
    if ($preState.Exists) {
        Write-Host ("         before this run: {0} bytes, sha256 {1}, last write {2}" -f $preState.Length, $preState.Sha256, $preState.LastWriteTimeUtc.ToString('u'))
        $script:Summary.Add("PRE-BUILD installer state: exists, $($preState.Length) bytes, sha256=$($preState.Sha256), lastWrite=$($preState.LastWriteTimeUtc.ToString('u'))")
    } else {
        Write-Host '         before this run: no installer at that path'
        $script:Summary.Add('PRE-BUILD installer state: absent')
    }

    $npx = Get-ToolPath 'npx'
    Invoke-Step 'NSIS installer bundle (npx tauri build)' $npx @('tauri', 'build') $desktop

    # The package must still be the package — checked before the installer verdict, so
    # that a build which wrote into the read-only package is reported as that, and not
    # as a missing installer.
    Write-Host ''
    Write-Host '[gate] verify the package again, after the build'
    Assert-PackageUntouched -PackageRoot $script:PackageRoot -Before $packageFilesBefore -VerifyScript $verifyScript `
        -LogPath (Join-Path $script:Evidence '90-package-verify-after.log')

    $verdict = Test-InstallerProduced -PreState $preState -BundleRoot $bundleRoot -ExpectedName $tauri.ExpectedName
    if (-not $verdict.Ok) {
        Fail-Run $verdict.Message @(
            "tauri.conf.json says the installer is '$($tauri.ExpectedName)'.",
            "bundle root: $bundleRoot",
            'The listing below the bundle root is kept for the evidence bundle, but the verdict comes from this check, not from a directory that happens to exist.'
        )
    }
    $installer = $verdict.File
    $installerHash = Get-FileSha256 -Path $installer.FullName
    Write-Host ''
    Write-Host 'installer produced by this run:' -ForegroundColor Green
    Write-Host ("  path   : {0}" -f $installer.FullName)
    Write-Host ("  bytes  : {0}" -f $installer.Length)
    Write-Host ("  sha256 : {0}" -f $installerHash)
    Write-Host ("  written: {0}" -f $installer.LastWriteTimeUtc.ToString('u'))

    $artefacts = @(Get-ChildItem -LiteralPath $bundleRoot -Recurse -File)
    Write-Host ("bundle contents under {0}:" -f $bundleRoot)
    $artefactLines = New-Object System.Collections.Generic.List[string]
    foreach ($item in $artefacts) {
        $hash = Get-FileSha256 -Path $item.FullName
        Write-Host ("  {0}  {1} bytes  {2}" -f $hash, $item.Length, $item.FullName)
        $artefactLines.Add(("ARTEFACT {0} sha256={1} bytes={2}" -f (Get-PathRelativeTo -Path $item.FullName -Base $bundleRoot), $hash, $item.Length))
        $script:Summary.Add(("ARTEFACT {0} sha256={1} bytes={2}" -f (Get-PathRelativeTo -Path $item.FullName -Base $bundleRoot), $hash, $item.Length))
    }

    # Deliberately not $script:Installer: PowerShell variable names are
    # case-insensitive, so `$script:Installer` and the `$installer` FileInfo above are
    # the same variable, and assigning this summary would silently replace the file
    # with it. (The harness caught exactly that: `$installer.LastWriteTimeUtc` became
    # null one statement later.)
    $script:InstallerInfo = [pscustomobject]@{
        Path    = $installer.FullName
        Length  = $installer.Length
        Sha256  = $installerHash
        Written = $installer.LastWriteTimeUtc
    }
    $script:InstallerRecord = @(
        'INSTALLER (the artefact this run verified, not merely a directory that exists)',
        "path        : $($installer.FullName)",
        "name        : $($installer.Name)",
        "bytes       : $($installer.Length)",
        "sha256      : $installerHash",
        "written     : $($installer.LastWriteTimeUtc.ToString('u'))",
        "expected as : $($tauri.ExpectedName)  (from $($tauri.ConfigPath))",
        "bundle root : $bundleRoot",
        "target dir  : $($metadata.TargetDirectory)  (cargo metadata: $($metadata.Command))",
        "pre-build   : $(if ($preState.Exists) { "existed, $($preState.Length) bytes, sha256 $($preState.Sha256)" } else { 'absent' })"
    ) -join "`n"
    Set-Content -LiteralPath (Join-Path $script:Evidence '91-installer.txt') -Value (
        @($script:InstallerRecord) + @('', 'Everything under the bundle root:') + @($artefactLines) + @(
            '', 'This hash is what an operator compares against the file they install, and what'
            'the report must record. A build that exits 0 without producing this file is not'
            'a build that produced an installer.'
        )
    )
    $script:Summary.Add(("INSTALLER VERIFIED {0} sha256={1} bytes={2}" -f $installer.FullName, $installerHash, $installer.Length))
}

# --- The package must still be the package ------------------------------------
if ($SkipBundle) {
    Write-Host ''
    Write-Host '[gate] verify the package again, after the build'
    Assert-PackageUntouched -PackageRoot $script:PackageRoot -Before $packageFilesBefore -VerifyScript $verifyScript `
        -LogPath (Join-Path $script:Evidence '90-package-verify-after.log')
}

Write-Host ''
Write-Host 'BUILD COMPLETE' -ForegroundColor Green
Write-Summary
Write-Host ("evidence: {0}" -f $script:Evidence)
if ($script:InstallerInfo) {
    Write-Host ("installer: {0}  ({1} bytes, sha256 {2})" -f $script:InstallerInfo.Path, $script:InstallerInfo.Length, $script:InstallerInfo.Sha256)
}
Write-Host ''
Write-Host 'Next: docs\windows-validation\checklist.md §7.2 (install, needs administrator),'
Write-Host 'then the rest of the checklist, and fill in result-template.md.'
exit 0
