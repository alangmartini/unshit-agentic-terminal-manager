; packaging\terminal-manager.iss — Inno Setup 6 script, per-user install.
;
; Build both executables first, then compile this script:
;   cargo build --release -p terminal-manager --bin terminal-manager
;   cargo build --release -p unshit-ptyd --bin unshit-ptyd
;   & "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe" packaging\terminal-manager.iss
;
; Output: dist\terminal-manager-0.4.0-setup.exe

#define MyAppName "Terminal Manager"
#define MyAppVersion "0.4.0"
#define MyAppPublisher "Alan Galvao"
#define MyAppURL "https://github.com/alangmartini/unshit-agentic-terminal-manager"
#define MyAppExeName "terminal-manager.exe"
#define MyDaemonExeName "unshit-ptyd.exe"
#define MyStartupValueName "Unshit Terminal Manager"
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
; `commandline` lets the app's self-update pass /CURRENTUSER or /ALLUSERS so a
; silent upgrade keeps the scope of the existing install.
PrivilegesRequiredOverridesAllowed=dialog commandline
DefaultDirName={autopf}\{#MyAppName}
DisableProgramGroupPage=yes
DefaultGroupName={#MyAppName}
OutputDir=..\dist
OutputBaseFilename=terminal-manager-{#MyAppVersion}-setup
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

[Registry]
; Remove the app-created, opt-in startup value without creating it during setup.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "{#MyStartupValueName}"; Flags: dontcreatekey uninsdeletevalue

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch {#MyAppName}"; Flags: nowait postinstall skipifsilent

; The detached daemon may still be running at uninstall time; best-effort kill
; prevents a "file in use" leftover. Errors are ignored on purpose.
[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM {#MyDaemonExeName}"; Flags: runhidden; RunOnceId: "KillDaemon"

; ---------------------------------------------------------------------------
; Self-update hand-off (src/updater/install.rs launches this installer with
; /VERYSILENT ... /SELFUPDATE=1 /PARENTPID=<ui pid> /RELAUNCH=<ui exe>).
;
; The UI persists its layout, starts this installer, shuts the session daemon
; down and exits. Both executables must be free before [Files] runs, so
; PrepareToInstall waits for the parent pid (it runs before the in-use check).
; DeinitializeSetup then relaunches the app whether Setup succeeded (new exe)
; or aborted (the old exe comes back and reattaches to surviving sessions), so
; a failed silent update never leaves the user with nothing running.
; ---------------------------------------------------------------------------
[Code]
function OpenProcess(dwDesiredAccess: DWORD; bInheritHandle: BOOL; dwProcessId: DWORD): THandle;
  external 'OpenProcess@kernel32.dll stdcall';
function WaitForSingleObject(hHandle: THandle; dwMilliseconds: DWORD): DWORD;
  external 'WaitForSingleObject@kernel32.dll stdcall';
function CloseHandle(hObject: THandle): BOOL;
  external 'CloseHandle@kernel32.dll stdcall';
{ Integer-typed handle variants so the -1 (INVALID_HANDLE_VALUE) comparison is
  independent of THandle's width. Kernel handle values fit in 32 bits. }
function CreateFileW(lpFileName: String; dwDesiredAccess: DWORD; dwShareMode: DWORD;
  lpSecurityAttributes: Cardinal; dwCreationDisposition: DWORD; dwFlagsAndAttributes: DWORD;
  hTemplateFile: Cardinal): Integer;
  external 'CreateFileW@kernel32.dll stdcall';
function CloseFileHandle(hObject: Integer): BOOL;
  external 'CloseHandle@kernel32.dll stdcall';

const
  SYNCHRONIZE = $00100000;
  WAIT_OBJECT_0 = $00000000;
  WAIT_TIMEOUT = $00000102;
  PARENT_EXIT_TIMEOUT_MS = 60000;
  GENERIC_WRITE = $40000000;
  OPEN_EXISTING = 3;
  { FILE_ATTRIBUTE_NORMAL is predefined by Inno's script runtime. }
  INVALID_HANDLE_VALUE_I = -1;
  FILE_POLL_MS = 250;
  FILE_FREE_TIMEOUT_MS = 30000;

function IsSelfUpdate(): Boolean;
begin
  Result := ExpandConstant('{param:SELFUPDATE|0}') = '1';
end;

function SelfUpdateParentPid(): Integer;
begin
  Result := StrToIntDef(ExpandConstant('{param:PARENTPID|0}'), 0);
end;

{ Waits up to TimeoutMs for the process to exit. True when it is gone (or never
  existed); False when it is still running after the timeout. }
function WaitForProcessExit(Pid: Integer; TimeoutMs: DWORD): Boolean;
var
  Handle: THandle;
  WaitResult: DWORD;
begin
  Result := True;
  if Pid <= 0 then Exit;
  Handle := OpenProcess(SYNCHRONIZE, False, Pid);
  if Handle = 0 then Exit;
  try
    WaitResult := WaitForSingleObject(Handle, TimeoutMs);
  finally
    CloseHandle(Handle);
  end;
  Result := WaitResult <> WAIT_TIMEOUT;
end;

{ True once the file can be opened for exclusive write access, i.e. no process
  has it mapped as a running image any more (or it does not exist). The daemon
  acknowledges its shutdown before its process is gone, so this check, not the
  parent pid, is what proves unshit-ptyd.exe can be replaced. }
function WaitForFileWritable(const Path: String; TimeoutMs: Integer): Boolean;
var
  Handle: Integer;
  Waited: Integer;
begin
  Result := True;
  if not FileExists(Path) then Exit;
  Waited := 0;
  repeat
    Handle := CreateFileW(Path, GENERIC_WRITE, 0, 0, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0);
    if Handle <> INVALID_HANDLE_VALUE_I then
    begin
      CloseFileHandle(Handle);
      Log(Format('Self-update: %s free after %d ms', [Path, Waited]));
      Exit;
    end;
    Sleep(FILE_POLL_MS);
    Waited := Waited + FILE_POLL_MS;
  until Waited >= TimeoutMs;
  Result := False;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Pid: Integer;
  Exe: String;
begin
  Result := '';
  if not IsSelfUpdate() then Exit;
  Pid := SelfUpdateParentPid();
  Log(Format('Self-update: waiting for parent pid %d to exit', [Pid]));
  if not WaitForProcessExit(Pid, PARENT_EXIT_TIMEOUT_MS) then
  begin
    Result := Format('Terminal Manager (pid %d) did not exit within %d seconds. Close it and run the installer again.', [Pid, PARENT_EXIT_TIMEOUT_MS div 1000]);
    Exit;
  end;
  Exe := ExpandConstant('{app}\{#MyDaemonExeName}');
  if not WaitForFileWritable(Exe, FILE_FREE_TIMEOUT_MS) then
  begin
    Result := Format('The session daemon (%s) is still running after %d seconds. Close it and run the installer again.', [Exe, FILE_FREE_TIMEOUT_MS div 1000]);
    Exit;
  end;
  Exe := ExpandConstant('{app}\{#MyAppExeName}');
  if not WaitForFileWritable(Exe, FILE_FREE_TIMEOUT_MS) then
    Result := Format('%s is still in use after %d seconds. Close every Terminal Manager window and run the installer again.', [Exe, FILE_FREE_TIMEOUT_MS div 1000]);
end;

procedure DeinitializeSetup();
var
  Relaunch: String;
  ResultCode: Integer;
begin
  if not IsSelfUpdate() then Exit;
  { Never start a second instance next to one that is still running. }
  if not WaitForProcessExit(SelfUpdateParentPid(), 0) then
  begin
    Log('Self-update: parent still running, not relaunching');
    Exit;
  end;
  Relaunch := ExpandConstant('{param:RELAUNCH}');
  if Relaunch = '' then
  begin
    try
      Relaunch := ExpandConstant('{app}\{#MyAppExeName}');
    except
      Relaunch := '';
    end;
  end;
  if (Relaunch = '') or not FileExists(Relaunch) then
  begin
    Log('Self-update: no executable to relaunch');
    Exit;
  end;
  Log('Self-update: relaunching ' + Relaunch);
  ExecAsOriginalUser(Relaunch, '', ExtractFileDir(Relaunch), SW_SHOWNORMAL, ewNoWait, ResultCode);
end;
