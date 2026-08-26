; Inno Setup script for the Windows installer.
;
; Why an installer at all, when the zip already works. The zip is what the
; in-app updater downloads and unpacks (crates/rudbgen-app/src/update.rs), so
; it is not going anywhere. What it cannot do is register the program with
; Windows: unzipping leaves no entry under "Apps & features" (the ARP keys
; below HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall), and winget
; reads exactly those keys to decide which version is installed, whether an
; upgrade is available, and how to remove it. A package whose installer leaves
; no ARP entry is rejected by winget-pkgs validation and, if it slipped
; through, would report "no applicable upgrade" forever. So the installer
; exists for winget, and the zip stays for the updater.
;
; The one thing this script must not do is install a bare executable.
; rudbgen.exe is a launcher, not a self-contained program: the JVM loader in
; crates/rudbgen-jdbc/src/jvm.rs resolves lib\rudbgen-bridge.jar and runtime\
; relative to current_exe() (architecture.md 5, "Distribution"). Installing
; only the exe produces a program that starts, draws its window, and then dies
; on the first database connection. Hence the single recursive [Files] entry
; below: the installer lays down the staged tree byte for byte, in the same
; shape the zip carries.
;
; Compiled from CI with:
;
;   ISCC.exe /DVersion=0.1.3 ^
;            /DSourceDir=<staging tree> ^
;            /DOutputDir=<where the .exe lands> ^
;            /DOutputBaseFilename=rudbgen-v0.1.3-x86_64-pc-windows-msvc-setup
;
; Version carries no "v" prefix — VersionInfoVersion is a numeric quad and
; rejects one.

#ifndef Version
  #error Version is required: pass /DVersion=X.Y.Z
#endif
#ifndef SourceDir
  #error SourceDir is required: pass /DSourceDir=<staged tree>
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif
#ifndef OutputBaseFilename
  #define OutputBaseFilename "rudbgen-setup"
#endif

[Setup]
; This GUID is a published identifier, not an implementation detail. Inno
; derives the uninstall registry key from it, winget records it as the
; package's ProductCode, and both an upgrade in place and `winget uninstall`
; find the existing install by matching it. It is also the GUID
; crates/rudbgen-app/src/update.rs hard-codes in ARP_KEY, so the updater can
; correct the recorded version after a self-update. Changing it would orphan
; every copy already on disk — the old entry would linger in "Apps & features"
; with no way to remove it and the new one would install alongside. It never
; changes.
;
; It is rudbgen's own GUID and deliberately not rudbman's: a product code names
; a *product*, so sharing one would have winget treat an installed rudbman as
; an installed rudbgen and offer to upgrade one into the other.
;
; The doubled leading brace is Inno's escape for a literal "{".
AppId={{9A84AE34-E54B-46EA-B55D-61C0666D6890}
AppName=rudbgen
AppVersion={#Version}
VersionInfoVersion={#Version}
AppPublisher=Xcomart
AppPublisherURL=https://github.com/xcomart/rudbgen
AppSupportURL=https://github.com/xcomart/rudbgen
AppUpdatesURL=https://github.com/xcomart/rudbgen

; Per-user install, deliberately. PrivilegesRequired=lowest means no UAC
; prompt and no elevation, which is what winget's default (unelevated) install
; flow wants and what lets the app update itself later without asking for
; administrator rights. Under "lowest", {autopf} resolves to
; %LOCALAPPDATA%\Programs and {autoprograms} to the per-user Start menu, so
; the same script would also do the right thing if it were ever run elevated.
PrivilegesRequired=lowest
DefaultDirName={autopf}\rudbgen
; There is exactly one shortcut and it is not in a folder of its own, so the
; "Select Start Menu Folder" page has nothing to ask about. [Icons] names
; {autoprograms} directly rather than going through {group}.
DisableProgramGroupPage=yes

; gpui renders through DirectX and the build only targets x86_64-pc-windows-msvc;
; there is no 32-bit or ARM artifact to fall back to. x64compatible rather than
; x64 so the installer also runs under the x64 emulation layer on ARM64
; Windows, where the same binaries do work.
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

; The payload is a jlink runtime plus a JAR plus the executable — tens of
; megabytes of highly compressible files, and solid compression pays for
; itself across the thousands of small class and module files under runtime\.
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

; Paths are relative to this script, which lives in packaging\windows\.
SetupIconFile=..\..\assets\icon.ico
LicenseFile=..\..\LICENSE
UninstallDisplayIcon={app}\rudbgen.exe
UninstallDisplayName=rudbgen

OutputDir={#OutputDir}
OutputBaseFilename={#OutputBaseFilename}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
; Unchecked: a desktop icon is an opinion, and a silent winget install (which
; passes /VERYSILENT and therefore accepts every default) should not litter
; the desktop of someone who only typed `winget install`.
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; One recursive entry, on purpose. The jlink runtime is thousands of files
; deep under runtime\; enumerating them here would be both unreadable and a
; standing invitation to miss one when the module list in release.yml changes.
; What ships is whatever the "Package (windows)" step staged, which is the
; same tree the zip contains and the same tree the smoke test checked —
; including the reference copies under templates\ and sample\.
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\rudbgen"; Filename: "{app}\rudbgen.exe"
Name: "{autodesktop}\rudbgen"; Filename: "{app}\rudbgen.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\rudbgen.exe"; Description: "{cm:LaunchProgram,rudbgen}"; Flags: nowait postinstall skipifsilent

; No [UninstallDelete] section, and that is a decision rather than an omission.
; Uninstalling removes only what was installed: settings, themes, the saved
; connection list, the template files the user has edited and the Windows
; Credential Manager entries holding database passwords all live outside {app}
; (under %APPDATA%\rudbgen and in the credential store) and are left untouched.
; That is what makes an uninstall followed by a reinstall — which is how some
; upgrade paths behave — keep a user's connections and templates instead of
; silently wiping them.
