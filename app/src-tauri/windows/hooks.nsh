; NSIS hooks for the Windows installer (tauri.conf.json bundle.windows.nsis.installerHooks).

; Tauri's uninstaller keeps HKCU\Software\SimRep\SimRep (the install folder, used to suggest the
; same folder on reinstall) unless "Delete application data" is ticked, and a silent uninstall
; cannot tick it. Remove that key on every real uninstall so nothing of the installer is left; an
; update (/UPDATE) keeps it. User data (%LOCALAPPDATA%\dev.simrep.app: projects' recovery file,
; WebView storage) is still only deleted through that checkbox.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegKey HKCU "Software\SimRep\SimRep"
    DeleteRegKey /ifempty HKCU "Software\SimRep"
  ${EndIf}
!macroend
