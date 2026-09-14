# Shared helpers for the Windows acceptance entry points (precheck.ps1, build.ps1).
#
# Dot-source it:  . (Join-Path $PSScriptRoot '_tools.ps1')
# It defines functions and executes nothing, so it is safe to source from anywhere.
#
# Why this file exists
# --------------------
# Tool resolution used to live in more than one place and to work in more than one
# way: the pre-check looked for `rustup` on PATH and then in `%USERPROFILE%\.cargo\bin`,
# while the build called a bare `rustup`, `cargo`, `node`, `npm` and `npx` and let the
# shell decide what those names meant. `-Toolchain` reached only some of the Rust
# invocations. The practical consequence is that a run could report the toolchain it
# checked and then build with a different one — evidence that cannot be reproduced.
#
# So every external tool is resolved here, to an absolute path, through one function.
# When a tool cannot be found the caller fails with a message naming the tool and every
# location that was searched; nothing ever falls back to a bare name.
#
# Environment discipline
# ----------------------
# Nothing in this file writes to the User or Machine environment and nothing edits a
# shell profile. `Set-ToolchainScope` — the one place that changes anything — sets
# RUSTUP_TOOLCHAIN and prepends rustup's own directory to PATH *in the current process
# only*, so that child processes (`npx tauri build`, and the cargo/rustc underneath it)
# inherit the pin, and so that nothing survives the process.

function Get-CargoBinDirectory {
    <#
    .SYNOPSIS
        The directory rustup installs its proxies (rustup, cargo, rustc) into.
    .DESCRIPTION
        `%CARGO_HOME%\bin`, which defaults to `%USERPROFILE%\.cargo\bin`. This
        directory — not PATH — is the authoritative location of the rustup proxies,
        so it is both a resolution fallback and the check for whether a `cargo` on
        PATH is a rustup proxy at all.
    #>
    [CmdletBinding()]
    param()
    if ($env:CARGO_HOME -and $env:CARGO_HOME.Trim()) { return (Join-Path $env:CARGO_HOME.Trim() 'bin') }
    if ($HOME) { return (Join-Path $HOME '.cargo/bin') }
    return $null
}

function Get-ToolExecutableSuffixes {
    [CmdletBinding()]
    param()
    if ($IsWindows) { return @('.exe', '.cmd', '.bat', '') }
    return @('')
}

function Get-ToolFallbackDirectories {
    <#
    .SYNOPSIS
        Directories to search after PATH, per tool.
    .DESCRIPTION
        Deliberately short. Rust tools fall back to rustup's own directory, because
        "rustup is not recognized" is a confusing way to tell an operator that their
        Rust is installed but their PATH is not. Node falls back to the default
        installer location on Windows for the same reason. Nothing else is guessed:
        a tool that can only be found by searching the whole disk is a tool whose
        provenance cannot be recorded.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Name)
    $directories = New-Object System.Collections.Generic.List[string]
    if ($Name -in @('rustup', 'cargo', 'rustc', 'rustfmt', 'cargo-clippy', 'clippy-driver')) {
        $cargoBin = Get-CargoBinDirectory
        if ($cargoBin) { $directories.Add($cargoBin) }
    }
    if ($Name -in @('node', 'npm', 'npx') -and $IsWindows -and $env:ProgramFiles) {
        $directories.Add((Join-Path $env:ProgramFiles 'nodejs'))
    }
    return $directories.ToArray()
}

function Get-ToolResolution {
    <#
    .SYNOPSIS
        Resolves one tool to an absolute path, recording where it came from.
    .OUTPUTS
        [pscustomobject] Name, Path (absolute, or $null), Origin ('PATH', 'fallback
        directory <dir>', or $null).
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Name,
        [string[]]$FallbackDirectories = @()
    )
    $command = Get-Command -Name $Name -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($command -and $command.Source) {
        return [pscustomobject]@{ Name = $Name; Path = $command.Source; Origin = 'PATH' }
    }
    foreach ($directory in $FallbackDirectories) {
        if ([string]::IsNullOrWhiteSpace($directory)) { continue }
        foreach ($suffix in (Get-ToolExecutableSuffixes)) {
            $candidate = Join-Path $directory "$Name$suffix"
            if (Test-Path -LiteralPath $candidate -PathType Leaf) {
                return [pscustomobject]@{
                    Name   = $Name
                    Path   = (Resolve-Path -LiteralPath $candidate).Path
                    Origin = "fallback directory $directory"
                }
            }
        }
    }
    return [pscustomobject]@{ Name = $Name; Path = $null; Origin = $null }
}

function Get-ToolPath {
    <#
    .SYNOPSIS
        The absolute path of one tool, or a loud failure naming what was searched.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Name)
    $fallbacks = Get-ToolFallbackDirectories -Name $Name
    $resolution = Get-ToolResolution -Name $Name -FallbackDirectories $fallbacks
    if ($resolution.Path) { return $resolution.Path }

    $hint = if ($Name -in @('rustup', 'cargo', 'rustc', 'rustfmt', 'cargo-clippy', 'clippy-driver')) {
        "Install Rust with rustup (https://rustup.rs), which puts these in $(Get-CargoBinDirectory), or add that directory to PATH."
    } elseif ($Name -in @('node', 'npm', 'npx')) {
        'Install Node.js (apps/desktop/package.json requires ^20.19.0 || >=22.12.0) and make sure its directory is on PATH.'
    } else {
        'Install it and make sure its directory is on PATH.'
    }
    $looked = @('PATH') + @($fallbacks)
    throw ("{0} was not found. Looked on: {1}. {2} Nothing here falls back to running a bare '{0}': a run whose tools are not pinned to absolute paths produces evidence nobody can reproduce." -f `
        $Name, ($looked -join ', '), $hint)
}

function Get-ToolResolutionTable {
    <#
    .SYNOPSIS
        The resolution of every tool this run needs, for both the report and the log.
    #>
    [CmdletBinding()]
    param([string[]]$Names = @('rustup', 'cargo', 'rustc', 'node', 'npm', 'npx'))
    $rows = foreach ($name in $Names) {
        Get-ToolResolution -Name $name -FallbackDirectories (Get-ToolFallbackDirectories -Name $name)
    }
    return @($rows)
}

function Set-ToolchainScope {
    <#
    .SYNOPSIS
        Pins the requested toolchain for this process and its children.
    .DESCRIPTION
        Two process-scope changes, and nothing else:

          * RUSTUP_TOOLCHAIN=<name> — inherited by every child, which is how it
            reaches `npx tauri build` and the cargo/rustc that Tauri runs underneath.
          * rustup's own directory is moved to the front of PATH, so the `cargo` a
            child finds is the rustup proxy. Without this, a `cargo` from another
            installation (Homebrew, a distro package, a scoop shim) would sit earlier
            on PATH, ignore RUSTUP_TOOLCHAIN entirely, and build with a toolchain
            nobody asked for.

        Neither change is written to the User or Machine environment, no shell profile
        is touched, and both disappear when the process exits.
    .OUTPUTS
        [pscustomobject] Toolchain, CargoBin, PinnedPath (bool), Detail.
    #>
    [CmdletBinding()]
    param([string]$Toolchain)
    if ([string]::IsNullOrWhiteSpace($Toolchain)) {
        return [pscustomobject]@{ Toolchain = ''; CargoBin = $null; PinnedPath = $false; Detail = 'no toolchain requested: whatever PATH resolves to will be used' }
    }
    $env:RUSTUP_TOOLCHAIN = $Toolchain
    $cargoBin = Get-CargoBinDirectory
    if (-not $cargoBin -or -not (Test-Path -LiteralPath $cargoBin -PathType Container)) {
        return [pscustomobject]@{
            Toolchain  = $Toolchain
            CargoBin   = $cargoBin
            PinnedPath = $false
            Detail     = "RUSTUP_TOOLCHAIN=$Toolchain is set, but rustup's directory ($cargoBin) does not exist, so the cargo a child process finds cannot be confirmed as the rustup proxy"
        }
    }
    $separator = [System.IO.Path]::PathSeparator
    $trimmed = $cargoBin.TrimEnd([char]'\', [char]'/')
    $rest = @($env:PATH -split [regex]::Escape($separator)) | Where-Object {
        $_ -and ($_.TrimEnd([char]'\', [char]'/') -ne $trimmed)
    }
    $env:PATH = (@($cargoBin) + $rest) -join $separator
    return [pscustomobject]@{
        Toolchain  = $Toolchain
        CargoBin   = $cargoBin
        PinnedPath = $true
        Detail     = "RUSTUP_TOOLCHAIN=$Toolchain and $cargoBin moved to the front of PATH, in this process only"
    }
}

function Get-RustCommand {
    <#
    .SYNOPSIS
        The command line to run a Rust tool with the requested toolchain.
    .DESCRIPTION
        With -Toolchain, Rust tools go through `rustup run <name> <tool>`, which is
        exact regardless of what PATH resolves to. `rustup` itself is never wrapped
        that way (there is no `rustup run <name> rustup`). RUSTUP_TOOLCHAIN is set as
        well — see Set-ToolchainScope — because that is the only mechanism that
        reaches a tool this script does not launch itself, such as the cargo inside
        `npx tauri build`.
    .OUTPUTS
        [pscustomobject] FilePath, Arguments, Display.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Tool,
        [string]$Toolchain = '',
        [string[]]$Arguments = @()
    )
    if ($Toolchain -and $Tool -ne 'rustup') {
        $rustup = Get-ToolPath -Name 'rustup'
        return [pscustomobject]@{
            FilePath  = $rustup
            Arguments = @('run', $Toolchain, $Tool) + $Arguments
            Display   = "rustup run $Toolchain $Tool $($Arguments -join ' ')".Trim()
        }
    }
    $path = Get-ToolPath -Name $Tool
    return [pscustomobject]@{
        FilePath  = $path
        Arguments = $Arguments
        Display   = "$Tool $($Arguments -join ' ')".Trim()
    }
}

function Get-PowerShellExecutable {
    <#
    .SYNOPSIS
        The absolute path of the PowerShell that is running this script.
    .DESCRIPTION
        Used to run the sibling entry points as their own process, so their exit code
        is unambiguous and an `exit` inside them cannot terminate the caller.
    #>
    [CmdletBinding()]
    param()
    $name = if ($IsWindows) { 'pwsh.exe' } else { 'pwsh' }
    $candidate = Join-Path $PSHOME $name
    if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    $process = Get-Process -Id $PID -ErrorAction SilentlyContinue
    if ($process -and $process.Path) { return $process.Path }
    throw "cannot locate the PowerShell executable (looked for $candidate). Run these scripts with PowerShell 7."
}

function Invoke-NativeCommand {
    <#
    .SYNOPSIS
        Runs one native command in an explicit working directory and reports its exit code.
    .DESCRIPTION
        `$ErrorActionPreference = 'Stop'` does not apply to native commands: a program
        that exits 3 does not throw, it just exits 3, and the failure is invisible
        unless $LASTEXITCODE is read immediately afterwards. Every native call in these
        scripts goes through this function, which reads $LASTEXITCODE itself (so it
        cannot be lost to an intervening command) and can be asked to throw on a
        non-zero exit.
        The tool is invoked by absolute path and the working directory is mandatory:
        nothing is left to the caller's current directory.
    .OUTPUTS
        [pscustomobject] FilePath, Arguments, ExitCode, Output (stdout and stderr merged).
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [string[]]$Arguments = @(),
        [Parameter(Mandatory)][string]$WorkingDirectory,
        [switch]$FailOnError,
        [string]$What = ''
    )
    if (-not (Test-Path -LiteralPath $WorkingDirectory -PathType Container)) {
        throw "the working directory '$WorkingDirectory' does not exist"
    }
    if (-not (Test-Path -LiteralPath $FilePath -PathType Leaf)) {
        throw "'$FilePath' does not exist"
    }
    Push-Location -LiteralPath $WorkingDirectory
    try {
        $output = & $FilePath @Arguments 2>&1 | Out-String
        $code = $LASTEXITCODE
    } finally {
        Pop-Location
    }
    if ($null -eq $code) { $code = 0 }
    $result = [pscustomobject]@{
        FilePath  = $FilePath
        Arguments = $Arguments
        ExitCode  = [int]$code
        Output    = ($output -replace "`r`n", "`n").Trim()
    }
    if ($FailOnError -and $result.ExitCode -ne 0) {
        $label = if ($What) { $What } else { Split-Path -Leaf $FilePath }
        throw ("{0} failed with exit code {1}: {2} {3}`n{4}" -f $label, $result.ExitCode, $FilePath, ($Arguments -join ' '), $result.Output)
    }
    return $result
}

function Invoke-PowerShellScript {
    <#
    .SYNOPSIS
        Runs a sibling entry point as its own PowerShell process and captures its output.
    .DESCRIPTION
        The pre-check and the package verifier both end with `exit <code>`. Running
        them as separate processes makes that exit code unambiguous and keeps their
        `exit` from terminating the caller. Environment (including RUSTUP_TOOLCHAIN
        and PATH) is inherited, so a pinned toolchain reaches them too.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Path,
        [string[]]$Arguments = @(),
        [Parameter(Mandatory)][string]$WorkingDirectory,
        [string]$LogPath = ''
    )
    $pwsh = Get-PowerShellExecutable
    $argumentList = @('-NoProfile', '-NonInteractive', '-File', $Path) + $Arguments
    $started = Get-Date
    $result = Invoke-NativeCommand -FilePath $pwsh -Arguments $argumentList -WorkingDirectory $WorkingDirectory
    if ($LogPath) {
        $header = @(
            "command  : $pwsh $($argumentList -join ' ')",
            "cwd      : $WorkingDirectory",
            "started  : $($started.ToString('u'))",
            "exit code: $($result.ExitCode)",
            ''
        )
        Set-Content -LiteralPath $LogPath -Value ($header + $result.Output)
    }
    return $result
}

function Get-DriveQualifier {
    <#
    .SYNOPSIS
        The drive qualifier ('C:') of a Windows path, or $null when there is none.
    .DESCRIPTION
        Pure string work, deliberately: the old disk-space check took the first three
        characters of a path and called them a drive, which turns `/Users/keychron/x`
        into the drive `/Us`. On Windows that never showed up; anywhere else it was one
        step away from a check that reports free space for a drive that does not exist.
        A qualifier is only returned for a letter followed by a colon and a separator,
        so a POSIX path or a UNC path (\\server\share) yields $null and the caller says
        so instead of guessing.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Path)
    if ($Path -match '^([A-Za-z]):[\\/]') { return ($Matches[1].ToUpperInvariant() + ':') }
    return $null
}

function Test-WritableDirectory {
    <#
    .SYNOPSIS
        Proves a directory can be written to, without leaving anything behind.
    .DESCRIPTION
        Creates the directory if it does not exist, writes one probe file inside it,
        deletes the probe file, and removes the directory again if this call created
        it. The probe is written where the build will actually write — never inside the
        package, where an interrupted run would leave a file the manifest does not
        describe.
    .OUTPUTS
        [pscustomobject] Path, Ok, Detail, CreatedDirectory.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Path)
    $created = $false
    $probe = Join-Path $Path ('.write-probe-{0}.tmp' -f [guid]::NewGuid().ToString('N').Substring(0, 8))
    $ok = $false
    $detail = ''
    try {
        if (-not (Test-Path -LiteralPath $Path)) {
            New-Item -ItemType Directory -Path $Path -Force | Out-Null
            $created = $true
        }
        Set-Content -LiteralPath $probe -Value 'probe' -NoNewline
        Remove-Item -LiteralPath $probe -Force
        $ok = $true
        $detail = "a probe file was created and deleted in $Path"
        if ($created) { $detail += ' (the directory itself was created by this check and removed again)' }
    } catch {
        $detail = "cannot write to ${Path}: $($_.Exception.Message)"
    } finally {
        if (Test-Path -LiteralPath $probe) { Remove-Item -LiteralPath $probe -Force -ErrorAction SilentlyContinue }
        if ($created -and (Test-Path -LiteralPath $Path)) {
            $left = @(Get-ChildItem -LiteralPath $Path -Force -ErrorAction SilentlyContinue)
            if ($left.Count -eq 0) { Remove-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue }
        }
    }
    return [pscustomobject]@{ Path = $Path; Ok = $ok; Detail = $detail; CreatedDirectory = $created }
}

function Get-FileSha256 {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-RelativeFileList {
    <#
    .SYNOPSIS
        Every file under a root, as forward-slash paths relative to that root.
    .DESCRIPTION
        `-Force` so hidden files (`.gitignore`, `.github`, `.cargo/config.toml`) are
        included: a listing that silently skips them would make a completeness check
        agree with an incomplete build.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Root)
    $full = (Resolve-Path -LiteralPath $Root).Path.TrimEnd([char]'\', [char]'/')
    $prefix = $full + [System.IO.Path]::DirectorySeparatorChar
    $files = Get-ChildItem -LiteralPath $full -Recurse -File -Force | ForEach-Object {
        if ($_.FullName.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            ($_.FullName.Substring($prefix.Length)) -replace '\\', '/'
        } else {
            $_.FullName
        }
    }
    return @($files)
}

function Compare-FileLists {
    <#
    .SYNOPSIS
        Gained and removed entries between two relative-path listings.
    #>
    [CmdletBinding()]
    param([string[]]$Before = @(), [string[]]$After = @())
    $comparer = [System.StringComparer]::OrdinalIgnoreCase
    $beforeSet = [System.Collections.Generic.HashSet[string]]::new([string[]]$Before, $comparer)
    $afterSet = [System.Collections.Generic.HashSet[string]]::new([string[]]$After, $comparer)
    $gained = New-Object System.Collections.Generic.List[string]
    $removed = New-Object System.Collections.Generic.List[string]
    foreach ($entry in $After) { if (-not $beforeSet.Contains($entry)) { $gained.Add($entry) } }
    foreach ($entry in $Before) { if (-not $afterSet.Contains($entry)) { $removed.Add($entry) } }
    return [pscustomobject]@{ Gained = @($gained); Removed = @($removed) }
}

function Copy-PackageToDirectory {
    <#
    .SYNOPSIS
        Copies every file of the package into a work directory, and proves it did.
    .DESCRIPTION
        File by file, with an explicit destination directory for each one, rather than
        a recursive `Copy-Item <dir>\*`: wildcard expansion is the one part of this
        operation whose behaviour differs between platforms and hidden files, and an
        incomplete copy would make every later step evidence about the wrong tree.
        The copy is then checked twice — the file count, and each file's SHA-256 against
        its source — and any discrepancy throws with the file name.
    .OUTPUTS
        [pscustomobject] Source, Destination, FileCount.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$Destination,
        [switch]$VerifyHashes
    )
    $sourceFull = (Resolve-Path -LiteralPath $Source).Path.TrimEnd([char]'\', [char]'/')
    $sourceFiles = @(Get-ChildItem -LiteralPath $sourceFull -Recurse -File -Force)
    if (-not (Test-Path -LiteralPath $Destination)) {
        New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    }
    $destinationFull = (Resolve-Path -LiteralPath $Destination).Path.TrimEnd([char]'\', [char]'/')
    $prefix = $sourceFull + [System.IO.Path]::DirectorySeparatorChar
    foreach ($file in $sourceFiles) {
        if (-not $file.FullName.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "unexpected file outside the source root: $($file.FullName)"
        }
        $relative = $file.FullName.Substring($prefix.Length)
        $target = Join-Path $destinationFull $relative
        $parent = Split-Path -Parent $target
        if (-not (Test-Path -LiteralPath $parent)) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
        Copy-Item -LiteralPath $file.FullName -Destination $target -Force
        if ($VerifyHashes) {
            $sourceHash = Get-FileSha256 -Path $file.FullName
            $copyHash = Get-FileSha256 -Path $target
            if ($sourceHash -ne $copyHash) {
                throw "the copy of '$relative' does not match its source ($sourceHash vs $copyHash): the work directory does not hold what the package holds"
            }
        }
    }
    $copied = @(Get-ChildItem -LiteralPath $destinationFull -Recurse -File -Force)
    if ($copied.Count -ne $sourceFiles.Count) {
        throw "the copy is incomplete: the package holds $($sourceFiles.Count) file(s) and the copy holds $($copied.Count)"
    }
    return [pscustomobject]@{ Source = $sourceFull; Destination = $destinationFull; FileCount = $copied.Count }
}

function Get-PathRelativeTo {
    <#
    .SYNOPSIS
        A path relative to a base directory, using forward slashes; used only in messages.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$Base)
    $baseFull = (Resolve-Path -LiteralPath $Base).Path.TrimEnd([char]'\', [char]'/')
    if ($Path.StartsWith($baseFull + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
        return ($Path.Substring($baseFull.Length + 1)) -replace '\\', '/'
    }
    return $Path
}
