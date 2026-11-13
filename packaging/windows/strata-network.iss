; Windows installer of Strata Network (Inno Setup 6).
; ISCC /DVersion=1.2.0 /DSource=<folder with strata-network.exe and strata-tray.exe> strata-network.iss

#ifndef Version
  #define Version "0.0.0"
#endif

[Setup]
AppId={{6C1E2B57-4F0B-4C55-9A0D-5B3B1F2E7A41}
AppName=Strata Network
AppVersion={#Version}
AppPublisher=Strata
DefaultDirName={autopf}\Strata Network
DefaultGroupName=Strata Network
DisableProgramGroupPage=yes
OutputBaseFilename=Strata-{#Version}-windows-x86_64-setup
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
Compression=lzma2
SolidCompression=yes
UninstallDisplayIcon={app}\strata-network.exe
WizardStyle=modern

[Languages]
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "{#Source}\strata-network.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\strata-tray.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Strata Network"; Filename: "{app}\strata-tray.exe"
Name: "{commonstartup}\Strata Network"; Filename: "{app}\strata-tray.exe"

[Run]
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""Strata Network"""; Flags: runhidden waituntilterminated
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""Strata Network"" dir=in action=allow program=""{app}\strata-network.exe"" enable=yes"; Flags: runhidden waituntilterminated
Filename: "{app}\strata-network.exe"; Parameters: "service install"; Flags: runhidden waituntilterminated
Filename: "{app}\strata-tray.exe"; Flags: nowait postinstall skipifsilent; Description: "Strata Network"

[UninstallRun]
Filename: "{app}\strata-network.exe"; Parameters: "service uninstall"; Flags: runhidden waituntilterminated; RunOnceId: "service"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""Strata Network"""; Flags: runhidden waituntilterminated; RunOnceId: "firewall"
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im strata-tray.exe"; Flags: runhidden waituntilterminated; RunOnceId: "tray"

[Code]
// Stops the running service and launcher so that their files can be replaced (upgrade).
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\sc.exe'), 'stop StrataNetwork', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im strata-tray.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(3000);
  Result := '';
end;
