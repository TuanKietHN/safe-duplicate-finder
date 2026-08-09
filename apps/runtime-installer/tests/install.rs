use safe_dedupe_runtime_installer::install::is_success_exit_code;

#[test]
fn accepts_success_and_reboot_required_but_not_arbitrary_failures() {
    assert!(is_success_exit_code(0));
    assert!(is_success_exit_code(3010));
    assert!(!is_success_exit_code(1));
    assert!(!is_success_exit_code(1603));
}
