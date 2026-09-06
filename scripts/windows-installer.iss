; Build through package-release.ps1 with Inno Setup 6.3 or newer.
#ifndef AppVersion
  #error AppVersion must be supplied by package-release.ps1
#endif
#ifndef PackageDirectory
  #error PackageDirectory must be supplied by package-release.ps1
#endif
#ifndef PackageOutputDirectory
  #error PackageOutputDirectory must be supplied by package-release.ps1
#endif
#ifndef RepositoryRoot
  #error RepositoryRoot must be supplied by package-release.ps1
#endif

[Setup]
AppId={{DE5616A8-30DB-41B8-9A24-D91F86DE45D7}
AppName=VCLogg2
AppVersion={#AppVersion}
AppPublisher=VCLogg2
AppPublisherURL=https://github.com/zhaiyanqi/vclogg2
DefaultDirName={localappdata}\Programs\VCLogg2
DisableDirPage=no
DisableProgramGroupPage=yes
UsePreviousAppDir=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
UninstallDisplayIcon={app}\vclogg2.exe
SetupIconFile={#RepositoryRoot}\crates\vclogg-app\resources\windows\vclogg2.ico
LicenseFile={#PackageDirectory}\LICENSE
OutputDir={#PackageOutputDirectory}
OutputBaseFilename=vclogg-{#AppVersion}-windows-x86_86-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; Flags: unchecked

[Files]
; The generated configuration is tracked by Inno's install/rollback/uninstall log.
Source: "{tmp}\vclogg2-data-dir.txt"; DestDir: "{app}"; Flags: external ignoreversion; ExternalSize: 1024
Source: "{#PackageDirectory}\vclogg2.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PackageDirectory}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PackageDirectory}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\VCLogg2"; Filename: "{app}\vclogg2.exe"; WorkingDir: "{app}"
Name: "{userdesktop}\VCLogg2"; Filename: "{app}\vclogg2.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\vclogg2.exe"; Description: "Launch VCLogg2"; Flags: nowait postinstall skipifsilent

[Code]
var
  DataDirectoryPage: TInputDirWizardPage;
  LoadedInstallationDirectory: String;

procedure InitializeWizard;
begin
  DataDirectoryPage := CreateInputDirPage(wpSelectDir,
    'Select data directory', 'Where should VCLogg2 store its data?',
    'Settings, history, index caches and temporary results will be stored here.' + #13#10 +
    'Existing data is kept during upgrades and uninstall. Choosing another directory does not move existing data.',
    False, 'VCLogg2');
  DataDirectoryPage.Add('Data directory:');
  DataDirectoryPage.Values[0] := ExpandConstant('{param:DATADIR|}');
  if DataDirectoryPage.Values[0] = '' then
    DataDirectoryPage.Values[0] := GetPreviousData('DataDirectory', ExpandConstant('{localappdata}\VCLogg2'));
end;

function ValidateDataDirectory: String;
var
  Directory: String;
begin
  Result := '';
  Directory := Trim(DataDirectoryPage.Values[0]);
  if (Directory = '') or (Pos(#13, Directory) > 0) or (Pos(#10, Directory) > 0) or
     (Pos(#0, Directory) > 0) then
    Result := 'Choose an absolute data directory.'
  else if not (((Length(Directory) >= 3) and (Directory[2] = ':') and (Directory[3] = '\')) or
               (Copy(Directory, 1, 2) = '\\')) then
    Result := 'Choose an absolute data directory, such as D:\VCLogg2Data.'
  else
    DataDirectoryPage.Values[0] := ExpandFileName(Directory);
end;

function NextButtonClick(CurPageID: Integer): Boolean;
var
  ConfigurationPath, ErrorMessage: String;
  Configuration: AnsiString;
begin
  Result := True;
  if (CurPageID = wpSelectDir) and
     (CompareText(LoadedInstallationDirectory, WizardDirValue) <> 0) then begin
    ConfigurationPath := AddBackslash(WizardDirValue) + 'vclogg2-data-dir.txt';
    if FileExists(ConfigurationPath) and (ExpandConstant('{param:DATADIR|}') = '') then begin
      if not LoadStringFromFile(ConfigurationPath, Configuration) then begin
        MsgBox('Cannot read the existing data directory configuration.', mbError, MB_OK);
        Result := False;
        Exit;
      end;
      DataDirectoryPage.Values[0] := Trim(Utf8Decode(Configuration));
    end;
    LoadedInstallationDirectory := WizardDirValue;
  end;
  if CurPageID = DataDirectoryPage.ID then begin
    ErrorMessage := ValidateDataDirectory;
    Result := ErrorMessage = '';
    if not Result then
      MsgBox(ErrorMessage, mbError, MB_OK);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Directory, ProbePath: String;
begin
  Result := ValidateDataDirectory;
  if Result <> '' then Exit;
  Directory := DataDirectoryPage.Values[0];
  if not ForceDirectories(Directory) then begin
    Result := 'Cannot create the data directory: ' + Directory;
    Exit;
  end;
  ProbePath := GenerateUniqueName(AddBackslash(Directory), '.tmp');
  if not SaveStringToFile(ProbePath, 'VCLogg2 write check', False) then begin
    Result := 'The data directory is not writable: ' + Directory;
    Exit;
  end;
  if not DeleteFile(ProbePath) then begin
    Result := 'Cannot remove the data directory write check: ' + ProbePath;
    Exit;
  end;
  if not SaveStringToFile(ExpandConstant('{tmp}\vclogg2-data-dir.txt'), Utf8Encode(Directory), False) then
    Result := 'Cannot prepare the data directory configuration.';
end;

procedure RegisterPreviousData(PreviousDataKey: Integer);
begin
  SetPreviousData(PreviousDataKey, 'DataDirectory', DataDirectoryPage.Values[0]);
end;

function UpdateReadyMemo(Space, NewLine, MemoUserInfoInfo, MemoDirInfo, MemoTypeInfo,
  MemoComponentsInfo, MemoGroupInfo, MemoTasksInfo: String): String;
begin
  Result := MemoDirInfo + NewLine + NewLine + 'Data directory:' + NewLine +
    Space + DataDirectoryPage.Values[0];
  if MemoTasksInfo <> '' then
    Result := Result + NewLine + NewLine + MemoTasksInfo;
end;
