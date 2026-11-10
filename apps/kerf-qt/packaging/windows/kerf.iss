; Industrial Kerf for Windows as an installer: for the current user alone, with no
; administrator, a Start menu entry, kerf:// review links opening the editor, and an
; uninstaller (Settings > Apps, or unins000.exe). Not signed. The Start menu entry, the
; links and Settings > Apps show the icon kerf-qt.exe carries; the installer has it too.
;
; scripts/package-kerf-windows.ps1 compiles it with Inno Setup's ISCC from the folder it
; packed, the same files as the zip:
;
;   ISCC /DSource=<packed folder> /DVersion=<version> /DOutputDir=<folder> /DOutputName=<name> kerf.iss
;
; scripts/check-kerf-windows-installer.ps1 installs it, checks each of these, and
; uninstalls it. /DWithoutLinks makes one that registers no kerf://, which that check has
; to fail (package.yml).

#ifndef Source
  #error Pass /DSource=<the packed folder>
#endif
#ifndef Version
  #error Pass /DVersion=<the version>
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif
#ifndef OutputName
  #define OutputName "Industrial-Kerf-" + Version + "-windows-x64-setup"
#endif

[Setup]
; One id for every version, so a newer installer replaces an older install.
AppId={{2226F529-6D3C-47F0-9585-839485B795C2}
AppName=Industrial Kerf
AppVersion={#Version}
AppVerName=Industrial Kerf {#Version}
AppPublisher=Industrial Kerf
AppPublisherURL=https://github.com/imjunseong958-droid/industrial-kerf
AppSupportURL=https://github.com/imjunseong958-droid/industrial-kerf/issues
AppUpdatesURL=https://github.com/imjunseong958-droid/industrial-kerf/releases
; For this user only: %LOCALAPPDATA%\Programs, the user's Start menu, and HKCU.
PrivilegesRequired=lowest
DefaultDirName={autopf}\Industrial Kerf
DisableProgramGroupPage=yes
DisableDirPage=auto
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
UninstallDisplayName=Industrial Kerf
UninstallDisplayIcon={app}\kerf-qt.exe
; The installer's own icon: Kerf's, the one kerf-qt.exe carries (tools/kerf-icon makes it
; from apps/kerf-qt/resources/icon/industrial-kerf.svg). Relative to this file.
SetupIconFile=..\..\resources\icon\industrial-kerf.ico
; Tells Explorer the kerf: scheme changed.
ChangesAssociations=yes
; An open editor is closed before its files are replaced or removed.
CloseApplications=yes
OutputDir={#OutputDir}
OutputBaseFilename={#OutputName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

[Files]
Source: "{#Source}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Industrial Kerf"; Filename: "{app}\kerf-qt.exe"; WorkingDir: "{app}"; Comment: "Edit video by asking"

#ifndef WithoutLinks
[Registry]
; kerf://<project>/rev<N>[@TC] opens the editor with the link as its argument
; (apps/kerf-qt/ui/review_link.h). HKA is HKCU in a per-user install.
Root: HKA; Subkey: "Software\Classes\kerf"; ValueType: string; ValueName: ""; ValueData: "URL:Industrial Kerf review link"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\kerf"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\kerf\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\kerf-qt.exe"",0"
Root: HKA; Subkey: "Software\Classes\kerf\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\kerf-qt.exe"" ""%1"""
#endif

[Run]
Filename: "{app}\kerf-qt.exe"; Description: "Open Industrial Kerf"; Flags: nowait postinstall skipifsilent
