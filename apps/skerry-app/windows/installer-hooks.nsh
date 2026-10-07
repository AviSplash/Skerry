; Skerry installer hooks (NSIS).
;
; Other computers connect to Skerry over the network, so Windows Firewall has
; to let it accept connections. If someone dismissed Windows' own firewall
; prompt, Windows also adds rules that *block* Skerry, and block rules win, so
; those are removed first. Changing firewall rules needs administrator rights:
; Windows asks once. Updates skip this when the rule already exists.

!macro NSIS_HOOK_POSTINSTALL
  nsExec::ExecToStack 'netsh advfirewall firewall show rule name="Skerry"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    ExecShellWait "runas" "powershell.exe" "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -Command $\"$$p = '$INSTDIR\${MAINBINARYNAME}.exe'; Get-NetFirewallApplicationFilter -ErrorAction SilentlyContinue | Where-Object { $$_.Program -and ([Environment]::ExpandEnvironmentVariables($$_.Program) -ieq $$p) } | Get-NetFirewallRule | Where-Object { $$_.Direction -eq 'Inbound' } | Remove-NetFirewallRule; New-NetFirewallRule -DisplayName 'Skerry' -Name 'Skerry' -Direction Inbound -Action Allow -Program $$p -Profile Any | Out-Null$\"" SW_HIDE
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Leave no rules behind (needs administrator rights; skipped if declined).
  nsExec::ExecToStack 'netsh advfirewall firewall show rule name="Skerry"'
  Pop $0
  Pop $1
  ${If} $0 == 0
    ExecShellWait "runas" "netsh.exe" 'advfirewall firewall delete rule name="Skerry"' SW_HIDE
  ${EndIf}
!macroend
