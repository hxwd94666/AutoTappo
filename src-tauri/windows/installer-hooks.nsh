; AutoTappo installer hooks
; DLL is loaded by the running pet, so stop the app and the pet before copying files.
!macro NSIS_HOOK_PREINSTALL
  ; 静默安装（/S）跳过提示；交互安装先明确告知会关闭桌宠。
  IfSilent +3
  MessageBox MB_YESNO|MB_ICONQUESTION "安装需要关闭 AutoTappo 和鸣潮桌宠 WuwaTappo，未同步的点击可能会延迟。是否继续安装？" IDYES +2
    Abort

  nsExec::Exec 'taskkill /F /T /IM AutoTappo.exe'
  nsExec::Exec 'taskkill /F /T /IM WuwaTappo.exe'
  Sleep 1500
!macroend
