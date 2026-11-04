; Script Inno Setup 6 para o Instalador Oficial do tVisio v2.2.3 (Windows x64)
; ==============================================================================

#define MyAppName "tVisio"
#define MyAppVersion "2.2.3"
#define MyAppPublisher "Tiago Treter (@TTreter)"
#define MyAppURL "https://github.com/TTreter/tVisio"
#define MyAppExeName "tVisio.exe"

[Setup]
; Informações Básicas do Aplicativo
AppId={{5C2CD55C-92AD-4999-8666-912BD3E70000}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} v{#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
VersionInfoVersion={#MyAppVersion}.0
VersionInfoCompany={#MyAppPublisher}
VersionInfoDescription=tVisio — Ultra-Low Latency Virtual Webcam
VersionInfoCopyright=Copyright (C) 2026 Tiago Treter - GPLv3

; Configuração de Instalação e Arquitetura x64
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
AllowNoIcons=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
CloseApplications=no
RestartApplications=no
RestartIfNeededByRun=no
AlwaysRestart=no

; Estilo e Compressão Ultra
SetupIconFile=assets\icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName},0
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
DisableWelcomePage=no

; Saída do Instalador
OutputDir=_local\dist
OutputBaseFilename=tVisio-Setup-v2.2.3-x64

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "startup"; Description: "Iniciar automaticamente com o Windows"; GroupDescription: "Inicialização:"; Flags: unchecked

[Files]
Source: "target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "assets\icon.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "assets\driver\UnityCaptureFilter64.dll"; DestDir: "{app}\_local\driver"; Flags: uninsneveruninstall; Check: ShouldInstallDriver64
Source: "assets\driver\UnityCaptureFilter32.dll"; DestDir: "{app}\_local\driver"; Flags: uninsneveruninstall; Check: ShouldInstallDriver32

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\icon.ico"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon; IconFilename: "{app}\icon.ico"
Name: "{commonstartup}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Parameters: "--startup"; Tasks: startup; IconFilename: "{app}\icon.ico"

[Run]
Filename: "{sys}\regsvr32.exe"; Parameters: "/s /i:""UnityCaptureName=tVisio Virtual Camera"" ""{app}\_local\driver\UnityCaptureFilter64.dll"""; Flags: runhidden; Check: ShouldInstallDriver64
Filename: "{syswow64}\regsvr32.exe"; Parameters: "/s /i:""UnityCaptureName=tVisio Virtual Camera"" ""{app}\_local\driver\UnityCaptureFilter32.dll"""; Flags: runhidden; Check: ShouldInstallDriver32
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\regsvr32.exe"; Parameters: "/u /s ""{app}\_local\driver\UnityCaptureFilter64.dll"""; Flags: runhidden; RunOnceId: "UnregUnityCapture64"
Filename: "{syswow64}\regsvr32.exe"; Parameters: "/u /s ""{app}\_local\driver\UnityCaptureFilter32.dll"""; Flags: runhidden; RunOnceId: "UnregUnityCapture32"; Check: Is64BitInstallMode

[UninstallDelete]
Type: filesandordirs; Name: "{app}\_local"

[Code]
function ShouldInstallDriver64(): Boolean;
begin
  // Instala o driver apenas se o DLL ainda não existir no disco
  // Evita tentativas de substituição de DLL travada em memória pelo Chrome/Discord/OBS
  Result := not FileExists(ExpandConstant('{app}\_local\driver\UnityCaptureFilter64.dll'));
end;

function ShouldInstallDriver32(): Boolean;
begin
  Result := Is64BitInstallMode and not FileExists(ExpandConstant('{app}\_local\driver\UnityCaptureFilter32.dll'));
end;

function IsAppAlreadyInstalled(): Boolean;
begin
  Result := RegKeyExists(HKLM64, 'SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{5C2CD55C-92AD-4999-8666-912BD3E70000}_is1')
         or RegKeyExists(HKLM32, 'SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{5C2CD55C-92AD-4999-8666-912BD3E70000}_is1')
         or RegKeyExists(HKCU64, 'SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{5C2CD55C-92AD-4999-8666-912BD3E70000}_is1')
         or RegKeyExists(HKCU32, 'SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{5C2CD55C-92AD-4999-8666-912BD3E70000}_is1')
         or FileExists(ExpandConstant('{autopf}\tVisio\tVisio.exe'))
         or FileExists(ExpandConstant('{commonpf}\tVisio\tVisio.exe'))
         or FileExists(ExpandConstant('{commonpf64}\tVisio\tVisio.exe'))
         or FileExists('C:\Program Files\tVisio\tVisio.exe');
end;

function ShouldSkipPage(PageID: Integer): Boolean;
begin
  // Em modo de atualização, pula telas redundantes (Diretório, Menu Iniciar, Atalhos e Resumo)
  if IsAppAlreadyInstalled() then
  begin
    if (PageID = wpSelectDir) or (PageID = wpSelectProgramGroup) or (PageID = wpSelectTasks) or (PageID = wpReady) then
      Result := True
    else
      Result := False;
  end
  else
    Result := False;
end;

function InitializeSetup(): Boolean;
var
  ErrorCode: Integer;
begin
  // Encerra qualquer processo tVisio.exe em execução antes de instalar para evitar bloqueio de arquivo
  Exec('taskkill.exe', '/F /IM tVisio.exe /T', '', SW_HIDE, ewWaitUntilTerminated, ErrorCode);
  Sleep(350);
  Result := True;
end;

function InitializeUninstall(): Boolean;
var
  ErrorCode: Integer;
begin
  Exec('taskkill.exe', '/F /IM tVisio.exe /T', '', SW_HIDE, ewWaitUntilTerminated, ErrorCode);
  Sleep(350);
  Result := True;
end;
