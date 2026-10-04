; The desktop and its matching managed daemon are one installation.
!include "LogicLib.nsh"

!macro GRAVITY_SERVICE ACTION
  Push $0
  Push $1
  DetailPrint "Gravity background daemon: ${ACTION}"
  nsExec::ExecToStack /TIMEOUT=60000 '"$INSTDIR\hermesd.exe" service ${ACTION}'
  Pop $0
  Pop $1
  ${If} $0 != 0
    DetailPrint "$1"
    MessageBox MB_OK|MB_ICONSTOP "Gravity could not ${ACTION} its background daemon.$\r$\n$\r$\n$1" /SD IDOK
    Pop $1
    Pop $0
    SetErrorLevel 1
    Abort "Gravity background daemon ${ACTION} failed."
  ${EndIf}
  Pop $1
  Pop $0
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro GRAVITY_SERVICE install
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
