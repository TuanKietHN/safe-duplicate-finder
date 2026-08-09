!include LogicLib.nsh

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Đang kiểm tra và chuẩn bị Microsoft Edge WebView2 Runtime..."
  ExecWait '"$INSTDIR\resources\safe-dedupe-runtime-installer.exe"' $0

  ; Hai tệp này chỉ phục vụ lúc cài đặt. Runtime tải hoàn chỉnh được giữ trong cache
  ; theo SHA-256 để lần retry không phải tải lại.
  Delete "$INSTDIR\resources\safe-dedupe-runtime-installer.exe"
  RMDir "$INSTDIR\resources"

  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "Không thể chuẩn bị Runtime cần thiết (mã lỗi $0). Các tệp tải dở đã được giữ để lần sau tiếp tục; không có tệp Runtime chưa xác minh nào được chạy."
    SetErrors
    Abort
  ${EndIf}

  ; Shortcut gỡ cài đặt riêng, dễ tìm trong cùng nhóm Start menu với ứng dụng.
  SetShellVarContext current
  CreateDirectory "$SMPROGRAMS\$AppStartMenuFolder"
  CreateShortcut "$SMPROGRAMS\$AppStartMenuFolder\Gỡ cài đặt ${PRODUCTNAME}.lnk" "$INSTDIR\uninstall.exe"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    MessageBox MB_ICONEXCLAMATION|MB_YESNO "Gỡ cài đặt sẽ xóa chương trình, cơ sở dữ liệu, nhật ký, thiết lập, WebView/cache và dữ liệu cục bộ của ứng dụng.$\r$\n$\r$\nCác tệp thật trong vùng cách ly trên ổ nguồn sẽ được GIỮ NGUYÊN để tránh mất dữ liệu.$\r$\n$\r$\nBạn có muốn tiếp tục?" IDYES +2
    Abort
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    SetShellVarContext current

    ; Chỉ xóa các root cố định của sản phẩm. Tuyệt đối không dò ổ đĩa, thư mục nguồn,
    ; thư mục xuất báo cáo hay .safe-duplicate-finder-quarantine.
    RMDir /r "$APPDATA\${BUNDLEID}"
    RMDir /r "$LOCALAPPDATA\${BUNDLEID}"

    Delete "$SMPROGRAMS\$AppStartMenuFolder\Gỡ cài đặt ${PRODUCTNAME}.lnk"
    RMDir "$SMPROGRAMS\$AppStartMenuFolder"
    Delete "$SMPROGRAMS\Gỡ cài đặt ${PRODUCTNAME}.lnk"

    DeleteRegKey HKCU "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty HKCU "${MANUKEY}"
  ${EndIf}
!macroend
