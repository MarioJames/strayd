use crate::{
    cases::Environment,
    process::{Ready, alive, kill_owned},
    report::Report,
    until,
};
use anyhow::{Context, Result, ensure};
use port_deck_engine::{TerminateRequest, scan_all, terminate};
use serde_json::json;
use std::{
    fs,
    process::{Command, Stdio},
    time::Duration,
};

pub fn stability(env: &Environment, report: &mut Report) {
    report.case(
        "repeated-concurrent-listeners",
        "native-fixture",
        |observed| {
            for round in 0..5 {
                let mut children = Vec::new();
                for index in 0..8 {
                    children.push(env.fixture(
                        &format!("load-{round}-{index}"),
                        "same-name",
                        None,
                        &["--fixture-ports", "3"],
                    )?);
                }
                let snapshot = scan_all();
                for child in &mut children {
                    let service = snapshot
                        .groups
                        .iter()
                        .flat_map(|group| &group.services)
                        .find(|service| service.pid == child.child.id())
                        .context("load fixture missing")?;
                    ensure!(service.ports.len() == 3, "lost listener under load");
                    child.cleanup()?;
                }
            }
            observed.push(
                json!({"rounds":5,"concurrent_processes":8,"ports_per_process":3,"released":true}),
            );
            Ok(())
        },
    );
}

pub fn systemd(env: &Environment, report: &mut Report) {
    if !cfg!(target_os = "linux") {
        report.unavailable(
            "systemd-owned-unit",
            "native-fixture",
            true,
            "requires a Linux systemd VM",
        );
        return;
    }
    let user =
        std::env::var("STRAYD_TEST_SYSTEMD_SCOPE").unwrap_or_else(|_| "user".into()) != "system";
    let ctl = |program: &str| {
        let mut cmd = Command::new(program);
        cmd.env("LC_ALL", "C");
        if user {
            cmd.arg("--user");
        }
        cmd
    };
    if !ctl("systemctl")
        .args(["show-environment"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
    {
        report.unavailable("systemd-owned-unit", "native-fixture", true, "selected systemd manager is unavailable; use a disposable VM with an active user manager, or STRAYD_TEST_SYSTEMD_SCOPE=system");
        return;
    }
    report.case("systemd-owned-unit", "native-fixture", |observed| {
        let unit = format!("strayd-test-{}-{}.service", std::process::id(), crate::unix_seconds());
        let ready_file = env.root.join("systemd-ready.json");
        let ledger = env.output.join("units.json");
        fs::write(&ledger, serde_json::to_vec(&json!({"unit":unit,"user":user}))?)?;
        let result = (|| -> Result<()> {
            let status = ctl("systemd-run").args(["--quiet", "--collect", "--unit", &unit, "--property", "RuntimeMaxSec=120"])
                .arg(&env.fixture).args(["--fixture-detached", "--fixture-ready-file"]).arg(&ready_file).status()?;
            ensure!(status.success(), "could not start dedicated transient unit");
            until(Duration::from_secs(10), || Ok(ready_file.exists()))?;
            let ready: Ready = serde_json::from_slice(&fs::read(&ready_file)?)?;
            let identity = env.registry.register(ready.pid)?;
            let service = scan_all().groups.into_iter().flat_map(|g| g.services).find(|s| s.pid == ready.pid).context("unit process not discovered")?;
            ensure!(service.manager_unit.as_deref() == Some(&unit), "scanner returned another unit; refusing stop");
            let wrong = terminate(TerminateRequest { platform:service.platform, pid:ready.pid, start_token:service.start_token.clone(), manager_unit:Some(format!("{}-stale.service", unit.trim_end_matches(".service"))) });
            ensure!(wrong == Err(port_deck_engine::EngineError::ManagedRelationChanged) && alive(&identity), "stale unit relation was not rejected: {wrong:?}");
            terminate(TerminateRequest { platform:service.platform, pid:ready.pid, start_token:service.start_token.clone(), manager_unit:Some(unit.clone()) })?;
            until(Duration::from_secs(4), || Ok(!alive(&identity)))?;
            observed.push(json!({"unit":unit,"user_manager":user,"stale_relation_rejected":true,"stopped":true}));
            Ok(())
        })();
        let cleanup = cleanup_units(&ledger);
        result.and(cleanup)
    });
}

pub fn cleanup_units(path: &std::path::Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    let unit = value["unit"].as_str().context("unit name")?;
    ensure!(
        unit.starts_with("strayd-test-")
            && unit.ends_with(".service")
            && unit
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'),
        "invalid owned unit"
    );
    let mut command = Command::new("systemctl");
    command.env("LC_ALL", "C");
    if value["user"] == true {
        command.arg("--user");
    }
    // RuntimeMaxSec also bounds the unit if the outer runner is forcibly killed.
    let result = command.args(["stop", "--", unit]).output()?;
    ensure!(
        result.status.success() || String::from_utf8_lossy(&result.stderr).contains("not loaded"),
        "unit cleanup failed"
    );
    fs::remove_file(path)?;
    Ok(())
}

pub fn permissions(env: &Environment, report: &mut Report) {
    if !cfg!(target_os = "linux") || std::env::var("STRAYD_TEST_ISOLATED").as_deref() != Ok("1") {
        report.unavailable(
            "cross-user-stop-denied",
            "native-fixture",
            true,
            "requires the dedicated permissions container; never creates host accounts",
        );
        return;
    }
    report.case("cross-user-stop-denied", "native-fixture", |observed| {
        let mut child = env.fixture("root-owned", "root-listener", None, &[])?;
        let service = env.scan(&child, observed)?;
        let mut command = assert_cmd::Command::new("runuser");
        command
            .args(["-u", "tester", "--"])
            .args(&env.cli)
            .args([
                "--no-config",
                "stop",
                "resource",
                "--id",
                &service.id,
                "--yes",
            ])
            .timeout(Duration::from_secs(10));
        let output = command.output()?;
        ensure!(
            !output.status.success() && alive(&child.identity),
            "cross-user stop did not preserve root process"
        );
        let error = String::from_utf8_lossy(&output.stderr);
        ensure!(
            error.contains("No resources matched") || error.contains("signal") || error.contains("权限") || error.contains("permission"),
            "unexpected restricted CLI result: {error}"
        );
        let target = env.output.join("permission-target.json");
        fs::write(&target, serde_json::to_vec(&child.identity)?)?;
        let probe = Command::new("runuser").args(["-u", "tester", "--"]).arg(std::env::current_exe()?)
            .arg("--permission-probe").arg(&target).arg("--output").arg(&env.output)
            .arg("--repository").arg(&env.repository).output()?;
        ensure!(probe.status.success() && alive(&child.identity), "native engine did not reject root-owned process: {}", String::from_utf8_lossy(&probe.stderr));
        observed.push(
            json!({"scanner_user":"tester","target_user":"root","engine_signal_denied":true,"restricted_cli_stderr":error}),
        );
        child.cleanup()
    });
}

pub fn permission_probe(path: &std::path::Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        ensure!(
            std::env::var("STRAYD_TEST_ISOLATED").as_deref() == Ok("1"),
            "probe requires isolated test container"
        );
        // The probe only attempts an operation it cannot perform: unprivileged -> root.
        ensure!(
            unsafe { libc::geteuid() } != 0,
            "probe must run unprivileged"
        );
        let target: crate::process::Identity = serde_json::from_slice(&fs::read(path)?)?;
        let status = fs::read_to_string(format!("/proc/{}/status", target.pid))?;
        ensure!(
            status
                .lines()
                .find(|line| line.starts_with("Uid:"))
                .is_some_and(|line| line.split_whitespace().skip(1).all(|uid| uid == "0")),
            "probe target must be root-owned"
        );
        let result = terminate(TerminateRequest {
            platform: port_deck_engine::host_platform(),
            pid: target.pid,
            start_token: target.start.to_string(),
            manager_unit: None,
        });
        ensure!(
            result == Err(port_deck_engine::EngineError::SignalDenied),
            "expected SignalDenied, got {result:?}"
        );
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        anyhow::bail!("requires Linux");
    }
}

pub fn desktop(env: &Environment, report: &mut Report) {
    if !cfg!(target_os = "macos") {
        report.unavailable(
            "macos-launchservices",
            "native-fixture",
            true,
            "requires a macOS runner with a GUI login session",
        );
        return;
    }
    report.case("macos-launchservices", "native-fixture", |observed| {
        let app = env.root.join("测试应用.app");
        let contents = app.join("Contents"); fs::create_dir_all(contents.join("MacOS"))?;
        fs::copy(&env.fixture, contents.join("MacOS/fixture"))?;
        fs::write(contents.join("Info.plist"), r#"<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>fixture</string><key>CFBundleIdentifier</key><string>test.strayd.fixture</string><key>CFBundlePackageType</key><string>APPL</string><key>LSUIElement</key><true/></dict></plist>"#)?;
        let ready_file = env.root.join("launchservices-ready.json");
        ensure!(Command::new("open").args(["-n", "-a"]).arg(&app).args(["--args", "--fixture-detached", "--fixture-ready-file"]).arg(&ready_file).status()?.success(), "LaunchServices failed");
        until(Duration::from_secs(10), || Ok(ready_file.exists()))?;
        let ready: Ready = serde_json::from_slice(&fs::read(ready_file)?)?;
        let identity = env.registry.register(ready.pid)?;
        let service = scan_all().groups.into_iter().flat_map(|g| g.services).find(|s| s.pid == ready.pid).context("LaunchServices process missing")?;
        ensure!(service.display_name == "测试应用", "wrong app name: {}", service.display_name);
        observed.push(serde_json::to_value(service)?);
        kill_owned(&identity)
    });
}

pub fn real_app(env: &Environment, report: &mut Report) {
    let Some(manifest) = std::env::var_os("STRAYD_REAL_APP_MANIFEST") else {
        for id in ["real-codefuse", "real-wave-music", "real-electron-helper"] {
            report.unavailable(id, "real-app", true, "no versioned application launcher manifest; synthetic fixtures do not validate this application");
        }
        return;
    };
    let parsed =
        (|| -> Result<serde_json::Value> { Ok(serde_json::from_slice(&fs::read(manifest)?)?) })();
    let manifest = match parsed {
        Ok(value) => value,
        Err(error) => {
            report.case("real-launcher-manifest", "real-app", |_| Err(error));
            return;
        }
    };
    for id in ["real-codefuse", "real-wave-music", "real-electron-helper"] {
        let value = manifest["applications"]
            .as_array()
            .and_then(|apps| apps.iter().find(|app| app["id"] == id));
        let Some(value) = value else {
            report.unavailable(
                id,
                "real-app",
                true,
                "application is not configured in the versioned launcher manifest",
            );
            continue;
        };
        report.case(id, "real-app", |observed| {
        let exe = value["executable"].as_str().context("manifest executable")?;
        let expected = value["expected_name"].as_str().context("manifest expected_name")?;
        let version = value["version"].as_str().filter(|v| !v.is_empty()).context("manifest version")?;
        let mut command = Command::new(exe);
        for arg in value["args"].as_array().context("manifest args array")? { command.arg(arg.as_str().context("argument must be text")?); }
        command.current_dir(value["cwd"].as_str().context("manifest cwd")?);
        let mut child = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        if let Err(error) = env.registry.register(child.id()) { let _ = child.kill(); let _ = child.wait(); return Err(error); }
        let result = (|| -> Result<()> {
            let mut found = None;
            until(Duration::from_secs(15), || {
                found = scan_all().groups.into_iter().flat_map(|g| g.services).find(|s| s.pid == child.id()); Ok(found.is_some())
            })?;
            let service = found.unwrap();
            ensure!(service.display_name == expected, "expected {expected}, got {}", service.display_name);
            // Only the owned PID is recorded. Do not persist arbitrary app command lines/secrets.
            observed.push(json!({"version":version,"version_source":"operator-verified-manifest","display_name":service.display_name,"pid":service.pid,"ports":service.ports,"started_at":service.started_at}));
            Ok(())
        })();
        let cleanup = env.registry.cleanup(); let _ = child.wait(); result.and(cleanup)
        });
    }
}

pub fn wsl(env: &Environment, report: &mut Report) {
    if !sysinfo::System::kernel_version()
        .unwrap_or_default()
        .to_lowercase()
        .contains("microsoft")
    {
        report.unavailable(
            "wsl-native-boundary",
            "native-fixture",
            true,
            "requires WSL2",
        );
        return;
    }
    if Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", "exit 0"])
        .status()
        .is_err()
    {
        report.unavailable(
            "wsl-native-boundary",
            "native-fixture",
            true,
            "Windows PowerShell interoperability is unavailable",
        );
        return;
    }
    report.case("wsl-native-boundary", "native-fixture", |observed| {
        use std::io::{BufRead, BufReader};
        let marker = format!("strayd-windows-boundary-{}", std::process::id());
        let script = format!("$ErrorActionPreference='Stop'; $marker='{marker}'; $l=[System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback,0); $l.Start(); try {{ Write-Output ($l.LocalEndpoint.Port); $t=[Console]::In.ReadLineAsync(); $null=$t.Wait(15000) }} finally {{ $l.Stop() }}");
        let mut windows = Command::new("powershell.exe").args(["-NoProfile", "-Command", &script]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let identity = env.registry.register(windows.id())?;
        let stdout = windows.stdout.take().context("Windows readiness")?;
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || { let mut line = String::new(); let _ = BufReader::new(stdout).read_line(&mut line); let _ = send.send(line); });
        let result = (|| -> Result<()> {
            let port: u16 = receive.recv_timeout(Duration::from_secs(10))?.trim().parse()?;
            let mut linux = env.fixture("wsl-linux", "wsl-listener", None, &["--fixture-bind-port", &port.to_string(), "--fixture-bind-host", "127.0.0.2"])?;
            std::net::TcpStream::connect(("127.0.0.2", port))?;
            let snapshot = scan_all();
            ensure!(snapshot.groups.iter().flat_map(|g| &g.services).any(|s| s.pid == linux.child.id()), "Linux process missing in WSL");
            ensure!(!snapshot.groups.iter().flat_map(|g| &g.services).any(|s| s.command.contains(&marker)), "Windows-side listener leaked into Linux discovery");
            observed.push(json!({"linux_pid":linux.child.id(),"shared_port":port,"linux_host":"127.0.0.2","windows_host":"127.0.0.1","windows_excluded":true})); linux.cleanup()
        })();
        drop(windows.stdin.take());
        if until(Duration::from_secs(3), || Ok(windows.try_wait()?.is_some())).is_err() { kill_owned(&identity)?; windows.wait()?; }
        result
    });
}
