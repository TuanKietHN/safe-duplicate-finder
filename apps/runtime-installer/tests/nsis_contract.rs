const HOOKS: &str = include_str!("../../desktop/src-tauri/windows/hooks.nsh");

#[test]
fn creates_a_dedicated_uninstall_shortcut() {
    assert!(HOOKS.contains("Gỡ cài đặt ${PRODUCTNAME}.lnk"));
    assert!(HOOKS.contains("$INSTDIR\\uninstall.exe"));
}

#[test]
fn explicit_uninstall_deletes_only_fixed_product_data_roots() {
    assert!(HOOKS.contains("RMDir /r \"$APPDATA\\${BUNDLEID}\""));
    assert!(HOOKS.contains("RMDir /r \"$LOCALAPPDATA\\${BUNDLEID}\""));
    assert!(HOOKS.contains("${If} $UpdateMode <> 1"));
    assert!(!HOOKS.contains("RMDir /r \"$EXEDIR"));
    assert!(!HOOKS.contains("RMDir /r \"$INSTDIR\\.safe-duplicate-finder-quarantine"));
}

#[test]
fn runtime_helper_failure_aborts_without_claiming_success() {
    assert!(HOOKS.contains("ExecWait"));
    assert!(HOOKS.contains("${If} $0 != 0"));
    assert!(HOOKS.contains("SetErrors"));
    assert!(HOOKS.contains("Abort"));
}
