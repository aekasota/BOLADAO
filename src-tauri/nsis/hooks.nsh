; Hooks do instalador NSIS do Boladão
!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Preparando componentes embutidos (yt-dlp, FFmpeg, Deno) e pastas de mídia..."
  nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --setup-init'
  Pop $0
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    ; Limpa binários extraídos e cache temporário (~360 MB), preservando mídias em Músicas/Vídeos
    RMDir /r "$LOCALAPPDATA\Boladao\bin"
    RMDir /r "$LOCALAPPDATA\Boladao\cache"

    ; Se o usuário marcou "Excluir dados do aplicativo", limpa também configurações, cookies e banco SQLite
    ${If} $DeleteAppDataCheckboxState = 1
      RMDir /r "$APPDATA\Boladao"
      RMDir /r "$LOCALAPPDATA\Boladao"
    ${EndIf}
  ${EndIf}
!macroend