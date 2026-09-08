//! Application-shaped OS processes; no vendor binaries or scanner substitutions.
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{fs, net::TcpStream, time::Duration};

use crate::{
    cases::{self, Environment, executable_name},
    process::{ManagedChild, alive, identity},
    report::Report,
    until,
};

fn verify(env: &Environment, child: &ManagedChild, expected: &str) -> Result<()> {
    child.assert_listening()?;
    let services = env.cli_services(&["--no-config", "list", "--json"])?;
    let service = services
        .iter()
        .find(|s| s.pid == child.ready.pid)
        .context("application mock missing from actual CLI")?;
    ensure!(
        service.display_name == expected,
        "expected {expected}, got {}",
        service.display_name
    );
    ensure!(
        service.ports == child.ready.ports,
        "CLI lost actual application bindings"
    );
    ensure!(
        service.start_token == child.identity.start.to_string(),
        "wrong OS process identity"
    );
    Ok(())
}

pub fn run(env: &Environment, report: &mut Report) {
    cases::native_binary(env, report);
    report.case("mock-unicode-app-bundle", "native-fixture", |observed| {
        // The bundle layout is intentionally also executable on Linux/Windows:
        // it tests the path contract there, not Apple's LaunchServices.
        let mut child = env.fixture(
            "Applications/测试播放器.app/Contents/MacOS",
            "测试播放器",
            None,
            &[],
        )?;
        verify(env, &child, "测试播放器")?;
        let service = env.scan(&child, observed)?;
        ensure!(service.display_name == "测试播放器", "bundle name was lost");
        env.terminate(&service, &child)?;
        until(Duration::from_secs(3), || Ok(!alive(&child.identity)))?;
        child.cleanup()?;
        ensure!(
            TcpStream::connect(("127.0.0.1", service.ports[0])).is_err(),
            "stopped port remains open"
        );
        Ok(())
    });
    report.case("mock-electron-helper-tree", "native-fixture", |observed| {
        let bundle = "Applications/Strayd Test Desk.app/Contents";
        let helper_dir = env
            .root
            .join(bundle)
            .join("Frameworks/Test Helper.app/Contents/MacOS");
        fs::create_dir_all(&helper_dir)?;
        let helper_exe = helper_dir.join(executable_name("Test Helper"));
        fs::copy(&env.fixture, &helper_exe)?;
        // A second instance has the same name but must survive stopping this tree.
        let mut control = env.fixture(&format!("control/{bundle}/MacOS"), "Electron", None, &[])?;
        let mut parent = env.fixture(
            &format!("{bundle}/MacOS"),
            "Electron",
            None,
            &[
                "--fixture-children",
                "1",
                "--fixture-child-executable",
                helper_exe.to_str().context("helper path encoding")?,
                "--fixture-child-arg",
                "--type=utility",
                "--fixture-child-arg",
                "--utility-sub-type=node.mojom.NodeService",
            ],
        )?;
        verify(env, &parent, "Strayd Test Desk")?;
        let helper = identity(parent.ready.children[0])?;
        let services = env.cli_services(&["--no-config", "list", "--json"])?;
        let service = services
            .iter()
            .find(|s| s.pid == helper.pid)
            .context("helper missing from CLI")?;
        ensure!(
            service.parent_pid == parent.ready.pid,
            "helper lost its real parent"
        );
        ensure!(
            service.display_name == "Strayd Test Desk",
            "helper must use the outer app name"
        );
        ensure!(
            service.command.contains("--type=utility"),
            "helper arguments missing"
        );
        ensure!(service.ports.len() == 1, "helper listener missing");
        TcpStream::connect(("127.0.0.1", service.ports[0]))?;
        observed.push(serde_json::to_value(service)?);
        let service = env.scan(&parent, observed)?;
        env.terminate(&service, &parent)?;
        until(Duration::from_secs(3), || {
            Ok(!alive(&parent.identity) && !alive(&helper))
        })?;
        verify(env, &control, "Strayd Test Desk")?;
        observed.push(
            json!({"helper_pid":helper.pid,"control_pid":control.ready.pid,"control_alive":true}),
        );
        parent.cleanup()?;
        control.cleanup()
    });
}
