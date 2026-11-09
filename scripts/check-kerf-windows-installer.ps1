<#
.SYNOPSIS
    Install Kerf with its Windows installer, check what it put where, and uninstall it.

.DESCRIPTION
    Installs for the current user, silently, into a folder of its own, then:
    runs the installed copy as check-kerf-windows.ps1 runs a package; finds its Start menu
    entry, pointing at the installed editor; finds kerf:// registered as Explorer sees it
    (AssocQueryString), and follows a kerf:// link as a click would, which has to start the
    installed editor with the link; then uninstalls it and finds each of those gone.

    It installs for the user running it, so it refuses to run where Industrial Kerf is
    installed already: its uninstall at the end would take that copy away.

    package-kerf-windows.ps1 runs this on the installer it makes, and
    .github/workflows/verify-release.yml on a published one, on a fresh runner.

.PARAMETER Installer
    The installer, Industrial-Kerf-<version>-windows-x64-setup.exe.

.EXAMPLE
    pwsh scripts/check-kerf-windows-installer.ps1 -Installer target/package/Industrial-Kerf-0.1.1-windows-x64-setup.exe
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Installer
)

$ErrorActionPreference = 'Stop'
$Installer = (Resolve-Path $Installer).Path

# The installer's AppId (apps/kerf-qt/packaging/windows/kerf.iss), as Inno Setup names its
# uninstall entry.
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{2226F529-6D3C-47F0-9585-839485B795C2}_is1'
$schemeKey = 'HKCU:\Software\Classes\kerf'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Industrial Kerf.lnk'

if (Test-Path $uninstallKey) {
    throw "Industrial Kerf is installed for this user already ($((Get-ItemProperty $uninstallKey).InstallLocation)); this check would uninstall it"
}

Add-Type -Namespace KerfCheck -Name Shell -MemberDefinition @'
[DllImport("shlwapi.dll", CharSet = CharSet.Unicode)]
public static extern int AssocQueryString(int flags, int str, string assoc, string extra, System.Text.StringBuilder result, ref int length);
'@

# The command Explorer runs to open a kerf:// link; empty when none is registered.
function Get-KerfLinkCommand {
    $length = 1024
    $command = New-Object System.Text.StringBuilder $length
    # ASSOCSTR_COMMAND, for the scheme's open verb.
    $result = [KerfCheck.Shell]::AssocQueryString(0, 1, 'kerf', 'open', $command, [ref]$length)
    if ($result -ne 0) { return '' }
    return $command.ToString()
}

# Under LOCALAPPDATA, which Windows gives in full: TEMP may be in 8.3 form (RUNNER~1), which
# the paths Windows reports for the installed editor would not match.
$scratch = Join-Path $env:LOCALAPPDATA "kerf-installer-check-$([guid]::NewGuid())"
$target = Join-Path $scratch 'Industrial Kerf'
New-Item -ItemType Directory -Path $scratch | Out-Null
$installed = $false
try {
    $log = Join-Path $scratch 'install.log'
    $setup = Start-Process -FilePath $Installer -Wait -PassThru -ArgumentList @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART',
        "/DIR=`"$target`"", "/LOG=`"$log`"")
    $installed = $true
    $editor = Join-Path $target 'kerf-qt.exe'
    if ($setup.ExitCode -ne 0 -or -not (Test-Path $editor)) {
        Get-Content $log -ErrorAction SilentlyContinue | Select-Object -Last 40 | Write-Host
        if ($setup.ExitCode -ne 0) { throw "The installer failed with exit code $($setup.ExitCode)" }
        throw "The installer put no kerf-qt.exe in $target"
    }
    if (-not (Test-Path $uninstallKey)) { throw 'The installer left no uninstall entry' }

    # The installed copy runs on its own, as the zip does.
    & (Join-Path $PSScriptRoot 'check-kerf-windows.ps1') -Package $target

    # The Start menu entry opens the installed editor.
    if (-not (Test-Path $shortcut)) { throw "No Start menu entry at $shortcut" }
    $link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut)
    if ($link.TargetPath -ne $editor) { throw "The Start menu entry opens $($link.TargetPath), not $editor" }

    # kerf:// as Explorer sees it, then followed as a click on a link is.
    $expected = "`"$editor`" `"%1`""
    $command = Get-KerfLinkCommand
    if ($command -ne $expected) { throw "kerf:// opens with '$command', not '$expected'" }
    $review = 'kerf://kerf-installer-check/rev1@00:00:01:00'
    Start-Process $review
    $deadline = (Get-Date).AddSeconds(60)
    $opened = $null
    while (-not $opened -and (Get-Date) -lt $deadline) {
        $opened = Get-CimInstance Win32_Process -Filter "Name = 'kerf-qt.exe'" |
            Where-Object { $_.ExecutablePath -eq $editor -and $_.CommandLine -like "*$review*" } |
            Select-Object -First 1
        if (-not $opened) { Start-Sleep -Milliseconds 500 }
    }
    if (-not $opened) { throw "Following $review started no installed kerf-qt.exe with the link" }
    # It says the project is not on this computer, and waits for an answer nobody gives.
    Stop-Process -Id $opened.ProcessId -Force
    Wait-Process -Id $opened.ProcessId -Timeout 30 -ErrorAction SilentlyContinue
    Write-Host "Following $review started $editor with it."

    # The uninstaller runs a copy of itself from the temporary folder and returns at once:
    # wait for what it removes.
    $uninstall = Start-Process -FilePath (Join-Path $target 'unins000.exe') -Wait -PassThru -ArgumentList @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')
    if ($uninstall.ExitCode -ne 0) { throw "The uninstaller failed with exit code $($uninstall.ExitCode)" }
    $deadline = (Get-Date).AddSeconds(120)
    while (((Test-Path $uninstallKey) -or (Test-Path $editor)) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    $installed = $false
    $left = @()
    if (Test-Path $editor) { $left += "kerf-qt.exe in $target" }
    if (Test-Path $uninstallKey) { $left += 'the uninstall entry' }
    if (Test-Path $shortcut) { $left += 'the Start menu entry' }
    if (Test-Path $schemeKey) { $left += 'the kerf:// registration' }
    if (Get-KerfLinkCommand) { $left += 'a command for kerf:// links' }
    if ($left) { throw "Uninstalling left $($left -join ', ')" }
    Write-Host 'The installer installed for this user, with a Start menu entry and kerf:// links, and uninstalled cleanly.'
} finally {
    if ($installed -and (Test-Path (Join-Path $target 'unins000.exe'))) {
        Start-Process -FilePath (Join-Path $target 'unins000.exe') -Wait -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') -ErrorAction SilentlyContinue
    }
    Get-Process kerf-qt -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$target*" } | Stop-Process -Force -ErrorAction SilentlyContinue
    Remove-Item $scratch -Recurse -Force -ErrorAction SilentlyContinue
}
