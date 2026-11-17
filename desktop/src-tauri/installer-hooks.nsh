; NSIS hooks for the Tauri installer (bundle.windows.nsis.installerHooks).

; On uninstall, remove the daily scheduled task the app may have created, so nothing keeps
; starting a deleted exe. Skipped when the uninstaller runs as part of an update
; ($UpdateMode is set by Tauri's uninstaller), where the task stays valid for the new
; version. A missing task is not an error.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    nsExec::Exec 'schtasks /Delete /F /TN StrazceKatastru'
    Pop $0
  ${EndIf}
!macroend
