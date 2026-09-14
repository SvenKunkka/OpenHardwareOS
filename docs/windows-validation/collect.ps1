<#
.SYNOPSIS
    OpenHardwareOS — read-only Windows evidence collector.

.DESCRIPTION
    Gathers environment and application evidence for a Windows real-hardware
    validation session into one timestamped bundle folder.

    THIS SCRIPT IS READ-ONLY WITH RESPECT TO THE MACHINE:

      * it never writes a value to hardware — no fan, no pump, no GPU, no disk;
      * it never writes to the registry (it READS one value: the per-user Run
        entry for OpenHardwareOS) and never exports, imports or backs up a hive;
      * it never starts, stops, creates or deletes a service, scheduled task or
        driver;
      * it never reads private files: the only files it reads are inside the
        application's own config directory (%APPDATA%\OpenHardwareOS by default,
        or $env:OHM_CONFIG_DIR when set);
      * the only things it creates are the bundle folder and the files in it.

    TWO DISCLOSURES, stated here and in docs/windows-validation/README.md:

      1. This script has never been executed on Windows by the authors — it was
         written on macOS. It is therefore "Prepared", not "Build/run passed".
         Review it by eye first, and run it with -DryRun to see the plan without
         writing anything.
      2. It invokes the application's OWN built binary (ohm-cli doctor / audit)
         when one exists. That binary may create the config directory and append
         a lifecycle entry to its own audit.jsonl. That is the application
         writing its own state in its own directory — never hardware, never the
         registry — but it is a write, so it is disclosed rather than hidden.

    Compatibility: Windows PowerShell 5.1 and PowerShell 7, no external modules.

.PARAMETER OutputRoot
    Directory the timestamped bundle folder is created in. Defaults to the
    current directory.

.PARAMETER DryRun
    Print what would be collected and where, write nothing at all, and exit.

.PARAMETER AuditTailLines
    How many trailing lines of the application's own audit.jsonl to copy.
    Default 40.

.EXAMPLE
    PS> .\collect.ps1 -DryRun
    PS> .\collect.ps1
    PS> .\collect.ps1 -OutputRoot D:\evidence -AuditTailLines 100
#>

[CmdletBinding()]
param(
    [string]$OutputRoot = (Get-Location).Path,
    [switch]$DryRun,
    [int]$AuditTailLines = 40
)

$ErrorActionPreference = 'Continue'

# Refuse to write inside an assembled acceptance package.
#
# The default output root is the current directory, so running this from a package's
# root — the obvious thing to do, and what the quick start used to say — writes the
# evidence bundle *inside* the package. The package's own manifest then no longer
# matches its contents, and every later `verify-package` or `build` refuses to run: the
# collector breaks the package it is collecting evidence for. A package is recognisable
# by the MANIFEST.sha256 at its root, so that is what is checked, for the resolved output
# root and for every directory above it.
function Get-PackageRootAbove {
    param([string]$Path)
    try {
        $current = [System.IO.Path]::GetFullPath($Path)
    } catch {
        return $null
    }
    while (-not [string]::IsNullOrWhiteSpace($current)) {
        if (Test-Path -LiteralPath (Join-Path $current 'MANIFEST.sha256') -PathType Leaf) {
            return $current
        }
        $parent = Split-Path -Parent $current
        if ($parent -eq $current) { break }
        $current = $parent
    }
    return $null
}

$script:PackageRoot = Get-PackageRootAbove -Path $OutputRoot
if ($null -ne $script:PackageRoot) {
    Write-Host ''
    Write-Host 'REFUSING TO COLLECT: the output folder is inside an assembled acceptance package.' -ForegroundColor Red
    Write-Host "  output root  : $OutputRoot"
    Write-Host "  package root : $script:PackageRoot"
    Write-Host ''
    Write-Host 'A bundle written into the package would stop it matching its own manifest, and'
    Write-Host 'every later verify-package.ps1 or build.ps1 would refuse to run. Give the'
    Write-Host 'collector somewhere outside the package to write, for example:'
    Write-Host ''
    Write-Host '  .\docs\windows-validation\collect.ps1 -OutputRoot "$env:USERPROFILE\OpenHardwareOS-evidence"'
    Write-Host ''
    Write-Host 'Nothing was collected and nothing was written.'
    exit 1
}

$script:Stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$script:Computer = $env:COMPUTERNAME
if ([string]::IsNullOrWhiteSpace($script:Computer)) { $script:Computer = 'unknown-host' }
$script:BundleName = "windows-validation-$($script:Computer)-$($script:Stamp)"
$script:BundlePath = Join-Path $OutputRoot $script:BundleName

# Repository root: this script lives in <root>\docs\windows-validation\.
$script:RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)

# ---------------------------------------------------------------------------
# Output buffer
# ---------------------------------------------------------------------------

$script:Lines = New-Object System.Collections.Generic.List[string]

function Add-Line {
    param([string]$Text = '')
    [void]$script:Lines.Add($Text)
    Write-Host $Text
}

function Add-Section {
    param([string]$Title)
    Add-Line ''
    Add-Line ('=' * 78)
    Add-Line $Title
    Add-Line ('=' * 78)
}

function Add-Failure {
    param([string]$What, [string]$Error)
    Add-Line "  [$What] NOT COLLECTED: $Error"
}

function Invoke-Collected {
    <# Run a block, record its output under a heading, never throw. #>
    param([string]$Heading, [scriptblock]$Body)
    Add-Section $Heading
    try {
        $result = & $Body
        if ($null -ne $result) {
            foreach ($item in @($result)) {
                if ($null -ne $item) { Add-Line ([string]$item) }
            }
        }
    } catch {
        Add-Failure $Heading $_.Exception.Message
    }
}

function Write-BundleFile {
    param([string]$Name, [System.Collections.Generic.List[string]]$Content)
    $path = Join-Path $script:BundlePath $Name
    $encoding = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllLines($path, $Content.ToArray(), $encoding)
    Add-Line "  wrote: $path"
}

# ---------------------------------------------------------------------------
# Collection helpers
# ---------------------------------------------------------------------------

function Get-CimStrings {
    param(
        [string]$ClassName,
        [string[]]$Properties,
        [string]$Namespace = 'root/cimv2'
    )
    $rows = Get-CimInstance -Namespace $Namespace -ClassName $ClassName -ErrorAction Stop
    $out = New-Object System.Collections.Generic.List[string]
    foreach ($row in @($rows)) {
        $parts = New-Object System.Collections.Generic.List[string]
        foreach ($prop in $Properties) {
            $value = $row.$prop
            if ($null -eq $value) { $value = '(null)' }
            [void]$parts.Add("$prop=$value")
        }
        [void]$out.Add('  ' + ($parts -join '  |  '))
    }
    if ($out.Count -eq 0) { [void]$out.Add('  (no instances returned)') }
    return $out
}

function Test-Elevated {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Get-ConfigRoot {
    if (-not [string]::IsNullOrWhiteSpace($env:OHM_CONFIG_DIR)) {
        return $env:OHM_CONFIG_DIR
    }
    if ([string]::IsNullOrWhiteSpace($env:APPDATA)) { return $null }
    return (Join-Path $env:APPDATA 'OpenHardwareOS')
}

function Get-JsonProbe {
    <# HTTP status and byte count only. Never dumps the tree. #>
    param([string]$Url)
    try {
        $response = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 5 -ErrorAction Stop
        $status = [int]$response.StatusCode
        $bytes = $response.RawContentLength
        if ($null -eq $bytes -or $bytes -lt 0) {
            if ($null -ne $response.Content) {
                $bytes = [Text.Encoding]::UTF8.GetByteCount([string]$response.Content)
            } else {
                $bytes = -1
            }
        }
        return "  HTTP status: $status   body bytes: $bytes   (tree not captured, by design)"
    } catch {
        $detail = $_.Exception.Message
        $status = $null
        if ($null -ne $_.Exception.Response -and $null -ne $_.Exception.Response.StatusCode) {
            try { $status = [int]$_.Exception.Response.StatusCode } catch { $status = $null }
        }
        if ($null -ne $status) {
            return "  HTTP status: $status   (error: $detail)"
        }
        return "  unreachable: $detail"
    }
}

function Get-WorkspaceTargetDir {
    # Every artefact in this repository lives in the *workspace* target directory:
    # `apps/desktop/src-tauri` is a member of the root Cargo workspace, so there is no
    # `apps\desktop\src-tauri\target` to look in. Ask cargo instead of assuming, so that a
    # `--target-dir` flag or a `build.target-dir` setting cannot hide the artefacts from
    # this collector; `<repo>\target` is used when cargo is not on PATH.
    #
    # `--manifest-path` is passed explicitly: without it cargo resolves the workspace from
    # the *current directory*, so running this collector from inside another Rust project
    # would report that project's target directory.
    $manifest = Join-Path $script:RepoRoot 'Cargo.toml'
    $cargo = Get-Command 'cargo' -ErrorAction SilentlyContinue
    if ($null -ne $cargo -and (Test-Path -LiteralPath $manifest -PathType Leaf)) {
        try {
            $json = (& $cargo.Source 'metadata' '--format-version' '1' '--no-deps' '--manifest-path' $manifest 2>$null) -join ''
            if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($json)) {
                $target = (ConvertFrom-Json $json).target_directory
                if (-not [string]::IsNullOrWhiteSpace($target)) { return [string]$target }
            }
        } catch {
            # fall through to the default below
        }
    }
    return (Join-Path $script:RepoRoot 'target')
}

function Get-CliCandidates {
    $targetRoot = Get-WorkspaceTargetDir
    $found = New-Object System.Collections.Generic.List[string]
    foreach ($profile in @('release', 'debug')) {
        $candidate = Join-Path $targetRoot (Join-Path $profile 'ohm-cli.exe')
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            [void]$found.Add($candidate)
        }
    }
    return $found
}

# ---------------------------------------------------------------------------
# The plan (also used by -DryRun)
# ---------------------------------------------------------------------------

$plan = @(
    'environment.txt  — OS build/version/arch, elevation, host and component models,'
    '                   NVIDIA driver version and nvml.dll presence, LibreHardwareMonitor'
    '                   process state, http://127.0.0.1:8085/data.json status + byte count,'
    '                   the per-user Run value for OpenHardwareOS (read only), and a listing'
    '                   of the application config directory.',
    'cli-doctor.txt   — output of `<repo>\target\{release|debug}\ohm-cli.exe doctor` when a'
    '                   built binary exists (skipped with a message when it does not).',
    'cli-audit.txt    — output of the same binary with `audit --limit 20`.',
    'audit-tail.jsonl — the last N lines of the application''s own audit.jsonl, copied'
    '                   verbatim, or a one line note when the file does not exist.'
)

if ($DryRun) {
    Write-Host ''
    Write-Host 'OpenHardwareOS evidence collector — DRY RUN (nothing will be written)'
    Write-Host ''
    Write-Host "Repository root : $script:RepoRoot"
    Write-Host "Bundle folder   : $script:BundlePath"
    Write-Host ''
    Write-Host 'Would collect:'
    foreach ($line in $plan) { Write-Host "  $line" }
    Write-Host ''
    Write-Host 'Would NOT do:'
    Write-Host '  * no hardware write of any kind (no fan, pump, GPU or disk value)'
    Write-Host '  * no registry write (one value is read: the per-user Run entry)'
    Write-Host '  * no service, scheduled task or driver is started, stopped or created'
    Write-Host '  * no file outside the application config directory is read'
    Write-Host ''
    Write-Host 'No bundle was written.'
    Write-Host ''
    exit 0
}

# ---------------------------------------------------------------------------
# Create the bundle
# ---------------------------------------------------------------------------

try {
    if (-not (Test-Path -LiteralPath $script:BundlePath)) {
        New-Item -ItemType Directory -Path $script:BundlePath -Force | Out-Null
    }
} catch {
    Write-Error "could not create the bundle folder '$($script:BundlePath)': $($_.Exception.Message)"
    exit 1
}

Add-Line ('=' * 78)
Add-Line 'OpenHardwareOS — Windows validation evidence bundle'
Add-Line ('=' * 78)
Add-Line "collected at : $(Get-Date -Format 'yyyy-MM-dd HH:mm:ssK')"
Add-Line "computer     : $($script:Computer)"
Add-Line "bundle       : $($script:BundlePath)"
Add-Line "repo root    : $($script:RepoRoot)"
Add-Line 'collector    : docs/windows-validation/collect.ps1 (read-only; see its header)'
Add-Line 'status       : Prepared — this collector has not been executed on Windows by'
Add-Line '               the authors. Review the output before treating it as evidence.'

# --- 1. Operating system ---------------------------------------------------

Invoke-Collected 'Operating system' {
    $os = Get-CimInstance -ClassName Win32_OperatingSystem -ErrorAction Stop
    $lines = New-Object System.Collections.Generic.List[string]
    [void]$lines.Add("  Caption        : $($os.Caption)")
    [void]$lines.Add("  Version        : $($os.Version)")
    [void]$lines.Add("  BuildNumber    : $($os.BuildNumber)")
    [void]$lines.Add("  OSArchitecture : $($os.OSArchitecture)")
    [void]$lines.Add("  InstallDate    : $($os.InstallDate)")
    [void]$lines.Add("  LastBootUpTime : $($os.LastBootUpTime)")
    [void]$lines.Add("  64-bit OS      : $([Environment]::Is64BitOperatingSystem)")
    [void]$lines.Add("  PROCESSOR_ARCHITECTURE : $($env:PROCESSOR_ARCHITECTURE)")
    [void]$lines.Add("  PowerShell     : $($PSVersionTable.PSVersion)  (edition: $($PSVersionTable.PSEdition))")
    [void]$lines.Add("  Culture        : $([System.Globalization.CultureInfo]::CurrentCulture.Name)")
    return $lines
}

# --- 2. Elevation ----------------------------------------------------------

Invoke-Collected 'Session privileges' {
    $elevated = Test-Elevated
    $lines = New-Object System.Collections.Generic.List[string]
    [void]$lines.Add("  running elevated : $elevated")
    if ($elevated) {
        [void]$lines.Add('  meaning: this shell can call NVML fan setters and can install the perMachine NSIS bundle.')
    } else {
        [void]$lines.Add('  meaning: NVML fan *writes* will be refused with permission_denied; reads and LHM')
        [void]$lines.Add('           writes are unaffected (LHM is the elevated process, not this one).')
    }
    return $lines
}

# --- 3. Host and component models -----------------------------------------

Invoke-Collected 'Computer system' {
    Get-CimStrings -ClassName 'Win32_ComputerSystem' -Properties @('Manufacturer', 'Model', 'SystemType', 'TotalPhysicalMemory')
}

Invoke-Collected 'Baseboard (motherboard — needed to interpret SuperIO fan channels)' {
    Get-CimStrings -ClassName 'Win32_BaseBoard' -Properties @('Manufacturer', 'Product', 'SerialNumber', 'Version')
}

Invoke-Collected 'CPU (Win32_Processor)' {
    Get-CimStrings -ClassName 'Win32_Processor' -Properties @('Name', 'NumberOfCores', 'NumberOfLogicalProcessors', 'MaxClockSpeed', 'Architecture')
}

Invoke-Collected 'GPU (Win32_VideoController)' {
    Get-CimStrings -ClassName 'Win32_VideoController' -Properties @('Name', 'DriverVersion', 'AdapterRAM', 'VideoProcessor')
}

Invoke-Collected 'BIOS' {
    Get-CimStrings -ClassName 'Win32_BIOS' -Properties @('Manufacturer', 'SMBIOSBIOSVersion', 'ReleaseDate')
}

Invoke-Collected 'Physical disks (Win32_DiskDrive)' {
    Get-CimStrings -ClassName 'Win32_DiskDrive' -Properties @('Model', 'InterfaceType', 'MediaType', 'Size', 'SerialNumber')
}

Invoke-Collected 'Physical disks (MSFT_PhysicalDisk, storage namespace)' {
    Get-CimStrings -Namespace 'root/Microsoft/Windows/Storage' -ClassName 'MSFT_PhysicalDisk' -Properties @('FriendlyName', 'MediaType', 'BusType', 'Size', 'HealthStatus')
}

Invoke-Collected 'Storage reliability counters (the exact class ohm-adapter-system queries on Windows)' {
    $lines = New-Object System.Collections.Generic.List[string]
    [void]$lines.Add('  query: SELECT DeviceId, Temperature FROM MSFT_StorageReliabilityCounter')
    [void]$lines.Add('  (adapters/system/src/windows.rs; rows outside 1-120 C are discarded by the adapter)')
    $rows = Get-CimStrings -Namespace 'root/Microsoft/Windows/Storage' -ClassName 'MSFT_StorageReliabilityCounter' -Properties @('DeviceId', 'Temperature', 'TemperatureMax', 'Wear', 'PowerOnHours')
    foreach ($row in $rows) { [void]$lines.Add($row) }
    return $lines
}

# --- 4. NVIDIA / NVML -----------------------------------------------------

Invoke-Collected 'NVIDIA driver and NVML presence' {
    $lines = New-Object System.Collections.Generic.List[string]
    try {
        $controllers = Get-CimInstance -ClassName Win32_VideoController -ErrorAction Stop
        $nvidia = @($controllers | Where-Object { $_.Name -like '*NVIDIA*' })
        if ($nvidia.Count -eq 0) {
            [void]$lines.Add('  no NVIDIA adapter reported by Win32_VideoController')
            [void]$lines.Add('  (NVML absence is a normal state: the nvidia adapter reports driver_missing)')
        } else {
            foreach ($gpu in $nvidia) {
                [void]$lines.Add("  adapter        : $($gpu.Name)")
                [void]$lines.Add("  driver version : $($gpu.DriverVersion)")
            }
        }
    } catch {
        [void]$lines.Add("  adapter query failed: $($_.Exception.Message)")
    }
    foreach ($candidate in @("$env:SystemRoot\System32\nvml.dll", "$env:ProgramW6432\NVIDIA Corporation\NVSMI\nvml.dll")) {
        if (-not [string]::IsNullOrWhiteSpace($candidate) -and (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            $item = Get-Item -LiteralPath $candidate
            [void]$lines.Add("  nvml present   : $candidate  (version $($item.VersionInfo.FileVersion))")
        } else {
            [void]$lines.Add("  nvml absent    : $candidate")
        }
    }
    $smi = Get-Command 'nvidia-smi.exe' -ErrorAction SilentlyContinue
    if ($null -ne $smi) {
        try {
            $smiOut = & $smi.Source '--query-gpu=name,driver_version,fan.speed,temperature.gpu' '--format=csv' 2>&1
            foreach ($row in @($smiOut)) { [void]$lines.Add("  nvidia-smi     : $row") }
            [void]$lines.Add('  (nvidia-smi fan.speed is a commanded percentage, not a tachometer)')
        } catch {
            [void]$lines.Add("  nvidia-smi failed: $($_.Exception.Message)")
        }
    } else {
        [void]$lines.Add('  nvidia-smi.exe : not on PATH')
    }
    return $lines
}

# --- 5. PawnIO ------------------------------------------------------------

Invoke-Collected 'PawnIO (the kernel driver LibreHardwareMonitor uses; winring0 must NOT be used)' {
    $lines = New-Object System.Collections.Generic.List[string]
    try {
        $keys = @(
            'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO',
            'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO'
        )
        $found = $false
        foreach ($key in $keys) {
            if (Test-Path -LiteralPath $key) {
                $props = Get-ItemProperty -LiteralPath $key -ErrorAction Stop
                [void]$lines.Add("  $key : DisplayVersion=$($props.DisplayVersion)  Publisher=$($props.Publisher)")
                $found = $true
            }
        }
        if (-not $found) {
            [void]$lines.Add('  PawnIO not found in the uninstall registry keys.')
            [void]$lines.Add('  LibreHardwareMonitor prompts for it; below version 2.0.0 it warns.')
        }
    } catch {
        [void]$lines.Add("  query failed: $($_.Exception.Message)")
    }
    return $lines
}

# --- 6. LibreHardwareMonitor ---------------------------------------------

Invoke-Collected 'LibreHardwareMonitor' {
    $lines = New-Object System.Collections.Generic.List[string]
    try {
        $procs = @(Get-Process -Name 'LibreHardwareMonitor' -ErrorAction SilentlyContinue)
        if ($procs.Count -eq 0) {
            [void]$lines.Add('  process: not running')
        } else {
            foreach ($proc in $procs) {
                [void]$lines.Add("  process: $($proc.ProcessName) (pid $($proc.Id))")
                try { [void]$lines.Add("  path   : $($proc.Path)") } catch { [void]$lines.Add('  path   : (not readable)') }
                try { [void]$lines.Add("  version: $($proc.FileVersionInfo.FileVersion)") } catch { }
            }
        }
    } catch {
        [void]$lines.Add("  process query failed: $($_.Exception.Message)")
    }
    [void]$lines.Add('  web server: http://127.0.0.1:8085/data.json')
    [void]$lines.Add((Get-JsonProbe -Url 'http://127.0.0.1:8085/data.json'))
    [void]$lines.Add('  (status 200 = reachable; 401 = LHM basic auth enabled; unreachable = server off)')
    return $lines
}

# --- 7. Autostart Run value (READ ONLY) -----------------------------------

Invoke-Collected 'Autostart — per-user Run value (read only; nothing is written)' {
    $lines = New-Object System.Collections.Generic.List[string]
    $runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    try {
        $value = Get-ItemProperty -LiteralPath $runKey -Name 'OpenHardwareOS' -ErrorAction Stop
        [void]$lines.Add("  $runKey\OpenHardwareOS")
        [void]$lines.Add("  value: $($value.OpenHardwareOS)")
        [void]$lines.Add('  hive: HKCU (per-user). A Run entry launches with the user token and can')
        [void]$lines.Add('        never start an elevated process — that is why it is the user hive.')
    } catch {
        [void]$lines.Add("  $runKey\OpenHardwareOS : not present (autostart is disabled)")
    }
    try {
        $machine = Get-ItemProperty -LiteralPath 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'OpenHardwareOS' -ErrorAction SilentlyContinue
        if ($null -ne $machine) {
            [void]$lines.Add('  WARNING: an OpenHardwareOS value also exists under HKLM\...\Run,')
            [void]$lines.Add('           which the application never writes. Capture this for the report.')
        } else {
            [void]$lines.Add('  HKLM\...\Run\OpenHardwareOS : not present (correct)')
        }
    } catch {
        [void]$lines.Add('  HKLM\...\Run : not readable')
    }
    return $lines
}

# --- 8. Application config directory (the only files this script reads) ---

$script:ConfigRoot = Get-ConfigRoot

Invoke-Collected 'Application config directory (listing only, inside the app''s own directory)' {
    $lines = New-Object System.Collections.Generic.List[string]
    $root = $script:ConfigRoot
    if ([string]::IsNullOrWhiteSpace($root)) {
        [void]$lines.Add('  could not resolve the config root (APPDATA is not set)')
        return $lines
    }
    $root = $root.TrimEnd([char]92)
    [void]$lines.Add("  root: $root")
    if (-not (Test-Path -LiteralPath $root)) {
        [void]$lines.Add('  not created yet — the application has not run under this account')
        return $lines
    }
    $entries = @(Get-ChildItem -LiteralPath $root -Recurse -ErrorAction SilentlyContinue)
    if ($entries.Count -eq 0) {
        [void]$lines.Add('  (the directory exists but is empty)')
    }
    foreach ($entry in $entries) {
        $kind = 'file'
        if ($entry.PSIsContainer) { $kind = 'dir ' }
        $size = ''
        if (-not $entry.PSIsContainer) { $size = "$($entry.Length) bytes" }
        $relative = $entry.FullName
        if ($relative.Length -gt $root.Length) { $relative = $relative.Substring($root.Length).TrimStart([char]92) }
        [void]$lines.Add("  $kind  $relative  $size  $($entry.LastWriteTime)")
    }
    return $lines
}

# --- 9. The application's own audit tail (inside its own directory) --------

$script:AuditLines = New-Object System.Collections.Generic.List[string]
$auditPath = $null
if (-not [string]::IsNullOrWhiteSpace($script:ConfigRoot)) {
    $auditPath = Join-Path $script:ConfigRoot 'audit.jsonl'
}
if ($null -ne $auditPath -and (Test-Path -LiteralPath $auditPath -PathType Leaf)) {
    try {
        $tail = @(Get-Content -LiteralPath $auditPath -Tail $AuditTailLines -ErrorAction Stop)
        foreach ($line in $tail) { [void]$script:AuditLines.Add($line) }
        Add-Line ''
        Add-Line "audit tail: $($script:AuditLines.Count) line(s) copied from $auditPath"
    } catch {
        [void]$script:AuditLines.Add("could not read $auditPath : $($_.Exception.Message)")
    }
} else {
    [void]$script:AuditLines.Add('audit.jsonl does not exist yet: no write has been recorded under this account.')
    Add-Line ''
    Add-Line 'audit tail: audit.jsonl not present'
}

# --- 10. The application's own binaries -----------------------------------

$script:DoctorLines = New-Object System.Collections.Generic.List[string]
$script:AuditCmdLines = New-Object System.Collections.Generic.List[string]
$candidates = Get-CliCandidates

if ($candidates.Count -eq 0) {
    $message = 'SKIPPED: no built ohm-cli.exe found. Build it first with `cargo build -p ohm-cli` from the repository root.'
    [void]$script:DoctorLines.Add($message)
    [void]$script:AuditCmdLines.Add($message)
    Add-Line ''
    Add-Line "binaries: $message"
} else {
    $cli = $candidates[0]
    Add-Line ''
    Add-Line "binaries: using $cli"
    foreach ($invocation in @(
        @{ Name = 'doctor'; Args = @('doctor'); Target = $script:DoctorLines },
        @{ Name = 'audit';  Args = @('audit', '--limit', '20'); Target = $script:AuditCmdLines }
    )) {
        try {
            [void]$invocation.Target.Add("command: `"$cli`" $($invocation.Args -join ' ')")
            [void]$invocation.Target.Add("started: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ssK')")
            $output = & $cli @($invocation.Args) 2>&1
            $code = $LASTEXITCODE
            foreach ($row in @($output)) {
                if ($null -ne $row) { [void]$invocation.Target.Add([string]$row) }
            }
            [void]$invocation.Target.Add("exit code: $code")
            Add-Line "  $($invocation.Name): exit code $code"
        } catch {
            [void]$invocation.Target.Add("FAILED to run: $($_.Exception.Message)")
            Add-Line "  $($invocation.Name): failed to run"
        }
    }
    [void]$script:DoctorLines.Add('NOTE: `doctor` creates the config directory if missing and may append a lifecycle entry')
    [void]$script:DoctorLines.Add('      to the application audit log. It never writes to hardware.')
}

# ---------------------------------------------------------------------------
# Write the bundle
# ---------------------------------------------------------------------------

# environment.txt is written first so that the log lines printed below are not
# duplicated inside it.
Write-BundleFile -Name 'environment.txt' -Content $script:Lines
Write-BundleFile -Name 'cli-doctor.txt' -Content $script:DoctorLines
Write-BundleFile -Name 'cli-audit.txt' -Content $script:AuditCmdLines
Write-BundleFile -Name 'audit-tail.jsonl' -Content $script:AuditLines

Add-Line ''
Add-Line 'Reminders for the operator:'
Add-Line '  * this bundle is raw evidence, not a verdict: fill in docs/windows-validation/result-template.md'
Add-Line '  * a step is only "Build/run passed" with a captured exit code, and only'
Add-Line '    "Verified on real hardware" with a before/after value and a device model'
Add-Line '  * add your screenshots to this folder and reference them by file name'
Add-Line ''
Add-Line 'BUNDLE FOLDER:'
Add-Line $script:BundlePath
Add-Line ''

exit 0
