# Fixture tests for scripts/release/package-windows.ps1.
#
#   pwsh -NoProfile -File scripts/release/test-packaging.ps1
#
# No Windows host, no real Windows build and no network are involved: the script
# under test is copied into a throwaway git repository together with fake
# cargo/rustc doubles, a fake PE binary and a fake NSIS installer, and is then
# run there as a child process exactly as the release workflow runs it.
#
# Two defects this protects against, both of which were published in v0.1.0 and
# v0.1.1:
#   * SHA256SUMS was written with the host newline, so the released list used
#     CRLF and `shasum -c SHA256SUMS` reported every entry as a missing file on
#     macOS and Linux.
#   * install.ps1 was copied out of a Windows checkout, so the published script
#     differed byte-for-byte from the committed one (CRLF instead of LF) and its
#     digest could not be reproduced from the repository.
#
# This is not evidence that a Windows build succeeds: it proves the packaging
# steps and their guards behave, nothing more.

[CmdletBinding()]
param([switch]$KeepTemp, [string]$Filter = '')

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$onWindows = [Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT
$script:Pwsh = Join-Path $PSHOME $(if ($onWindows) { 'pwsh.exe' } else { 'pwsh' })
if (-not (Test-Path -LiteralPath $script:Pwsh -PathType Leaf)) {
    $script:Pwsh = if ($onWindows) { 'powershell.exe' } else { 'pwsh' }
}

$script:Passed = 0
$script:Failed = 0
$script:Root = Join-Path ([IO.Path]::GetTempPath()) ('ohm-release-packaging-' + [Guid]::NewGuid().ToString('N'))
$script:Bin = Join-Path $script:Root 'bin'
$script:Cases = 0

function Require([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Require-Contains([string]$Text, [string]$Expected) {
    # The child's console wraps long lines, so compare with whitespace collapsed.
    $flatText = ($Text -replace '\s+', ' ')
    $flatExpected = ($Expected -replace '\s+', ' ')
    Require ($flatText -like "*$flatExpected*") "expected the output to mention '$Expected'; got:`n$Text"
}

function Test-Case([string]$Name, [scriptblock]$Action) {
    if ($Filter -and $Name -notlike "*$Filter*") { return }
    $script:Cases += 1
    Write-Host ''
    Write-Host "--- $Name"
    try {
        & $Action
        $script:Passed += 1
        Write-Host "    [PASS] $Name"
    } catch {
        $script:Failed += 1
        Write-Host "    [FAIL] $Name"
        Write-Host "           $($_.Exception.Message)"
    }
}

function Set-UnixExecutable([string]$Path) {
    if ($onWindows) { return }
    & chmod '+x' $Path
    if ($LASTEXITCODE -ne 0) { throw "chmod failed for $Path" }
}

# A double is a .cmd on Windows (that is what a PATH lookup finds there) and an
# executable shell script elsewhere; both dispatch to the same PowerShell body.
function New-Double([string]$Name, [string]$Body) {
    $impl = Join-Path $script:Bin "$Name.ps1"
    Set-Content -LiteralPath $impl -Value $Body -Encoding utf8
    if ($onWindows) {
        $wrapper = Join-Path $script:Bin "$Name.cmd"
        Set-Content -LiteralPath $wrapper -Encoding ascii -Value @(
            '@echo off',
            "`"$($script:Pwsh)`" -NoProfile -File `"$impl`" %*"
        )
    } else {
        $wrapper = Join-Path $script:Bin $Name
        Set-Content -LiteralPath $wrapper -Encoding utf8 -Value @(
            '#!/bin/sh',
            "exec `"$($script:Pwsh)`" -NoProfile -File `"$impl`" `"`$@`""
        )
        Set-UnixExecutable $wrapper
    }
}

function New-FakePe([string]$Path) {
    $bytes = [byte[]]::new(512)
    $bytes[0] = 0x4D; $bytes[1] = 0x5A
    $pe = 128
    [BitConverter]::GetBytes([int]$pe).CopyTo($bytes, 60)
    $bytes[$pe] = 0x50; $bytes[$pe + 1] = 0x45
    [BitConverter]::GetBytes([uint16]0x8664).CopyTo($bytes, $pe + 4)
    [IO.File]::WriteAllBytes($Path, $bytes)
}

function New-FixtureRepo {
    $repo = Join-Path $script:Root ('repo-' + [Guid]::NewGuid().ToString('N'))
    $release = Join-Path $repo 'scripts/release'
    $bundle = Join-Path $repo 'target/x86_64-pc-windows-msvc/release/bundle/nsis'
    $null = New-Item (Join-Path $repo 'artifacts'), $release, $bundle -ItemType Directory -Force
    Copy-Item (Join-Path $PSScriptRoot 'package-windows.ps1') (Join-Path $release 'package-windows.ps1')
    Set-Content -LiteralPath (Join-Path $repo 'Cargo.toml') -Encoding utf8 -Value @(
        '[workspace]', 'members = []'
    )
    Set-Content -LiteralPath (Join-Path $repo 'LICENSE') -Encoding utf8 -Value 'Apache License 2.0 fixture'
    Set-Content -LiteralPath (Join-Path $repo 'artifacts/THIRD_PARTY_NOTICES.txt') -Encoding utf8 -Value 'notices fixture'
    # The committed installer script is LF, exactly like the real repository.
    [IO.File]::WriteAllText((Join-Path $repo 'scripts/install.ps1'),
        "# installer fixture`nparam([string]`$Version)`nWrite-Host `$Version`n", [Text.UTF8Encoding]::new($false))
    New-FakePe (Join-Path $repo 'target/x86_64-pc-windows-msvc/release/ohm-cli.exe')
    Set-Content -LiteralPath (Join-Path $bundle 'OpenHardwareOS_0.1.2_x64-setup.exe') -Encoding utf8 -Value 'nsis fixture'
    & git -C $repo init --quiet
    & git -C $repo config user.email 'fixture@example.invalid'
    & git -C $repo config user.name 'Fixture'
    # A fixture must not inherit the host's end-of-line conversion: the point of
    # the tests is to control it explicitly per case.
    & git -C $repo config core.autocrlf false
    & git -C $repo add -A
    & git -C $repo commit --quiet -m 'fixture'
    if ($LASTEXITCODE -ne 0) { throw 'Cannot commit the fixture repository' }
    return $repo
}

function Invoke-Packaging([string]$Repo) {
    $script = Join-Path $Repo 'scripts/release/package-windows.ps1'
    $env:OHM_FAKE_TARGET_DIR = (Join-Path $Repo 'target')
    # The failing cases are expected to write to stderr; that must not abort the
    # harness (a native command's stderr is not this process's error).
    $prior = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $lines = & $script:Pwsh -NoProfile -File $script -Version 'v0.1.2' 2>&1
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $prior
    }
    return @{ ExitCode = $code; Output = ($lines | Out-String) }
}

function Get-BlobPath([string]$Repo, [string]$Relative) {
    $blob = (& git -C $Repo rev-parse "HEAD:$Relative").Trim()
    if ($LASTEXITCODE -ne 0) { throw "cannot read the committed blob for $Relative" }
    return $blob
}

function Get-FileBlobHash([string]$Path) {
    $hash = (& git hash-object -- $Path).Trim()
    if ($LASTEXITCODE -ne 0) { throw "cannot hash $Path" }
    return $hash
}

function Assert-NoCarriageReturn([string]$Path) {
    $text = [IO.File]::ReadAllText($Path)
    Require (-not $text.Contains("`r")) "$([IO.Path]::GetFileName($Path)) contains a carriage return; POSIX tooling cannot read it"
}

# --------------------------------------------------------------------------
# doubles
# --------------------------------------------------------------------------
$null = New-Item $script:Bin -ItemType Directory -Force
New-Double 'cargo' @'
$ErrorActionPreference = 'Stop'
if ($args.Count -gt 0 -and $args[0] -eq 'metadata') {
    $target = ($env:OHM_FAKE_TARGET_DIR -replace '\\', '/')
    Write-Output ('{"target_directory":"' + $target + '"}')
    exit 0
}
Write-Error "fake cargo: unexpected arguments: $($args -join ' ')"
exit 1
'@
New-Double 'rustc' @'
$ErrorActionPreference = 'Stop'
if ($args.Count -gt 0 -and $args[0] -eq '--version') { Write-Output 'rustc 1.98.1 (fixture)'; exit 0 }
Write-Error "fake rustc: unexpected arguments: $($args -join ' ')"
exit 1
'@
$env:PATH = "$($script:Bin)$([IO.Path]::PathSeparator)$env:PATH"

Write-Host '================================================================================'
Write-Host ' OpenHardwareOS - release packaging tests (FIXTURES ONLY)'
Write-Host '================================================================================'
Write-Host " host            : $([Environment]::OSVersion.VersionString)"
Write-Host " child shell     : $($script:Pwsh)"
Write-Host " temporary root  : $($script:Root)"
Write-Host ''
Write-Host ' These fixtures prove the packaging steps and guards; they are NOT evidence'
Write-Host ' that a Windows build succeeds. That needs a real Windows machine.'

try {
    Test-Case 'refuses to package over an existing release output' {
        $repo = New-FixtureRepo
        $null = New-Item (Join-Path $repo 'artifacts/release') -ItemType Directory -Force
        $run = Invoke-Packaging $repo
        Require ($run.ExitCode -ne 0) 'packaging must refuse an existing output directory'
        Require-Contains $run.Output 'Output already exists'
    }

    Test-Case 'publishes LF checksums, all assets, and the committed installer script' {
        $repo = New-FixtureRepo
        $run = Invoke-Packaging $repo
        Require ($run.ExitCode -eq 0) "packaging failed:`n$($run.Output)"
        $output = Join-Path $repo 'artifacts/release'
        $names = @(Get-ChildItem $output -File | ForEach-Object Name | Sort-Object)
        $expected = @(
            'LICENSE', 'OpenHardwareOS-v0.1.2-windows-x86_64-setup.exe', 'SHA256SUMS',
            'THIRD_PARTY_NOTICES.txt', 'install.ps1', 'ohm-cli-v0.1.2-windows-x86_64.zip', 'release.json'
        )
        Require ((($names | Sort-Object) -join ',') -ceq (($expected | Sort-Object) -join ',')) "unexpected asset set: $($names -join ',')"
        $sums = Join-Path $output 'SHA256SUMS'
        Assert-NoCarriageReturn $sums
        # Every entry must name a real file and carry that file's real digest.
        $entries = @(Get-Content -LiteralPath $sums | Where-Object { $_ -ne '' })
        Require ($entries.Count -eq 6) "expected six checksummed assets, found $($entries.Count)"
        foreach ($entry in $entries) {
            Require ($entry -match '^([0-9a-f]{64})  (.+)$') "malformed checksum line: $entry"
            $digest = $Matches[1]
            $name = $Matches[2]
            $file = Join-Path $output $name
            Require (Test-Path -LiteralPath $file -PathType Leaf) "checksummed file is absent: $name"
            $actual = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
            Require ($actual -eq $digest) "digest mismatch for $name"
        }
        Require (@($entries | Where-Object { $_ -match 'SHA256SUMS' }).Count -eq 0) 'the list must not try to checksum itself'
        # The published script must be the committed script.
        $blob = Get-BlobPath $repo 'scripts/install.ps1'
        Require ((Get-FileBlobHash (Join-Path $output 'install.ps1')) -ceq $blob) 'published install.ps1 is not the committed blob'
        Require-Contains $run.Output 'SHA256SUMS uses LF'
    }

    Test-Case 'a CRLF checkout still publishes the committed script' {
        $repo = New-FixtureRepo
        # Exactly what a Windows checkout with core.autocrlf=true hands out: CRLF
        # in the working tree, LF in the blob, nothing committed.
        $path = Join-Path $repo 'scripts/install.ps1'
        [IO.File]::WriteAllText($path, ([IO.File]::ReadAllText($path) -replace "`n", "`r`n"), [Text.UTF8Encoding]::new($false))
        Require ([IO.File]::ReadAllText($path).Contains("`r")) 'the fixture did not get a CRLF working tree'
        Require ((& git -C $repo status --porcelain).Trim() -cne '') 'the CRLF worktree should differ from the commit'
        $committed = (& git -C $repo cat-file blob 'HEAD:scripts/install.ps1' | Out-String)
        Require (-not $committed.Contains("`r")) 'the committed blob should still be LF'
        $run = Invoke-Packaging $repo
        Require ($run.ExitCode -eq 0) "packaging failed:`n$($run.Output)"
        $published = Join-Path $repo 'artifacts/release/install.ps1'
        Assert-NoCarriageReturn $published
        Require ((Get-FileBlobHash $published) -ceq (Get-BlobPath $repo 'scripts/install.ps1')) 'a CRLF checkout changed the published script'
    }

    Test-Case 'a genuinely CRLF committed script fails loudly instead of publishing' {
        $repo = New-FixtureRepo
        $path = Join-Path $repo 'scripts/install.ps1'
        [IO.File]::WriteAllText($path, ([IO.File]::ReadAllText($path) -replace "`n", "`r`n"), [Text.UTF8Encoding]::new($false))
        & git -C $repo add -A
        & git -C $repo commit --quiet -m 'commit a CRLF installer script'
        if ($LASTEXITCODE -ne 0) { throw 'cannot commit the CRLF fixture' }
        $run = Invoke-Packaging $repo
        Require ($run.ExitCode -ne 0) 'a CRLF blob must not be published silently'
        # The console wraps this message mid-phrase, so assert on the part that
        # always stays on one line.
        Require-Contains $run.Output 'Published install.ps1 ('
        Require (-not (Test-Path -LiteralPath (Join-Path $repo 'artifacts/release/SHA256SUMS'))) 'no release list may be produced for a refused package'
    }

    Test-Case 'the carriage-return guard is real' {
        $repo = New-FixtureRepo
        # Simulate the shipped defect while keeping the guard: the list is written
        # with the host newline again.
        $scriptPath = Join-Path $repo 'scripts/release/package-windows.ps1'
        $text = [IO.File]::ReadAllText($scriptPath)
        $patched = $text.Replace('(($sums -join "`n") + "`n")', '(($sums -join "`r`n") + "`r`n")')
        Require ($patched -cne $text) 'could not patch the fixture copy; the guard moved'
        [IO.File]::WriteAllText($scriptPath, $patched, [Text.UTF8Encoding]::new($false))
        $run = Invoke-Packaging $repo
        Require ($run.ExitCode -ne 0) 'the CRLF guard did not fail the run'
        Require-Contains $run.Output 'carriage return'
    }

    Test-Case 'the installer-blob guard is real' {
        $repo = New-FixtureRepo
        # Keep the normalisation, drop the comparison: the run must still fail,
        # because a missing guard would publish whatever the worktree held.
        $scriptPath = Join-Path $repo 'scripts/release/package-windows.ps1'
        $text = [IO.File]::ReadAllText($scriptPath)
        $patched = $text.Replace('if ($published -cne $blob) {', 'if ($false) {')
        Require ($patched -cne $text) 'could not patch the fixture copy; the comparison moved'
        [IO.File]::WriteAllText($scriptPath, $patched, [Text.UTF8Encoding]::new($false))
        # Commit a CRLF script: without the comparison the run would now succeed
        # and publish something that does not match the blob.
        $installer = Join-Path $repo 'scripts/install.ps1'
        [IO.File]::WriteAllText($installer, ([IO.File]::ReadAllText($installer) -replace "`n", "`r`n"), [Text.UTF8Encoding]::new($false))
        & git -C $repo add -A
        & git -C $repo commit --quiet -m 'CRLF installer script'
        if ($LASTEXITCODE -ne 0) { throw 'cannot commit the CRLF fixture' }
        $run = Invoke-Packaging $repo
        # With the guard removed the packaging succeeds, which is exactly why the
        # guard exists: this case asserts the patched script succeeds, so a future
        # removal of the guard is visible as a failure of the real case above.
        Require ($run.ExitCode -eq 0) "the patched script should have succeeded; the case is not measuring what it claims:`n$($run.Output)"
        Require ((Get-FileBlobHash (Join-Path $repo 'artifacts/release/install.ps1')) -cne (Get-BlobPath $repo 'scripts/install.ps1')) 'the patched script unexpectedly produced a matching file'
    }
} finally {
    if ($KeepTemp) {
        Write-Host ''
        Write-Host "kept: $($script:Root)"
    } else {
        Remove-Item -LiteralPath $script:Root -Recurse -Force -ErrorAction SilentlyContinue
        Write-Host ''
        Write-Host 'Temporary fixtures removed.'
    }
}

Write-Host ''
Write-Host '================================================================================'
Write-Host ' RELEASE PACKAGING TEST SUMMARY'
Write-Host '================================================================================'
Write-Host "  cases: $($script:Cases)   passed: $($script:Passed)   failed: $($script:Failed)"
if ($script:Failed -eq 0) {
    Write-Host 'RESULT: PASS'
    exit 0
}
Write-Host 'RESULT: FAIL'
exit 1
