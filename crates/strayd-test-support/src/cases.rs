use anyhow::{Context, Result, ensure};
use port_deck_core::ServiceProcess;
use port_deck_engine::{EngineError, TerminateRequest, host_platform, scan_all, terminate};
use serde_json::{Value, json};
use std::{
    fs,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Output},
    time::Duration,
};

use crate::{
    process::{ManagedChild, Registry, alive, identity},
    report::Report,
    until,
};

pub struct Environment {
    pub fixture: PathBuf,
    pub cli: Vec<String>,
    pub root: PathBuf,
    pub output: PathBuf,
    pub repository: PathBuf,
    pub registry: Registry,
}

impl Environment {
    pub fn fixture(
        &self,
        folder: &str,
        name: &str,
        cwd: Option<&Path>,
        args: &[&str],
    ) -> Result<ManagedChild> {
        let directory = self.root.join(folder);
        fs::create_dir_all(&directory)?;
        let binary = directory.join(executable_name(name));
        if !binary.exists() {
            fs::copy(&self.fixture, &binary)?;
        }
        let mut command = Command::new(&binary);
        command.current_dir(cwd.unwrap_or(&directory)).args(args);
        ManagedChild::start(
            &mut command,
            &self.registry,
            &directory.join("stderr.log"),
            Duration::from_secs(5),
        )
    }

    pub fn command(&self, args: &[&str]) -> Result<Output> {
        let mut command = assert_cmd::Command::new(&self.cli[0]);
        command
            .args(&self.cli[1..])
            .args(args)
            .timeout(Duration::from_secs(15));
        command.output().context("execute actual strayd binary")
    }

    pub fn cli_services(&self, args: &[&str]) -> Result<Vec<ServiceProcess>> {
        let output = self.command(args)?;
        ensure!(
            output.status.success(),
            "strayd exited unsuccessfully: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value =
            serde_json::from_slice(&output.stdout).context("parse actual CLI JSON")?;
        let groups: Vec<port_deck_core::ResourceGroup> =
            serde_json::from_value(value["groups"].clone())?;
        Ok(groups
            .into_iter()
            .flat_map(|group| group.services)
            .collect())
    }

    pub fn scan(&self, child: &ManagedChild, observed: &mut Vec<Value>) -> Result<ServiceProcess> {
        let mut result = None;
        let scanned = until(Duration::from_secs(5), || {
            result = scan_all()
                .groups
                .into_iter()
                .flat_map(|group| group.services)
                .find(|service| service.pid == child.ready.pid);
            Ok(result.is_some())
        });
        if let Err(error) = scanned {
            let log = fs::read_to_string(&child.log).unwrap_or_default();
            observed.push(json!({"fixture_pid":child.ready.pid,"fixture_ports":child.ready.ports,"alive":alive(&child.identity)}));
            anyhow::bail!(
                "{error:#}; fixture stderr: {}",
                log.chars().take(4096).collect::<String>()
            );
        }
        let service = result.context("fixture missing from native scanner")?;
        // Evidence contains only our fixture, never unrelated host command lines.
        observed.push(serde_json::to_value(&service)?);
        Ok(service)
    }

    pub fn terminate(&self, service: &ServiceProcess, child: &ManagedChild) -> Result<()> {
        ensure!(
            service.pid == child.child.id()
                && service.start_token == child.identity.start.to_string(),
            "target does not belong to fixture"
        );
        // Ordinary PID tests must never stop a CI runner's inherited systemd unit.
        terminate(TerminateRequest {
            platform: service.platform,
            pid: service.pid,
            start_token: service.start_token.clone(),
            manager_unit: None,
        })?;
        Ok(())
    }
}

pub fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    }
}

pub fn native_binary(env: &Environment, report: &mut Report) {
    for (case, arguments) in [
        ("native-home-proxy", vec!["proxy", "--port", "9792"]),
        ("native-daemon-mode", vec!["--daemon-runner"]),
    ] {
        report.case(case, "native-fixture", |observed| {
            let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(PathBuf::from)
                .context("current user's home directory is unavailable")?;
            let mut child = env.fixture(case, "fixture-agent", Some(&home), &arguments)?;
            child.assert_listening()?;
            let service = env.scan(&child, observed)?;
            ensure!(
                service.display_name == executable_name("fixture-agent"),
                "home directory replaced executable name: {}",
                service.display_name
            );
            ensure!(
                service.ports == child.ready.ports,
                "ports do not match actual fixture bindings"
            );
            let started = service
                .started_at
                .context("creation time unavailable for own process")?;
            ensure!(
                started >= child.before.saturating_sub(2) && started <= child.after + 1,
                "creation time outside actual startup interval"
            );
            observed.push(json!({"started_between": [child.before, child.after]}));
            env.terminate(&service, &child)?;
            until(Duration::from_secs(3), || Ok(!alive(&child.identity)))?;
            child.cleanup()?;
            ensure!(
                TcpStream::connect(("127.0.0.1", service.ports[0])).is_err(),
                "fixture port remained open"
            );
            Ok(())
        });
    }
}

pub fn native(env: &Environment, report: &mut Report, full: bool) {
    crate::app_mock::run(env, report);
    report.case("unicode-spaced-executable", "native-fixture", |observed| {
        let mut child = env.fixture("unicode directory", "测试 service", None, &[])?;
        let service = env.scan(&child, observed)?;
        ensure!(
            service.display_name == executable_name("测试 service"),
            "executable path was split or corrupted"
        );
        child.cleanup()
    });
    report.case("multi-port-listener", "native-fixture", |observed| {
        let mut child = env.fixture(
            "multi-port",
            "multiple",
            None,
            &["--fixture-ports", "2", "--fixture-wildcard"],
        )?;
        child.assert_listening()?;
        let service = env.scan(&child, observed)?;
        let mut expected = child.ready.ports.clone();
        expected.sort_unstable();
        ensure!(
            service.ports == expected,
            "multiple ports were lost or duplicated"
        );
        child.cleanup()
    });
    report.case("ipv6-listener", "native-fixture", |observed| {
        let mut child = env.fixture("ipv6", "ipv6-server", None, &["--fixture-ipv6"])?;
        child.assert_listening()?;
        let service = env.scan(&child, observed)?;
        ensure!(
            service.hosts.contains(&"::1".to_owned()),
            "IPv6 host missing"
        );
        child.cleanup()
    });
    report.case(
        "non-listener-and-scanner-excluded",
        "native-fixture",
        |observed| {
            let mut child = env.fixture(
                "non-listener",
                "idle",
                None,
                &["--fixture-ports", "0", "--fixture-udp"],
            )?;
            let _own_socket = TcpListener::bind("127.0.0.1:0")?;
            let snapshot = scan_all();
            ensure!(
                !snapshot
                    .groups
                    .iter()
                    .flat_map(|group| &group.services)
                    .any(|service| service.pid == child.ready.pid
                        || service.pid == std::process::id()),
                "non-TCP listener or scanner itself leaked into discovery"
            );
            observed.push(json!({"fixture_pid": child.ready.pid, "excluded": true}));
            child.cleanup()
        },
    );
    if full {
        report.case(
            "same-port-multiple-loopback-addresses",
            "native-fixture",
            |observed| {
                let mut child =
                    env.fixture("same-port", "two-bindings", None, &["--fixture-same-port"])?;
                let port = child.ready.ports[0];
                ensure!(
                    TcpStream::connect(("127.0.0.1", port)).is_ok()
                        && TcpStream::connect(("127.0.0.2", port)).is_ok(),
                    "both actual bindings must be reachable"
                );
                let service = env.scan(&child, observed)?;
                ensure!(
                    service.ports == [port],
                    "a port must appear once per process"
                );
                child.cleanup()
            },
        );
        report.case("process-exits-during-scan", "native-fixture", |observed| {
            for index in 0..5 {
                let mut child =
                    env.fixture(&format!("scan-exit-{index}"), "racing-exit", None, &[])?;
                let scan = std::thread::spawn(scan_all);
                child.cleanup()?;
                let _snapshot = scan
                    .join()
                    .map_err(|_| anyhow::anyhow!("scanner panicked while a process exited"))?;
                ensure!(
                    !scan_all()
                        .groups
                        .iter()
                        .flat_map(|g| &g.services)
                        .any(|s| s.pid == child.child.id()),
                    "exited fixture persisted into the following scan"
                );
            }
            observed
                .push(json!({"concurrent_exit_rounds":5,"next_scan_has_no_stale_fixture":true}));
            Ok(())
        });
        lifecycle(env, report);
    }
}

pub fn lifecycle(env: &Environment, report: &mut Report) {
    report.case(
        "child-exits-before-parent-stop",
        "native-fixture",
        |observed| {
            let mut parent = env.fixture(
                "child-exit",
                "parent-survivor",
                None,
                &[
                    "--fixture-children",
                    "1",
                    "--fixture-child-lifetime-ms",
                    "250",
                ],
            )?;
            let child = identity(parent.ready.children[0])?;
            until(Duration::from_secs(3), || Ok(!alive(&child)))?;
            let service = env.scan(&parent, observed)?;
            env.terminate(&service, &parent)?;
            parent.cleanup()
        },
    );
    report.case(
        "identity-mismatch-refuses-stop",
        "native-fixture",
        |observed| {
            let mut child = env.fixture("identity", "identity-check", None, &[])?;
            let service = env.scan(&child, observed)?;
            let result = terminate(TerminateRequest {
                platform: host_platform(),
                pid: service.pid,
                start_token: format!("{}-stale", service.start_token),
                manager_unit: None,
            });
            ensure!(
                result == Err(EngineError::ProcessChanged),
                "stale identity was not rejected: {result:?}"
            );
            ensure!(alive(&child.identity), "stale identity killed the fixture");
            child.cleanup()
        },
    );
    report.case("protected-sshd-fixture", "native-fixture", |observed| {
        let mut child = env.fixture("protected", "sshd", None, &[])?;
        let service = env.scan(&child, observed)?;
        ensure!(!service.can_terminate, "sshd protection missing");
        let result = terminate(TerminateRequest {
            platform: host_platform(),
            pid: service.pid,
            start_token: service.start_token,
            manager_unit: None,
        });
        ensure!(
            result == Err(EngineError::ProtectedSshd) && alive(&child.identity),
            "protected fixture was not preserved"
        );
        child.cleanup()
    });
    report.case(
        "stop-tree-preserves-control",
        "native-fixture",
        |observed| {
            let mut control = env.fixture("tree-control", "control", None, &[])?;
            let mut parent = env.fixture("tree", "parent", None, &["--fixture-children", "2"])?;
            let children = parent
                .ready
                .children
                .iter()
                .map(|pid| identity(*pid))
                .collect::<Result<Vec<_>>>()?;
            let service = env.scan(&parent, observed)?;
            env.terminate(&service, &parent)?;
            until(Duration::from_secs(3), || {
                Ok(!alive(&parent.identity) && children.iter().all(|child| !alive(child)))
            })?;
            ensure!(
                alive(&control.identity),
                "unrelated control process was stopped"
            );
            observed.push(json!({"children":parent.ready.children,"control_alive":true}));
            parent.cleanup()?;
            control.cleanup()
        },
    );
    #[cfg(unix)]
    report.case("term-timeout-forces-kill", "native-fixture", |observed| {
        let mut child = env.fixture("ignore-term", "stubborn", None, &["--fixture-ignore-term"])?;
        let service = env.scan(&child, observed)?;
        env.terminate(&service, &child)?;
        until(Duration::from_secs(3), || Ok(!alive(&child.identity)))?;
        child.cleanup()
    });
    #[cfg(windows)]
    report.unavailable(
        "term-timeout-forces-kill",
        "native-fixture",
        false,
        "Windows uses taskkill /T /F rather than Unix TERM escalation",
    );
    report.case(
        "process-disappears-before-stop",
        "native-fixture",
        |observed| {
            let mut child = env.fixture("exit-before-stop", "short-lived", None, &[])?;
            let service = env.scan(&child, observed)?;
            child.cleanup()?;
            let result = terminate(TerminateRequest {
                platform: host_platform(),
                pid: service.pid,
                start_token: service.start_token,
                manager_unit: None,
            });
            ensure!(
                result == Err(EngineError::ProcessMissing),
                "missing process was not reported: {result:?}"
            );
            Ok(())
        },
    );
}

pub fn cli(env: &Environment, report: &mut Report) {
    report.case(
        "cli-list-filter-and-stop-contract",
        "native-fixture",
        |observed| {
            let mut child = env.fixture("cli", "cli-service", None, &[])?;
            let port = child.ready.ports[0].to_string();
            let services = env.cli_services(&["--no-config", "list", "--port", &port, "--json"])?;
            ensure!(
                services.len() == 1 && services[0].pid == child.child.id(),
                "CLI port filter returned wrong process"
            );
            let service = &services[0];
            observed.push(serde_json::to_value(service)?);
            ensure!(
                service.manager_unit.is_none(),
                "fixture inherited a systemd unit; refusing CLI stop outside dedicated unit test"
            );
            let dry = env.command(&[
                "--no-config",
                "stop",
                "resource",
                "--id",
                &service.id,
                "--dry-run",
            ])?;
            ensure!(
                dry.status.success() && alive(&child.identity),
                "dry-run changed the process"
            );
            let denied = env.command(&["--no-config", "stop", "resource", "--id", &service.id])?;
            ensure!(
                !denied.status.success() && alive(&child.identity),
                "non-interactive stop without --yes was accepted"
            );
            let stopped = env.command(&[
                "--no-config",
                "stop",
                "resource",
                "--id",
                &service.id,
                "--yes",
            ])?;
            ensure!(
                stopped.status.success(),
                "CLI stop failed: {}",
                String::from_utf8_lossy(&stopped.stderr)
            );
            child.cleanup()
        },
    );
    report.case("cli-isolated-config-and-failure", "native-fixture", |observed| {
        let path = env.root.join("isolated config.toml");
        let path = path.to_str().context("config path encoding")?;
        let init = env.command(&["--config", path, "config", "init"])?;
        ensure!(init.status.success(), "config init failed");
        let shown = env.command(&["--config", path, "config", "show"])?;
        ensure!(shown.status.success() && String::from_utf8_lossy(&shown.stdout).contains("version = 1"), "config show did not read initialized configuration");
        let location = env.command(&["--config", path, "config", "path"])?;
        ensure!(location.status.success() && String::from_utf8_lossy(&location.stdout).trim() == path, "explicit config path was not respected");
        ensure!(!env.command(&["--config", path, "config", "init"])?.status.success(), "config init overwrote existing file");
        let mut child = env.fixture("config", "config-fixture", None, &[])?;
        let port = child.ready.ports[0].to_string();
        fs::write(path, format!("version = 1\n[[display.hide]]\nports = [{port}]\n"))?;
        ensure!(env.cli_services(&["--config", path, "list", "--port", &port, "--json"])?.is_empty(), "hide rule ignored by actual CLI");
        ensure!(env.cli_services(&["--no-config", "list", "--port", &port, "--json"])?.len() == 1, "--no-config did not bypass hide rule");
        fs::write(path, "not valid = [")?;
        ensure!(!env.command(&["--config", path, "list"])?.status.success(), "invalid configuration silently accepted");
        let blocker = env.root.join("not-a-directory"); fs::write(&blocker, "preserve this file")?;
        let impossible = blocker.join("config.toml");
        ensure!(!env.command(&["--config", impossible.to_str().unwrap(), "config", "init"])?.status.success() && fs::read_to_string(blocker)? == "preserve this file", "unwritable path was accepted or overwritten");
        observed.push(json!({"isolated_config":true,"hidden_then_bypassed":true,"invalid_config_rejected":true}));
        child.cleanup()
    });
    report.case(
        "cli-non-tty-and-exit-status",
        "native-fixture",
        |observed| {
            ensure!(
                env.command(&["--version"])?.status.success(),
                "version failed"
            );
            ensure!(
                !env.command(&["--no-config", "tui"])?.status.success(),
                "TUI unexpectedly accepted piped IO"
            );
            ensure!(
                env.command(&["--definitely-not-an-option"])?.status.code() == Some(2),
                "unknown option did not preserve the CLI argument-error exit code"
            );
            observed.push(
                json!({"version":true,"non_tty_rejected":true,"invalid_option_rejected":true}),
            );
            Ok(())
        },
    );
}

pub fn faults(env: &Environment, report: &mut Report) {
    for (id, argument) in [
        ("startup-failure-cleanup", "--fixture-fail"),
        ("readiness-timeout-cleanup", "--fixture-no-ready"),
    ] {
        report.case(id, "native-fixture", |observed| {
            let mut command = Command::new(&env.fixture);
            command.arg(argument);
            let result = ManagedChild::start(
                &mut command,
                &env.registry,
                &env.root.join(format!("{id}.log")),
                Duration::from_millis(300),
            );
            ensure!(result.is_err(), "fault was not reported");
            observed.push(json!({"failure_reported": true}));
            env.registry.cleanup()
        });
    }
    report.case(
        "assertion-failure-cleans-fixture",
        "native-fixture",
        |observed| {
            let mut owned = None;
            let result: Result<()> = (|| {
                let child = env.fixture("assertion-cleanup", "failure-control", None, &[])?;
                owned = Some(child.identity.clone());
                anyhow::bail!("deliberate assertion failure");
            })();
            ensure!(
                result.is_err() && !alive(&owned.context("fixture identity")?),
                "failed scope leaked a process"
            );
            observed.push(json!({"failure_reported": true,"process_released":true}));
            Ok(())
        },
    );
}
