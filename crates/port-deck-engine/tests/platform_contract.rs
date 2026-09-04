use port_deck_core::HostPlatform;
use port_deck_engine::{TerminateRequest, host_platform, terminate};

#[test]
fn selects_the_scanner_for_the_compile_target() {
    let expected = if cfg!(target_os = "windows") {
        HostPlatform::Windows
    } else if cfg!(target_os = "macos") {
        HostPlatform::MacOs
    } else {
        HostPlatform::Linux
    };

    assert_eq!(host_platform(), expected);
}

#[test]
fn refuses_a_resource_from_another_host_platform() {
    let other = match host_platform() {
        HostPlatform::Windows => HostPlatform::Linux,
        HostPlatform::Linux | HostPlatform::MacOs => HostPlatform::Windows,
    };

    let error = terminate(TerminateRequest {
        platform: other,
        pid: 99_999,
        start_token: "not-used".into(),
        manager_unit: None,
    })
    .expect_err("cross-platform termination must be rejected before touching a process");

    assert!(error.contains("不属于当前宿主平台"));
}
