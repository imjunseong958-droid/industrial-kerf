<#
.SYNOPSIS
    Run an unpacked Windows package of Kerf as someone who unzipped it would, and
    fail if it needs anything it does not carry.

.DESCRIPTION
    With nothing on PATH but Windows' own folders, runs ffmpeg -version,
    ffprobe -version, kerf --version and kerfd --endpoint from the package, and opens
    the editor's start window and takes a picture of it.

    A machine that builds or tests Kerf has Visual Studio's C++ runtime in System32,
    so a package missing one of those libraries still runs there. While the editor
    runs, every library it has loaded is listed: each has to come from the package or
    from Windows, and none of the libraries the package carries (Qt, FFmpeg, the C++
    runtime) from anywhere else. The editor loads all that kerf, kerfd and the bridge
    link to, as it links the bridge, and the bridge FFmpeg.

    package-kerf-windows.ps1 runs this on what it packs, and
    .github/workflows/verify-release.yml on a published release, unpacked on a fresh
    runner.

.PARAMETER Package
    The unpacked folder, with kerf-qt.exe in it.

.EXAMPLE
    pwsh scripts/check-kerf-windows.ps1 -Package target/package/Industrial-Kerf-0.1.0-windows-x64
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Package
)

$ErrorActionPreference = 'Stop'
$Package = (Resolve-Path $Package).Path.TrimEnd('\')
if (-not (Test-Path (Join-Path $Package 'kerf-qt.exe'))) { throw "No kerf-qt.exe in $Package" }

# Names of what the package has to bring itself. Windows has none of them, so one
# of these loaded from anywhere but the package was missing from it. (Its icuuc.dll
# is Windows' own, which windeployqt copies, and passes Qt on to System32's icu.dll.)
$carried = @('vcruntime*', 'msvcp1*', 'concrt*', 'vccorlib*', 'qt6*', 'av*-*', 'sw*-*', 'postproc*')

# Every library the process has loaded so far, by full path. The process may exit
# while being read.
function Get-LoadedModules([System.Diagnostics.Process]$process) {
    try {
        $process.Refresh()
        return @($process.Modules | ForEach-Object { $_.FileName })
    } catch {
        return @()
    }
}

# What loaded from outside the package and Windows, or what the package carries
# loaded from Windows instead: one line each.
function Find-OutsideModules([string[]]$modules) {
    $windows = $env:SystemRoot.TrimEnd('\') + '\'
    $own = $Package + '\'
    $outside = @()
    foreach ($module in ($modules | Sort-Object -Unique)) {
        $name = Split-Path $module -Leaf
        if ($module.StartsWith($own, [System.StringComparison]::OrdinalIgnoreCase)) { continue }
        $isCarried = $false
        foreach ($pattern in $carried) { if ($name -like $pattern) { $isCarried = $true } }
        if ($module.StartsWith($windows, [System.StringComparison]::OrdinalIgnoreCase) -and -not $isCarried) { continue }
        $outside += $module
    }
    return $outside
}

$path = $env:Path
$env:Path = "$env:SystemRoot\System32;$env:SystemRoot"
$scratch = Join-Path ([System.IO.Path]::GetTempPath()) "kerf-check-$([guid]::NewGuid())"
New-Item -ItemType Directory -Path $scratch | Out-Null
try {
    foreach ($check in @(@('ffmpeg.exe', '-version'), @('ffprobe.exe', '-version'), @('kerf.exe', '--version'))) {
        & (Join-Path $Package $check[0]) $check[1] | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "$($check[0]) did not run from the package (exit code $LASTEXITCODE)" }
    }
    # kerfd only has to say where clients reach it.
    $endpoint = & (Join-Path $Package 'kerfd.exe') --dir $scratch --endpoint
    if ($LASTEXITCODE -ne 0 -or -not $endpoint) { throw "kerfd.exe did not run from the package (exit code $LASTEXITCODE)" }

    # The editor, opening its start window and taking a picture of it.
    $shot = Join-Path $scratch 'start.png'
    $editor = Start-Process -FilePath (Join-Path $Package 'kerf-qt.exe') -PassThru -ArgumentList @(
        '--settings-dir', "`"$(Join-Path $scratch 'settings')`"",
        '--screenshot', "`"$shot`"",
        '--quit-after-ms', '5000')
    # Taken now: Start-Process leaves no exit code otherwise.
    $null = $editor.Handle
    $modules = @()
    $deadline = (Get-Date).AddSeconds(60)
    while (-not $editor.HasExited -and (Get-Date) -lt $deadline) {
        $modules += Get-LoadedModules $editor
        Start-Sleep -Milliseconds 250
    }
    if (-not $editor.WaitForExit(1000)) {
        $editor.Kill()
        throw 'kerf-qt.exe did not finish on its own'
    }
    if ($editor.ExitCode -ne 0) { throw "kerf-qt.exe failed with exit code $($editor.ExitCode)" }
    if (-not (Test-Path $shot)) { throw 'kerf-qt.exe ran and made no screenshot' }

    $modules = @($modules | Sort-Object -Unique)
    # Qt's platform plugin is loaded once the window is made, so a list without it
    # was read too early to say anything.
    if (-not ($modules | Where-Object { $_ -like '*\platforms\qwindows.dll' })) {
        throw "Read the editor's libraries too early: no platform plugin among $($modules.Count)"
    }
    $outside = @(Find-OutsideModules $modules)
    if ($outside.Count -gt 0) {
        throw "kerf-qt.exe loaded libraries from outside the package:`n  $($outside -join "`n  ")"
    }
    Write-Host "The package ran on its own; the editor loaded $($modules.Count) libraries, each from the package or Windows."
} finally {
    $env:Path = $path
    Remove-Item $scratch -Recurse -Force -ErrorAction SilentlyContinue
}
