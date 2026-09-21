; HDR Auto-Switch's deliberately current-user, in-place NSIS installer.
; Template inputs/registration layout were checked against Tauri CLI 2.11.4:
; crates/tauri-bundler/src/bundle/windows/nsis/{installer.nsi,utils.nsh}.
; Adapted Tauri packaging conventions: see LICENSE-Tauri.txt (MIT).
; Retain only safe Tauri shortcut helpers. The dangerous process macro is
; removed at preprocessing time; an accidental invocation is a compile error.
; Never run an old uninstaller: shipped v1 NSIS kills by basename.

Unicode true
ManifestDPIAware true
RequestExecutionLevel user
AllowRootDirInstall false
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "Win\COM.nsh"
!include "Win\Propkey.nsh"
!include "utils.nsh"
!macroundef CheckIfAppIsRunning

!define PRODUCTNAME "{{product_name}}"
!define MANUFACTURER "{{manufacturer}}"
!define MAINBINARYNAME "{{main_binary_name}}"
!define MAINBINARYSRCPATH "{{main_binary_path}}"
!define BUNDLEID "{{bundle_id}}"
!define PRODUCTKEY "Software\${MANUFACTURER}\${PRODUCTNAME}"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}"
!define WEBVIEW2APPGUID "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

!if "{{install_mode}}" != "currentUser"
  !error "HDR's ownership-safe NSIS template supports the shipped currentUser mode only"
!endif
!if "${PRODUCTNAME}" != "HDR Auto-Switch"
  !error "Review legacy installed-product identity before renaming this product"
!endif
!if "${MANUFACTURER}" != "soptik"
  !error "Review legacy installed-product identity before changing its publisher"
!endif
!if "${MAINBINARYNAME}" != "tauri-app"
  !error "Review predecessor and startup ownership before changing the main binary name"
!endif

Name "${PRODUCTNAME}"
OutFile "{{out_file}}"
InstallDir "$LOCALAPPDATA\${PRODUCTNAME}"
VIProductVersion "{{version_with_build}}"
VIAddVersionKey "ProductName" "${PRODUCTNAME}"
VIAddVersionKey "FileDescription" "${PRODUCTNAME}"
VIAddVersionKey "FileVersion" "{{version}}"
VIAddVersionKey "ProductVersion" "{{version}}"
VIAddVersionKey "LegalCopyright" "{{copyright}}"
BrandingText "${PRODUCTNAME}"

{{#if installer_icon}}
!define MUI_ICON "{{installer_icon}}"
{{/if}}
{{#if uninstaller_icon}}
!define MUI_UNICON "{{uninstaller_icon}}"
{{/if}}
{{#if uninstaller_sign_cmd}}
!uninstfinalize '{{uninstaller_sign_cmd}}'
{{/if}}

Var HdrUiLevel
Var HdrPassive
Var HdrNoShortcut
Var HdrArguments
Var HdrUninstallRecovery
Var HdrUninstallFailure
Var HdrUninstallRestoreFailed
Var HdrUninstallRestoreDir
Var HdrUninstallRestoreDirOwned
Var HdrUninstallRetirementStarted
Var HdrUninstallMetadataRestoreFailed

!macro HdrCreateUninstallDirectory DIRECTORY PARENT SUFFIX FAILURE
  ClearErrors
  GetTempFileName ${DIRECTORY} "${PARENT}"
  ${If} ${Errors}
    Goto ${FAILURE}
  ${EndIf}
  ClearErrors
  Delete "${DIRECTORY}"
  ${If} ${Errors}
    Goto ${FAILURE}
  ${EndIf}
  StrCpy ${DIRECTORY} "${DIRECTORY}.${SUFFIX}"
  System::Call 'kernel32::CreateDirectoryW(w "${DIRECTORY}", p 0) i .r0'
  ${If} $0 == 0
    Goto ${FAILURE}
  ${EndIf}
!macroend

!macro HdrBackupUninstallFile NAME
  System::Call 'kernel32::CopyFileW(w "$INSTDIR\${NAME}", w "$HdrUninstallRecovery\${NAME}", i 1) i .r0'
  ${If} $0 == 0
    Goto hdr_uninstall_prepare_failed
  ${EndIf}
!macroend

!macro HdrDeleteUninstallPayload PATH
  StrCpy $HdrUninstallFailure "${PATH}"
  ClearErrors
  Delete "${PATH}"
  ${If} ${Errors}
    Goto hdr_uninstall_failed
  ${EndIf}
  ; IfFileExists also returns false on lookup errors; only confirmed absence
  ; (ERROR_FILE_NOT_FOUND / ERROR_PATH_NOT_FOUND) authorizes metadata removal.
  System::Call 'kernel32::GetFileAttributesW(w "${PATH}") i .r0 ?e'
  Pop $1
  ${If} $0 != -1
    Goto hdr_uninstall_failed
  ${EndIf}
  ${If} $1 != 2
  ${AndIf} $1 != 3
    Goto hdr_uninstall_failed
  ${EndIf}
!macroend

!macro HdrRestoreUninstallFile NAME
  System::Call 'kernel32::GetFileAttributesW(w "$INSTDIR\${NAME}") i .r0 ?e'
  Pop $1
  ${If} $0 == -1
    ${If} $1 == 2
    ${OrIf} $1 == 3
      ${If} $HdrUninstallRestoreDirOwned != 1
        !insertmacro HdrCreateUninstallDirectory $HdrUninstallRestoreDir "$INSTDIR" "hdr-uninstall-restore" hdr_uninstall_restore_incomplete
        StrCpy $HdrUninstallRestoreDirOwned 1
      ${EndIf}
      ; Keep partial copies off registered paths. Staging under $INSTDIR makes
      ; the no-replace rename same-volume even when the backups are on C:.
      System::Call 'kernel32::CopyFileW(w "$HdrUninstallRecovery\${NAME}", w "$HdrUninstallRestoreDir\${NAME}.restore", i 1) i .r0'
      ${If} $0 != 0
        System::Call 'kernel32::MoveFileExW(w "$HdrUninstallRestoreDir\${NAME}.restore", w "$INSTDIR\${NAME}", i 0) i .r0'
      ${EndIf}
      ${If} $0 == 0
        StrCpy $HdrUninstallRestoreFailed 1
      ${EndIf}
    ${Else}
      StrCpy $HdrUninstallRestoreFailed 1
    ${EndIf}
  ${EndIf}
!macroend

!define MUI_PAGE_CUSTOMFUNCTION_PRE HdrSkipPassive
!insertmacro MUI_PAGE_WELCOME
{{#if license}}
!define MUI_PAGE_CUSTOMFUNCTION_PRE HdrSkipPassive
!insertmacro MUI_PAGE_LICENSE "{{license}}"
{{/if}}
!define MUI_PAGE_CUSTOMFUNCTION_PRE HdrSkipPassive
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_NOAUTOCLOSE
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_FUNCTION HdrRun
!define MUI_PAGE_CUSTOMFUNCTION_PRE HdrSkipPassive
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
{{#each languages}}
!insertmacro MUI_LANGUAGE "{{this}}"
{{/each}}

Function .onInit
  SetShellVarContext current
  ${If} ${RunningX64}
    !if "{{arch}}" == "x64"
      SetRegView 64
    !else if "{{arch}}" == "arm64"
      SetRegView 64
    !else
      SetRegView 32
    !endif
  ${EndIf}
  ReadRegStr $0 HKCU "${PRODUCTKEY}" ""
  ${If} $0 != ""
    StrCpy $INSTDIR $0
  ${EndIf}
  StrCpy $HdrUiLevel 5
  ${If} ${Silent}
    StrCpy $HdrUiLevel 2
  ${EndIf}
  ${GetOptions} $CMDLINE "/P" $HdrPassive
  ${IfNot} ${Errors}
    StrCpy $HdrPassive 1
    StrCpy $HdrUiLevel 2
  ${EndIf}
  ${GetOptions} $CMDLINE "/NS" $HdrNoShortcut
  ${IfNot} ${Errors}
    StrCpy $HdrNoShortcut 1
  ${EndIf}
FunctionEnd

Function HdrSkipPassive
  ${If} $HdrPassive == 1
    Abort
  ${EndIf}
FunctionEnd

Section "Verify orderly handoff" HdrGuard
  ; This helper exits before Tauri/plugins/config/HDR initialization. Do not
  ; replace it with an invocation of the installed v1 binary.
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File "/oname=hdr-install-guard.exe" "${MAINBINARYSRCPATH}"
  ClearErrors
  ExecWait '"$PLUGINSDIR\hdr-install-guard.exe" --hdr-installer-preflight nsis "$INSTDIR\${MAINBINARYNAME}.exe" $HdrUiLevel' $0
  ${If} ${Errors}
    StrCpy $0 1
  ${EndIf}
  ${If} $0 != 0
    DetailPrint "Installation blocked. Quit HDR Auto-Switch from its tray menu, then retry. No process was terminated."
    SetErrorLevel 1
    Quit
  ${EndIf}
SectionEnd

Section "WebView2 runtime" HdrWebView
  ; Retain the shipped default WebView bootstrap behavior, after our guard.
  ${If} ${RunningX64}
    ReadRegStr $0 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${Else}
    ReadRegStr $0 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${EndIf}
  ${If} $0 == ""
    ReadRegStr $0 HKCU "SOFTWARE\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${EndIf}
  ${If} $0 == ""
    !if "{{install_webview2_mode}}" == "downloadBootstrapper"
      NSISdl::download "https://go.microsoft.com/fwlink/p/?LinkId=2124703" "$PLUGINSDIR\MicrosoftEdgeWebview2Setup.exe"
      Pop $0
      ${If} $0 != "success"
        Abort "Unable to download Microsoft WebView2. No application files were replaced."
      ${EndIf}
      ExecWait '"$PLUGINSDIR\MicrosoftEdgeWebview2Setup.exe" {{webview2_installer_args}} /install' $0
      ${If} $0 != 0
        Abort "Microsoft WebView2 installation failed. No application files were replaced."
      ${EndIf}
    !else if "{{install_webview2_mode}}" != "skip"
      !error "Review WebView2 packaging before changing the shipped bootstrap mode"
    !endif
  ${EndIf}
SectionEnd

Section "HDR Auto-Switch" HdrInstall
  ; Recheck after the potentially long WebView download and directly before
  ; replacement. Retry never means permission to force termination.
  ClearErrors
  ExecWait '"$PLUGINSDIR\hdr-install-guard.exe" --hdr-installer-preflight nsis "$INSTDIR\${MAINBINARYNAME}.exe" $HdrUiLevel' $0
  ${If} ${Errors}
    StrCpy $0 1
  ${EndIf}
  ${If} $0 != 0
    SetErrorLevel 1
    Quit
  ${EndIf}

  SetOutPath "$INSTDIR"
  SetOverwrite on
  ClearErrors
  File "${MAINBINARYSRCPATH}"
  ${If} ${Errors}
    Abort "The application executable could not be replaced. The upgrade was not completed."
  ${EndIf}
  {{#each resources_dirs}}
  CreateDirectory "$INSTDIR\\{{this}}"
  {{/each}}
  {{#each resources}}
  File /a "/oname={{this.[1]}}" "{{no-escape @key}}"
  {{/each}}
  {{#each binaries}}
  File /a "/oname={{this}}" "{{no-escape @key}}"
  {{/each}}
  ; A sharing lock on only uninstall.exe must not leave the v1 killer in place
  ; while the installer reports success or offers to launch the new app.
  ClearErrors
  WriteUninstaller "$INSTDIR\uninstall.exe"
  ${If} ${Errors}
    SetErrorLevel 1
    Abort "The safe uninstaller could not be written. This upgrade did not complete. Do not run the old uninstaller; close the file lock and retry this installer."
  ${EndIf}
  WriteRegStr HKCU "${PRODUCTKEY}" "" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "MainBinaryName" "${MAINBINARYNAME}.exe"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${PRODUCTNAME}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${MANUFACTURER}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "{{version}}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" '$\"$INSTDIR\${MAINBINARYNAME}.exe$\"'
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" '$\"$INSTDIR$\"'
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '$\"$INSTDIR\uninstall.exe$\"'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "EstimatedSize" "{{estimated_size}}"

  ; Verify the newly written ownership metadata instead of reporting success
  ; after a denied/partial registry write. A failed check also prevents launch.
  ClearErrors
  ExecWait '"$PLUGINSDIR\hdr-install-guard.exe" --hdr-installer-preflight nsis "$INSTDIR\${MAINBINARYNAME}.exe" $HdrUiLevel' $0
  ${If} ${Errors}
    StrCpy $0 1
  ${EndIf}
  ${If} $0 != 0
    SetErrorLevel 1
    Quit
  ${EndIf}

  ${If} $HdrNoShortcut != 1
    CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
  ${EndIf}
  ${If} $HdrPassive == 1
    SetAutoClose true
  ${EndIf}
SectionEnd

Function HdrRun
  Exec '"$INSTDIR\${MAINBINARYNAME}.exe"'
FunctionEnd

Function .onInstSuccess
  ${If} $HdrPassive == 1
  ${OrIf} ${Silent}
    ${GetOptions} $CMDLINE "/R" $0
    ${IfNot} ${Errors}
      ${GetOptions} $CMDLINE "/ARGS" $HdrArguments
      Exec '"$INSTDIR\${MAINBINARYNAME}.exe" $HdrArguments'
    ${EndIf}
  ${EndIf}
FunctionEnd

Function un.onInit
  SetShellVarContext current
  ${If} ${RunningX64}
    !if "{{arch}}" == "x64"
      SetRegView 64
    !else if "{{arch}}" == "arm64"
      SetRegView 64
    !else
      SetRegView 32
    !endif
  ${EndIf}
  StrCpy $HdrUiLevel 5
  ${If} ${Silent}
    StrCpy $HdrUiLevel 2
  ${EndIf}
FunctionEnd

Section "Uninstall"
  ; Only the v2 uninstaller uses this command; v1 is never invoked by an upgrade.
  ; Keep the executable and ownership metadata until the check/owned startup
  ; retirement succeeds. Never delete a generic Run value here.
  ClearErrors
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --hdr-installer-uninstall nsis "$INSTDIR\${MAINBINARYNAME}.exe" $HdrUiLevel' $0
  ${If} ${Errors}
    StrCpy $0 1
  ${EndIf}
  ${If} $0 != 0
    DetailPrint "Uninstall blocked: orderly exit/owned startup retirement could not be verified."
    SetErrorLevel 1
    Quit
  ${EndIf}
  ; NSIS normally runs a temporary self-copy, so the registered uninstall.exe
  ; can be removed. _?= disables that behavior and must fail, not defer deletion.
  ; The ownership helper requires BOTH original executable paths on a retry.
  ; Keep recovery outside $PLUGINSDIR until success: Quit cleans $PLUGINSDIR.
  StrCpy $HdrUninstallRecovery ""
  StrCpy $HdrUninstallRetirementStarted 0
  StrCpy $HdrUninstallMetadataRestoreFailed 0
  ClearErrors
  InitPluginsDir
  ${If} ${Errors}
    Goto hdr_uninstall_prepare_failed
  ${EndIf}
  !insertmacro HdrCreateUninstallDirectory $HdrUninstallRecovery "$TEMP" "hdr-uninstall-recovery" hdr_uninstall_prepare_failed
  !insertmacro HdrBackupUninstallFile "${MAINBINARYNAME}.exe"
  !insertmacro HdrBackupUninstallFile "uninstall.exe"
  ; Persist exact ownership/registry bytes while the registered paths still
  ; exist. The copied helper can retire or repair that receipt after deletion.
  ClearErrors
  ExecWait '"$HdrUninstallRecovery\${MAINBINARYNAME}.exe" --hdr-installer-retirement-prepare nsis "$INSTDIR\${MAINBINARYNAME}.exe" $HdrUiLevel' $0
  ${If} ${Errors}
    StrCpy $0 1
  ${EndIf}
  ${If} $0 != 0
    Goto hdr_uninstall_prepare_failed
  ${EndIf}

  {{#each resources}}
  !insertmacro HdrDeleteUninstallPayload "$INSTDIR\\{{this.[1]}}"
  {{/each}}
  {{#each binaries}}
  !insertmacro HdrDeleteUninstallPayload "$INSTDIR\\{{this}}"
  {{/each}}
  !insertmacro HdrDeleteUninstallPayload "$INSTDIR\${MAINBINARYNAME}.exe"
  !insertmacro HdrDeleteUninstallPayload "$INSTDIR\uninstall.exe"

  ; The transaction is not complete just because the payload is absent.
  ; Deletion and readback of ALL owned metadata must succeed while the durable
  ; cleanup pair and exact registry receipt are still available for recovery.
  StrCpy $HdrUninstallFailure "Owned installation metadata retirement was not confirmed"
  StrCpy $HdrUninstallRetirementStarted 1
  ClearErrors
  ExecWait '"$HdrUninstallRecovery\${MAINBINARYNAME}.exe" --hdr-installer-retirement-commit nsis "$INSTDIR\${MAINBINARYNAME}.exe" $HdrUiLevel' $0
  ${If} ${Errors}
    StrCpy $0 1
  ${EndIf}
  ${If} $0 != 0
    Goto hdr_uninstall_failed
  ${EndIf}

  ; Retire the recovery pair together only after every installed payload is
  ; confirmed absent AND checked registry retirement succeeds. A failed move
  ; restores both cleanup paths before repairing the original registrations.
  StrCpy $HdrUninstallFailure "$HdrUninstallRecovery (recovery cleanup)"
  ClearErrors
  Rename "$HdrUninstallRecovery" "$PLUGINSDIR\hdr-uninstall-complete"
  ${If} ${Errors}
    Goto hdr_uninstall_failed
  ${EndIf}
  !insertmacro IsShortcutTarget "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  Pop $0
  ${If} $0 == 1
    Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  ${EndIf}
  !insertmacro IsShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  Pop $0
  ${If} $0 == 1
    Delete "$DESKTOP\${PRODUCTNAME}.lnk"
  ${EndIf}
  {{#each resources_ancestors}}
  RMDir "$INSTDIR\\{{this}}"
  {{/each}}
  RMDir "$INSTDIR"
  ; Neither legacy nor v2 user settings are removed by this installer.
  Goto hdr_uninstall_done

hdr_uninstall_prepare_failed:
  StrCpy $0 "Uninstall blocked: recovery copies or the exact registry receipt could not be prepared. No bundled file was removed; shortcuts and installation metadata were retained. Startup may already be disabled. Close file locks and retry. Any recovery copies remain at: $HdrUninstallRecovery"
  SetErrorLevel 1
  DetailPrint "$0"
  ${If} $HdrUiLevel == 5
    MessageBox MB_OK|MB_ICONSTOP "$0"
  ${EndIf}
  Quit

hdr_uninstall_failed:
  StrCpy $HdrUninstallRestoreFailed 0
  StrCpy $HdrUninstallRestoreDir ""
  StrCpy $HdrUninstallRestoreDirOwned 0
  !insertmacro HdrRestoreUninstallFile "${MAINBINARYNAME}.exe"
  !insertmacro HdrRestoreUninstallFile "uninstall.exe"
  Goto hdr_uninstall_restore_report

hdr_uninstall_restore_incomplete:
  StrCpy $HdrUninstallRestoreFailed 1

hdr_uninstall_restore_report:
  ${If} $HdrUninstallRetirementStarted == 1
    ; Verify complete, original cleanup bytes before re-registering anything.
    ; A denied rollback must preserve this helper/receipt, not report success.
    ClearErrors
    ExecWait '"$HdrUninstallRecovery\${MAINBINARYNAME}.exe" --hdr-installer-retirement-restore nsis "$INSTDIR\${MAINBINARYNAME}.exe" $HdrUiLevel' $0
    ${If} ${Errors}
      StrCpy $0 1
    ${EndIf}
    ${If} $0 != 0
      StrCpy $HdrUninstallMetadataRestoreFailed 1
    ${EndIf}
  ${EndIf}
  ${If} $HdrUninstallRestoreDirOwned == 1
    ClearErrors
    RMDir "$HdrUninstallRestoreDir"
    ${If} ${Errors}
      DetailPrint "Nonempty or locked recovery staging retained at $HdrUninstallRestoreDir. Do not use .restore files for manual repair."
    ${EndIf}
  ${EndIf}
  SetErrorLevel 1
  StrCpy $0 "Uninstall incomplete: $HdrUninstallFailure. Earlier files may be gone and startup may already be disabled. Shortcuts retained; existing files were not overwritten. Do not launch the app. Recovery: $HdrUninstallRecovery"
  ${If} $HdrUninstallRestoreFailed == 1
    StrCpy $0 "$0$\r$\nAutomatic recovery was blocked. After closing locks, restore the two recovery files only where originals are missing; do not overwrite conflicts or use .restore files. See RECOVERY.txt."
  ${Else}
    StrCpy $0 "$0$\r$\nMissing cleanup executables were restored. Keep the recovery directory until a retry succeeds."
  ${EndIf}
  ${If} $HdrUninstallMetadataRestoreFailed == 1
    StrCpy $0 "$0$\r$\nInstallation metadata recovery was NOT confirmed. Resolve registry access/conflicts and run the recovery-only command in RECOVERY.txt before retrying uninstall."
  ${Else}
    StrCpy $0 "$0$\r$\nInstallation metadata was retained or its exact recovery was confirmed. After recovery, close locks and retry normally without _?=."
  ${EndIf}
  DetailPrint "$0"
  ${If} $HdrUiLevel == 5
    MessageBox MB_OK|MB_ICONSTOP "$0"
  ${EndIf}
  Quit

hdr_uninstall_done:
SectionEnd
