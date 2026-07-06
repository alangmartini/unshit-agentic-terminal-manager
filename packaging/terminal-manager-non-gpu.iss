; packaging\terminal-manager-non-gpu.iss — one-off variant of
; terminal-manager.iss that emits a distinctly named setup.exe for the
; non-GPU/software-renderer test build (this branch), so it does not clobber
; the official dist\terminal-manager-0.2.0-setup.exe. Only the
; OutputBaseFilename differs; everything else is identical to the base script.

#define MyAppName "Terminal Manager"
#define MyAppVersion "0.2.2"
#define MyAppPublisher "Alan Galvao"
#define MyAppURL "https://github.com/alangmartini/unshit-agentic-terminal-manager"
#define MyAppExeName "terminal-manager.exe"
#define MyDaemonExeName "unshit-ptyd.exe"
; Release binaries, relative to this script (packaging\ -> repo root -> target\release).
#define ReleaseDir "..\target\release"

[Setup]
; A fixed GUID identifies the app across versions for upgrades/uninstall. Never change it.
AppId={{B3E1B6B2-7C44-4E2E-9C1A-0A1D2E3F4A5B}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
VersionInfoVersion={#MyAppVersion}
; --- Per-user install: no admin, no UAC ---
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DefaultDirName={autopf}\{#MyAppName}
DisableProgramGroupPage=yes
DefaultGroupName={#MyAppName}
OutputDir=..\dist
OutputBaseFilename=terminal-manager-{#MyAppVersion}-non-gpu-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=app.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
LicenseFile=..\LICENSE

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional icons:"; Flags: unchecked

[Files]
; BOTH executables land in the SAME {app} dir so the UI finds the daemon as a sibling.
Source: "{#ReleaseDir}\{#MyAppExeName}";   DestDir: "{app}"; Flags: ignoreversion
Source: "{#ReleaseDir}\{#MyDaemonExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE";                      DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"
Name: "{autodesktop}\{#MyAppName}";  Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch {#MyAppName}"; Flags: nowait postinstall skipifsilent

; The detached daemon may still be running at uninstall time; best-effort kill
; prevents a "file in use" leftover. Errors are ignored on purpose.
[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM {#MyDaemonExeName}"; Flags: runhidden; RunOnceId: "KillDaemon"
