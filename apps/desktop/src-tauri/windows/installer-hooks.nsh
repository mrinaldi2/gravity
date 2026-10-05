; The desktop and its matching managed daemon are one installation.
!include "LogicLib.nsh"

!macro GRAVITY_SERVICE ACTION
  Push $0
  Push $1
  DetailPrint "Hermes service: ${ACTION}"
  ; install stops the old daemon, may migrate the home, then waits for the
  ; new one's health check.
  nsExec::ExecToStack /TIMEOUT=900000 '"$INSTDIR\hermesd.exe" service ${ACTION}'
  Pop $0
  Pop $1
  ${If} $0 != 0
    DetailPrint "$1"
    MessageBox MB_OK|MB_ICONSTOP "The Hermes could not ${ACTION} the Hermes service.$\r$\n$\r$\n$1" /SD IDOK
    Pop $1
    Pop $0
    SetErrorLevel 1
    Abort "Hermes service ${ACTION} failed."
  ${EndIf}
  Pop $1
  Pop $0
!macroend

; Releases before the rename installed the product as "Gravity": its own
; directory, uninstall entry and shortcuts, and gravity-desktop.exe as the
; app. The Hermes installs beside it, so take that install over here: close
; the old app and remove its program files, shortcuts and registry entries.
; The background service is left to POSTINSTALL, whose "service install"
; replaces it. User data is never touched here: not the daemon home, nor
; the app's own data, which the app copies to its new bundle id on first
; launch. Every step is guarded, so a second run finds nothing and does
; nothing.
!define LEGACY_PRODUCTNAME "Gravity"
!define LEGACY_UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${LEGACY_PRODUCTNAME}"

; Removes a legacy shortcut only when it still opens the old app.
!macro GRAVITY_REMOVE_LEGACY_SHORTCUT LNK
  ${If} ${FileExists} "${LNK}"
    !insertmacro IsShortcutTarget "${LNK}" "$5\$6"
    Pop $0
    ${If} $0 = 1
      !insertmacro UnpinShortcut "${LNK}"
      Delete "${LNK}"
    ${EndIf}
  ${EndIf}
!macroend

!macro GRAVITY_REMOVE_LEGACY_FILE NAME
  ClearErrors
  Delete "$5\${NAME}"
  ${If} ${Errors}
    ; Still locked: remove it on the next reboot rather than fail the install.
    Delete /REBOOTOK "$5\${NAME}"
  ${EndIf}
!macroend

!macro GRAVITY_TAKE_OVER_LEGACY_INSTALL
  Push $0
  Push $1
  Push $2
  Push $3
  Push $5
  Push $6
  ; $5: the old install directory; the template saves it unquoted under the
  ; product key, and quoted as the uninstall entry's InstallLocation.
  ReadRegStr $5 HKCU "Software\${MANUFACTURER}\${LEGACY_PRODUCTNAME}" ""
  ${If} $5 == ""
    ReadRegStr $5 HKCU "${LEGACY_UNINSTKEY}" "InstallLocation"
    nsis_tauri_utils::StrReplace "$5" '"' ""
    Pop $5
  ${EndIf}
  ${If} $5 == ""
  ${AndIf} ${FileExists} "$LOCALAPPDATA\${LEGACY_PRODUCTNAME}\uninstall.exe"
    StrCpy $5 "$LOCALAPPDATA\${LEGACY_PRODUCTNAME}"
  ${EndIf}
  ; $6: the old app's exe name.
  ReadRegStr $6 HKCU "${LEGACY_UNINSTKEY}" "MainBinaryName"
  ${If} $6 == ""
    StrCpy $6 "gravity-desktop.exe"
  ${EndIf}

  ${If} $5 != ""
    DetailPrint "Replacing the earlier ${LEGACY_PRODUCTNAME} install in $5"
    !insertmacro CheckIfAppIsRunning "$6" "${LEGACY_PRODUCTNAME}"
    !insertmacro GRAVITY_REMOVE_LEGACY_SHORTCUT "$SMPROGRAMS\${LEGACY_PRODUCTNAME}.lnk"
    !insertmacro GRAVITY_REMOVE_LEGACY_SHORTCUT "$DESKTOP\${LEGACY_PRODUCTNAME}.lnk"
    ${If} $5 == $INSTDIR
      ; Installed into the same folder: our own files overwrite the rest.
      ${If} $6 != "${MAINBINARYNAME}.exe"
        !insertmacro GRAVITY_REMOVE_LEGACY_FILE "$6"
      ${EndIf}
    ${Else}
      ; Only the files the old installer put there; RMDir leaves the folder
      ; if anything else is in it.
      !insertmacro GRAVITY_REMOVE_LEGACY_FILE "$6"
      !insertmacro GRAVITY_REMOVE_LEGACY_FILE "gravity-desktop.exe"
      !insertmacro GRAVITY_REMOVE_LEGACY_FILE "hermes-desktop.exe"
      !insertmacro GRAVITY_REMOVE_LEGACY_FILE "gravityd.exe"
      !insertmacro GRAVITY_REMOVE_LEGACY_FILE "hermesd.exe"
      !insertmacro GRAVITY_REMOVE_LEGACY_FILE "LICENSE"
      !insertmacro GRAVITY_REMOVE_LEGACY_FILE "THIRD_PARTY_NOTICES.txt"
      !insertmacro GRAVITY_REMOVE_LEGACY_FILE "uninstall.exe"
      RMDir /REBOOTOK "$5"
    ${EndIf}
  ${EndIf}

  ; Registry entries go even when the folder was already gone, so Apps &
  ; features stops listing an install that no longer exists.
  DeleteRegKey HKCU "${LEGACY_UNINSTKEY}"
  DeleteRegKey HKCU "Software\${MANUFACTURER}\${LEGACY_PRODUCTNAME}"
  DeleteRegKey /ifempty HKCU "Software\${MANUFACTURER}"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${LEGACY_PRODUCTNAME}"
  Pop $6
  Pop $5
  Pop $3
  Pop $2
  Pop $1
  Pop $0
!macroend

; Releases before H-110 installed per user (%LOCALAPPDATA%\The Hermes, an
; HKCU uninstall entry). This setup installs per machine into Program Files,
; where only an administrator can change the app the daemon trusts as the
; owner's. Run the per-user copy's own uninstaller, silently: its hook stops
; and removes the background task (POSTINSTALL installs it again from here)
; and leaves the daemon home (%USERPROFILE%\.thehermes) alone. A silent
; uninstall never ticks "delete app data", so the app's own data stays too.
; HKCU is the owner's when they approve the UAC prompt themselves; if another
; account elevates, nothing is found and the owner uninstalls the old copy
; from Settings > Apps instead.
!macro HERMES_REMOVE_PER_USER_INSTALL
  Push $0
  Push $5
  ReadRegStr $5 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}" "InstallLocation"
  nsis_tauri_utils::StrReplace "$5" '"' ""
  Pop $5
  ${If} $5 != ""
  ${AndIf} $5 != $INSTDIR
  ${AndIf} ${FileExists} "$5\uninstall.exe"
    DetailPrint "Removing the earlier per-user install in $5"
    ; _?= runs the uninstaller in place, so ExecWait waits for it.
    ExecWait '"$5\uninstall.exe" /S _?=$5' $0
    ${If} $0 != 0
      DetailPrint "The per-user uninstaller exited with $0"
    ${EndIf}
    Delete "$5\uninstall.exe"
    RMDir "$5"
  ${EndIf}
  ; The entry goes even if its folder was already gone, so Apps & features
  ; lists one The Hermes.
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}"
  DeleteRegKey HKCU "Software\${MANUFACTURER}\${PRODUCTNAME}"
  DeleteRegKey /ifempty HKCU "Software\${MANUFACTURER}"
  Pop $5
  Pop $0
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro GRAVITY_TAKE_OVER_LEGACY_INSTALL
  !insertmacro HERMES_REMOVE_PER_USER_INSTALL
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; A home from before the rename makes "service install" move it and
  ; restart every bot, so ask first, Cancel by default. Declining installs
  ; the app only; the old service keeps running and the app asks again.
  ; A silent install proceeds, like a headless "service install".
  Push $R9
  StrCpy $R9 "install"
  ${If} ${FileExists} "$PROFILE\.gravity\bus.sqlite"
  ${AndIfNot} ${FileExists} "$PROFILE\.thehermes\*.*"
    MessageBox MB_OKCANCEL|MB_ICONQUESTION|MB_DEFBUTTON2 "Updating the Hermes service moves $PROFILE\.gravity to $PROFILE\.thehermes and restarts every bot (about 30 seconds, plus a backup of the database and transcripts).$\r$\n$\r$\nClose terminals, editors and Explorer windows open in that folder first.$\r$\n$\r$\nChoose Cancel to install the app only; it asks again when it starts." /SD IDOK IDOK +2
    StrCpy $R9 "skip"
  ${EndIf}
  ${If} $R9 == "install"
    !insertmacro GRAVITY_SERVICE install
  ${Else}
    DetailPrint "Hermes service: not updated; the app asks again when it starts"
  ${EndIf}
  Pop $R9
  ; Releases before the rename shipped the sidecar as gravityd.exe; the
  ; service now runs hermesd.exe from the daemon home, so drop the old copy.
  Delete "$INSTDIR\gravityd.exe"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Ask the user to close the desktop before stopping its background sessions.
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  ${If} ${FileExists} "$INSTDIR\hermesd.exe"
    !insertmacro GRAVITY_SERVICE uninstall
  ${EndIf}
!macroend
