; Inno Setup script for the Windows installer. Build it with build-installer.ps1, which
; compiles the app, gathers the license notices and passes the version in.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define AppName "PaintBoard"
#define AppExe "paintboard.exe"
#define Root "..\.."

[Setup]
; Keeps upgrades and the uninstaller tied to this app; never change it.
AppId={{6F2C9A41-3B7E-4C0D-9E55-1A8D2B7F4C63}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppName}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
; Installs for the current user without asking for administrator rights; the dialog
; offers installing for everyone instead.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ChangesAssociations=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#Root}\target\installer
OutputBaseFilename=PaintBoard-{#AppVersion}-setup
SetupIconFile={#Root}\crates\pb-app\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Root}\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\target\installer\THIRD-PARTY.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; .pboard files open in PaintBoard with its icon.
Root: HKA; Subkey: "Software\Classes\.pboard"; ValueType: string; ValueName: ""; ValueData: "PaintBoard.Board"; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\PaintBoard.Board"; ValueType: string; ValueName: ""; ValueData: "PaintBoard board"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\PaintBoard.Board\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"",0"
Root: HKA; Subkey: "Software\Classes\PaintBoard.Board\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""
; .pbplugin files (plugins in one file) open in PaintBoard, which offers to install them.
Root: HKA; Subkey: "Software\Classes\.pbplugin"; ValueType: string; ValueName: ""; ValueData: "PaintBoard.Plugin"; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\PaintBoard.Plugin"; ValueType: string; ValueName: ""; ValueData: "PaintBoard plugin"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\PaintBoard.Plugin\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"",0"
Root: HKA; Subkey: "Software\Classes\PaintBoard.Plugin\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
