<#
.SYNOPSIS
    Verifies every file in this package against MANIFEST.sha256.

.DESCRIPTION
    Run this before trusting anything in the package. It answers exactly one
    question — is this the source tree the manifest describes? — and it answers
    "no" loudly: a missing file, an extra file, or a single changed byte all fail.

.PARAMETER Quiet
    Print only a one-line summary. Used by the pre-check.

.EXAMPLE
    PS> .\scripts\verify-package.ps1
#>
#Requires -Version 7.0
[CmdletBinding()]
param([switch]$Quiet)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$manifest = Join-Path $root 'MANIFEST.sha256'

if (-not (Test-Path -LiteralPath $manifest)) {
    Write-Host "FAIL: MANIFEST.sha256 not found in $root" -ForegroundColor Red
    exit 1
}

$problems = New-Object System.Collections.Generic.List[string]
$expected = @{}
$checked = 0

foreach ($line in Get-Content -LiteralPath $manifest) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    # Format: <64 hex chars><two spaces><relative path with forward slashes>
    if ($line -notmatch '^([0-9a-f]{64})  (.+)$') {
        $problems.Add("unparseable manifest line: $line")
        continue
    }
    $hash = $Matches[1]
    $relative = $Matches[2]
    $expected[$relative] = $hash
    $path = Join-Path $root ($relative -replace '/', [System.IO.Path]::DirectorySeparatorChar)
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        $problems.Add("missing: $relative")
        continue
    }
    $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    $checked++
    if ($actual -ne $hash) {
        $problems.Add("changed: $relative`n    expected $hash`n    actual   $actual")
    }
}

# Anything present that the manifest does not describe is also a finding: an extra
# file is either a mistake or something added after packaging.
$onDisk = Get-ChildItem -LiteralPath $root -Recurse -File |
    ForEach-Object { $_.FullName.Substring($root.Length + 1) -replace '\\', '/' } |
    Where-Object { $_ -ne 'MANIFEST.sha256' -and $_ -notlike '.precheck-*' }
foreach ($file in $onDisk) {
    if (-not $expected.ContainsKey($file)) { $problems.Add("not in the manifest: $file") }
}

if ($problems.Count -gt 0) {
    if ($Quiet) {
        Write-Output ("{0} problem(s): {1}" -f $problems.Count, ($problems -join '; '))
    } else {
        Write-Host ''
        Write-Host ("PACKAGE VERIFICATION FAILED — {0} problem(s) against {1} checked file(s):" -f $problems.Count, $checked) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "  * $problem" -ForegroundColor Red }
        Write-Host ''
        Write-Host 'Do not use this package for acceptance evidence: it is not the tree the manifest describes.'
    }
    exit 1
}

if ($Quiet) {
    Write-Output ("{0} file(s) verified" -f $checked)
} else {
    Write-Host ("PACKAGE VERIFIED — {0} file(s) match MANIFEST.sha256" -f $checked) -ForegroundColor Green
}
exit 0
