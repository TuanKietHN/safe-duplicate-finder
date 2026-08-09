const HOOKS: &str = include_str!("../../desktop/src-tauri/windows/hooks.nsh");
const BASE_TAURI_CONFIG: &str = include_str!("../../desktop/src-tauri/tauri.conf.json");
const ONLINE_INSTALLER_CONFIG: &str =
    include_str!("../../desktop/src-tauri/tauri.online-installer.conf.json");
const BUILD_ONLINE_INSTALLER: &str =
    include_str!("../../../installer/windows/build-online-installer.ps1");

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

#[test]
fn normal_desktop_checks_do_not_require_a_prebuilt_runtime_helper() {
    assert!(!BASE_TAURI_CONFIG.contains("safe-dedupe-runtime-installer.exe"));
}

#[test]
fn online_installer_build_explicitly_bundles_the_runtime_helper() {
    assert!(ONLINE_INSTALLER_CONFIG.contains("safe-dedupe-runtime-installer.exe"));
    assert!(ONLINE_INSTALLER_CONFIG.contains("resources/safe-dedupe-runtime-installer.exe"));
    assert!(BUILD_ONLINE_INSTALLER.contains("tauri.online-installer.conf.json"));
}
