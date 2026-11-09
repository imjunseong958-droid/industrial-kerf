<#
.SYNOPSIS
    Build the Qt editor and its tools for Windows and pack them into a folder that
    runs on its own, as a zip: unpack it anywhere and run kerf-qt.exe.

.DESCRIPTION
    Needs, as a Windows build of the editor does: Rust, Visual Studio's C++ tools,
    CMake, Qt for MSVC with its bin folder on PATH (for windeployqt), a shared GPL
    build of FFmpeg in FFMPEG_DIR, and LIBCLANG_PATH for its bindings.

    The FFmpeg build makes the package GPL, so it may only be given to anyone with
    its source: scripts/collect-kerf-windows-sources.sh makes that, of this commit.

    The package finds everything beside kerf-qt.exe. check-kerf-windows.ps1 runs it
    once with nothing else on PATH and lists what the editor loaded, so a library that
    was left out stops the package here and not on someone else's machine.

.PARAMETER OutDir
    Where the folder and the zip go, under the repository unless absolute.

.PARAMETER SkipBuild
    Pack what was last built.

.PARAMETER SkipRun
    Do not run the package. For a machine with no desktop to run it on.

.EXAMPLE
    pwsh scripts/package-kerf-windows.ps1
#>
[CmdletBinding()]
param(
    [string]$OutDir = 'target/package',
    [string]$FfmpegDir = $env:FFMPEG_DIR,
    [switch]$SkipBuild,
    [switch]$SkipRun
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not [System.IO.Path]::IsPathRooted($OutDir)) { $OutDir = Join-Path $root $OutDir }

# No param block: named parameters would take a tool's own -p as theirs.
function Invoke-Tool {
    $program, $arguments = $args
    & $program @arguments
    if ($LASTEXITCODE -ne 0) { throw "$program $arguments failed with exit code $LASTEXITCODE" }
}

# The C++ runtime the editor and the Rust programs link to, which a Windows without
# Visual Studio may not have: from the folder Microsoft keeps for redistributing it.
function Find-CppRuntime {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) { throw "vswhere.exe not found at $vswhere" }
    $studio = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $studio) { throw 'No Visual Studio with the C++ tools was found' }
    $redist = Get-ChildItem (Join-Path $studio 'VC\Redist\MSVC') -Directory |
        Where-Object { $_.Name -match '^\d+\.\d+' } |
        Sort-Object { [version]$_.Name } -Descending |
        ForEach-Object { Get-ChildItem (Join-Path $_.FullName 'x64') -Directory -Filter 'Microsoft.VC*.CRT' -ErrorAction SilentlyContinue } |
        Select-Object -First 1
    if (-not $redist) { throw "No C++ runtime to redistribute under $studio" }
    return $redist.FullName
}

if (-not $FfmpegDir -or -not (Test-Path (Join-Path $FfmpegDir 'bin\ffmpeg.exe'))) {
    throw 'Set FFMPEG_DIR, or pass -FfmpegDir, to a shared build of FFmpeg (with bin\ffmpeg.exe)'
}
$windeployqt = (Get-Command windeployqt -ErrorAction SilentlyContinue).Source
if (-not $windeployqt) { throw 'windeployqt not found: put the bin folder of Qt for MSVC on PATH' }

# What the package says about its licence and its source has to hold.
$ffmpegBuild = & (Join-Path $FfmpegDir 'bin\ffmpeg.exe') -hide_banner -buildconf
if ($LASTEXITCODE -ne 0) { throw 'The FFmpeg build did not run' }
if ($ffmpegBuild -match '--enable-nonfree') { throw 'This FFmpeg build is nonfree, and may not be given to anyone' }
if (-not ($ffmpegBuild -match '--enable-gpl') -or -not ($ffmpegBuild -match '--enable-version3')) {
    throw 'THIRD-PARTY.txt and SOURCE.txt describe a GPL version 3 build of FFmpeg, and this is not one'
}
$gplText = Join-Path $FfmpegDir 'LICENSE.txt'
if (-not (Test-Path $gplText) -or -not (Select-String -Path $gplText -Quiet -Pattern 'GNU GENERAL PUBLIC LICENSE')) {
    throw "No GPL text at $gplText"
}
if (& git -C $root status --porcelain --untracked-files=no) {
    throw 'Commit first: the package names the commit its source comes from'
}
$commit = & git -C $root rev-parse HEAD

$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$name = "Industrial-Kerf-$version-windows-x64"
$build = Join-Path $root 'target/qt-package'
$release = Join-Path $root 'target/release'

Push-Location $root
try {
    if (-not $SkipBuild) {
        # Optimized: the editor's live preview and its exports draw frames in the bridge.
        Invoke-Tool cargo build --release -p kerf-cli --bin kerf
        Invoke-Tool cargo build --release -p kerf-runtime --bin kerfd
        Invoke-Tool cargo build --release -p kerf-mcp --bin kerf-mcp
        Invoke-Tool cargo build --release -p kerf-qt-bridge
        $bridge = (Join-Path $release 'kerf_qt_bridge.dll.lib').Replace('\', '/')
        Invoke-Tool cmake -S apps/kerf-qt -B $build "-DKERF_BRIDGE_LIBRARY=$bridge"
        Invoke-Tool cmake --build $build --config Release --target kerf-qt --parallel
    }

    $stage = Join-Path $OutDir $name
    if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $stage | Out-Null

    Copy-Item (Join-Path $build 'Release/kerf-qt.exe') $stage
    Copy-Item (Join-Path $release 'kerf_qt_bridge.dll') $stage
    # The editor looks beside itself for kerfd and kerf-mcp. kerf opens the editor beside it.
    foreach ($tool in 'kerfd', 'kerf', 'kerf-mcp') { Copy-Item (Join-Path $release "$tool.exe") $stage }

    # Qt's libraries and plugins. Its software OpenGL is for what draws with OpenGL, and
    # the editor draws without.
    Invoke-Tool $windeployqt --release --no-translations --no-system-d3d-compiler --no-opengl-sw (Join-Path $stage 'kerf-qt.exe')

    # Windows starts a program's child from the parent's folder before PATH, so these are
    # the ffmpeg and ffprobe that kerfd and the editor run.
    $ffmpegBin = Join-Path $FfmpegDir 'bin'
    Copy-Item (Join-Path $ffmpegBin '*.dll') $stage
    foreach ($tool in 'ffmpeg', 'ffprobe') { Copy-Item (Join-Path $ffmpegBin "$tool.exe") $stage }

    Copy-Item (Join-Path (Find-CppRuntime) '*.dll') $stage

    Copy-Item LICENSE (Join-Path $stage 'LICENSE.txt')
    Copy-Item apps/kerf-qt/packaging/windows/THIRD-PARTY.txt $stage
    Copy-Item licenses (Join-Path $stage 'licenses') -Recurse
    New-Item -ItemType Directory -Path (Join-Path $stage 'fonts') | Out-Null
    Copy-Item apps/kerf-qt/resources/fonts/*-LICENSE.txt (Join-Path $stage 'fonts')
    Copy-Item $gplText (Join-Path $stage 'COPYING-GPLv3.txt')
    $ffmpegBuild | Set-Content -Path (Join-Path $stage 'FFmpeg-BUILD.txt') -Encoding utf8
    $qtVersion = (Get-Item (Join-Path $stage 'Qt6Core.dll')).VersionInfo.ProductVersion
    @"
Industrial Kerf $version for Windows, from commit $commit.
Its source: https://github.com/imjunseong958-droid/industrial-kerf

This package is under the GNU General Public License version 3 (COPYING-GPLv3.txt),
and you have the right to its complete source. That is $name-source.tar, published
beside this package wherever you got it. It holds Kerf and its Rust crates, FFmpeg,
each library built into FFmpeg, the scripts that built them, and Qt $qtVersion.

Which licence covers which part is in THIRD-PARTY.txt.
"@ | Set-Content -Path (Join-Path $stage 'SOURCE.txt') -Encoding utf8

    if (-not $SkipRun) {
        # With nothing else on PATH, it has to find all it needs beside itself, and the
        # editor may load nothing the package carries from anywhere else.
        & (Join-Path $PSScriptRoot 'check-kerf-windows.ps1') -Package $stage
    }

    $zip = Join-Path $OutDir "$name.zip"
    if (Test-Path $zip) { Remove-Item $zip -Force }
    Compress-Archive -Path $stage -DestinationPath $zip -CompressionLevel Optimal
    $hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $name.zip" | Set-Content -Path "$zip.sha256" -Encoding ascii
    $megabytes = [math]::Round((Get-Item $zip).Length / 1MB)
    Write-Host "Packed $zip ($megabytes MB), sha256 $hash"
} finally {
    Pop-Location
}
