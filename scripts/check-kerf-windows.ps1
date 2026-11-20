<#
.SYNOPSIS
    Run an unpacked Windows package of Kerf as someone who unzipped it would, and
    fail if it needs anything it does not carry.

.DESCRIPTION
    With nothing on PATH but Windows' own folders, has kerfd import a clip the package's
    ffmpeg made, and runs ffmpeg -version, ffprobe -version, kerf --version and kerfd
    --endpoint from the package, and opens the editor with no project, which has to draw
    its window (DirectX 11, through GPUI) and quit on its own when asked to.

    A machine that builds or tests Kerf has Visual Studio's C++ runtime in System32,
    so a package missing one of those libraries still runs there. While the editor
    runs, every library it has loaded is listed: each has to come from the package or
    from Windows, and none of the libraries the package carries (FFmpeg, the C++
    runtime) from anywhere else. The editor loads all that kerf and kerfd link to, as
    it links kerf-bridge, and kerf-bridge FFmpeg.

    kerf-editor.exe has to carry Kerf's icon as a resource, which is where Explorer, the
    taskbar, the Start menu entry and kerf:// links take it from (kerf-windows-icon.ps1).
    A package from before 0.1.2 has none, and is not held to it.

    package-kerf-windows.ps1 runs this on what it packs, and
    .github/workflows/verify-release.yml on a published release, unpacked on a fresh
    runner.

.PARAMETER Package
    The unpacked folder, with kerf-editor.exe in it.

.EXAMPLE
    pwsh scripts/check-kerf-windows.ps1 -Package target/package/Industrial-Kerf-0.1.0-windows-x64
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Package
)

$ErrorActionPreference = 'Stop'
$Package = (Resolve-Path $Package).Path.TrimEnd('\')
if (-not (Test-Path (Join-Path $Package 'kerf-editor.exe'))) { throw "No kerf-editor.exe in $Package" }
. (Join-Path $PSScriptRoot 'kerf-windows-icon.ps1')

# Names of what the package has to bring itself. Windows has none of them, so one
# of these loaded from anywhere but the package was missing from it.
$carried = @('vcruntime*', 'msvcp1*', 'concrt*', 'vccorlib*', 'av*-*', 'sw*-*', 'postproc*')

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
    # The editor's icon, among its resources. kerf.exe says which version this is
    # ("kerf 0.1.2"): the editor had no icon before 0.1.2.
    $said = "$(& (Join-Path $Package 'kerf.exe') --version)".Trim()
    $packageVersion = $null
    if ($said -notmatch '(\d+\.\d+\.\d+)' -or -not [version]::TryParse($Matches[1], [ref]$packageVersion)) {
        throw "kerf.exe --version said '$said', with no version in it"
    }
    $editorExe = Join-Path $Package 'kerf-editor.exe'
    if ($packageVersion -lt [version]'0.1.2' -and (Get-KerfProgramIcon $editorExe).Count -eq 0) {
        Write-Host "Industrial Kerf $packageVersion is from before the editor had an icon: not checked."
    } else {
        $pictures = Assert-KerfProgramIcon $editorExe
        Write-Host "kerf-editor.exe carries its icon at $(($pictures.Side | Sort-Object) -join ', ') pixels."
    }

    # kerfd only has to say where clients reach it.
    $endpoint = & (Join-Path $Package 'kerfd.exe') --dir $scratch --endpoint
    if ($LASTEXITCODE -ne 0 -or -not $endpoint) { throw "kerfd.exe did not run from the package (exit code $LASTEXITCODE)" }
    # kerfd finds the package's ffmpeg and ffprobe: without them it still imports, at a
    # made-up size, rate and length.
    $clip = Join-Path $scratch 'clip.mp4'
    & (Join-Path $Package 'ffmpeg.exe') -v error -f lavfi -i 'testsrc=duration=2:size=320x240:rate=25' -c:v mpeg4 $clip
    if ($LASTEXITCODE -ne 0) { throw "ffmpeg.exe made no clip (exit code $LASTEXITCODE)" }
    $requests = @(
        (@{ protocolVersion = 1; id = 'import'; operation = 'import'; actor = 'ui'; idempotencyKey = 'check-import'; baseRevision = 0; path = $clip } | ConvertTo-Json -Compress),
        (@{ protocolVersion = 1; id = 'state'; operation = 'get_state' } | ConvertTo-Json -Compress)
    )
    $answers = $requests | & (Join-Path $Package 'kerfd.exe') --stdio --dir (Join-Path $scratch 'project')
    if (-not ($answers -match 'rate=25 dur=00:00:02:00 video=320x240')) {
        throw "kerfd.exe did not probe the clip with the package's ffprobe (it is 320x240, 25 fps, 2 s): $answers"
    }

    # The editor, opening with no project, and quitting on its own after five seconds.
    $editor = Start-Process -FilePath $editorExe -PassThru -ArgumentList @('--quit-after-ms', '5000')
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
        throw 'kerf-editor.exe did not finish on its own'
    }
    if ($editor.ExitCode -ne 0) { throw "kerf-editor.exe failed with exit code $($editor.ExitCode)" }

    $modules = @($modules | Sort-Object -Unique)
    # Direct3D 11 is loaded once GPUI makes the device it draws the window with, so a
    # list without it was read too early to say anything.
    if (-not ($modules | Where-Object { $_ -like '*\d3d11.dll' })) {
        throw "Read the editor's libraries too early: no d3d11.dll among $($modules.Count)"
    }
    $outside = @(Find-OutsideModules $modules)
    if ($outside.Count -gt 0) {
        throw "kerf-editor.exe loaded libraries from outside the package:`n  $($outside -join "`n  ")"
    }
    Write-Host "The package ran on its own; the editor loaded $($modules.Count) libraries, each from the package or Windows."
} finally {
    $env:Path = $path
    Remove-Item $scratch -Recurse -Force -ErrorAction SilentlyContinue
}
