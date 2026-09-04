use std::{
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(any(target_os = "windows", target_os = "linux"))]
use std::process::Output;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::{collections::BTreeSet, thread, time::Duration};

use netstat2::{AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, TcpState, get_sockets_info};
#[cfg(any(target_os = "windows", target_os = "linux"))]
use port_deck_core::decode_command_output;
use port_deck_core::{
    HostPlatform, NativeListenerRecord, NativeProcessRecord, ResourceGroup, ResourceKind,
    RuntimeKind, ServiceProcess, classify_runtime, ensure_process_identity, group_native_resources,
    group_related_services, is_safe_service_unit,
};
use serde::{Deserialize, Serialize};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use sysinfo::Signal;
use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
compile_error!("Strayd currently supports Windows, Linux, and macOS only");

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSnapshot {
    pub groups: Vec<ResourceGroup>,
    pub warnings: Vec<String>,
    pub scanned_at: u128,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminateRequest {
    pub platform: HostPlatform,
    pub pid: u32,
    pub start_token: String,
    pub manager_unit: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineAction {
    StopWindowsProcess,
    StopSystemdService,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    #[error("target resource is not on the current host platform")]
    WrongPlatform,
    #[error("refusing to terminate a protected system process")]
    ProtectedProcess,
    #[error("process start identity is missing")]
    MissingStartToken,
    #[error("invalid systemd service unit")]
    InvalidServiceUnit,
    #[error("could not open {url}: {detail}")]
    OpenUrl { url: String, detail: String },
    #[error("could not start {program}: {detail}")]
    Spawn { program: String, detail: String },
    #[error("could not send TERM to the target process")]
    SignalDenied,
    #[error("process no longer exists")]
    ProcessMissing,
    #[error("process identity changed")]
    ProcessChanged,
    #[error("sshd is a protected remote entry point")]
    ProtectedSshd,
    #[error("systemd ownership changed")]
    ManagedRelationChanged,
    #[error("command failed")]
    CommandFailed {
        action: EngineAction,
        exit_code: Option<i32>,
        detail: Option<String>,
    },
}

pub const fn host_platform() -> HostPlatform {
    #[cfg(target_os = "windows")]
    return HostPlatform::Windows;

    #[cfg(target_os = "linux")]
    return HostPlatform::Linux;

    #[cfg(target_os = "macos")]
    return HostPlatform::MacOs;
}

pub fn scan_all() -> ScanSnapshot {
    let (services, warnings) = match scan_host_services() {
        Ok(services) => (services, Vec::new()),
        Err(error) => (
            Vec::new(),
            vec![format!("{}: {error}", platform_label(host_platform()))],
        ),
    };

    let mut services = services;
    services.sort_by_key(|service| {
        let kind = match service.resource_kind {
            ResourceKind::Development => 0,
            ResourceKind::Tunnel => 1,
            ResourceKind::System => 2,
            ResourceKind::Other => 3,
        };
        (kind, service.ports.first().copied().unwrap_or_default())
    });

    ScanSnapshot {
        groups: group_related_services(services),
        warnings,
        scanned_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    }
}

pub fn terminate(request: TerminateRequest) -> Result<(), EngineError> {
    if request.platform != host_platform() {
        return Err(EngineError::WrongPlatform);
    }
    if request.pid <= 4 || request.pid == std::process::id() {
        return Err(EngineError::ProtectedProcess);
    }
    if request.start_token.is_empty() {
        return Err(EngineError::MissingStartToken);
    }
    if request
        .manager_unit
        .as_deref()
        .is_some_and(|unit| !is_safe_service_unit(unit))
    {
        return Err(EngineError::InvalidServiceUnit);
    }

    terminate_host_process(&request)
}

impl From<&ServiceProcess> for TerminateRequest {
    fn from(service: &ServiceProcess) -> Self {
        Self {
            platform: service.platform,
            pid: service.pid,
            start_token: service.start_token.clone(),
            manager_unit: service.manager_unit.clone(),
        }
    }
}

pub fn terminate_service(service: &ServiceProcess) -> Result<(), EngineError> {
    terminate(TerminateRequest::from(service))
}

pub fn open_local_service(port: u16) -> Result<(), EngineError> {
    let url = format!("http://localhost:{port}");
    let mut command = open_command(&url);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| EngineError::OpenUrl {
            url,
            detail: error.to_string(),
        })
}

fn scan_host_services() -> Result<Vec<ServiceProcess>, String> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_cwd(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );

    let sockets = get_sockets_info(
        AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6,
        ProtocolFlags::TCP,
    )
    .map_err(|error| error.to_string())?;
    let own_pid = std::process::id();
    let mut listener_records = Vec::new();

    for socket in sockets {
        let ProtocolSocketInfo::Tcp(tcp) = socket.protocol_socket_info else {
            continue;
        };
        if tcp.state != TcpState::Listen {
            continue;
        }
        for pid in socket.associated_pids {
            if pid <= 4 || pid == own_pid {
                continue;
            }
            let Some(process) = system.process(Pid::from_u32(pid)) else {
                continue;
            };
            let metadata = process_metadata(pid, process);
            listener_records.push(NativeListenerRecord {
                port: tcp.local_port,
                host: tcp.local_addr.to_string(),
                pid: metadata.pid,
                parent_pid: metadata.parent_pid,
                process_name: metadata.process_name,
                command: metadata.command,
                cwd: metadata.cwd,
                start_token: metadata.start_token,
            });
        }
    }

    let process_records = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let pid = pid.as_u32();
            (pid > 4 && pid != own_pid).then(|| process_metadata(pid, process))
        })
        .collect();

    let services = group_native_resources(host_platform(), listener_records, process_records);
    #[cfg(target_os = "linux")]
    let services = services
        .into_iter()
        .map(|mut service| {
            service.manager_unit = linux_manager_unit(service.pid);
            service
        })
        .collect();
    Ok(services)
}

fn process_metadata(pid: u32, process: &Process) -> NativeProcessRecord {
    let process_name = process.name().to_string_lossy().into_owned();
    let command = process
        .cmd()
        .iter()
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    NativeProcessRecord {
        pid,
        parent_pid: process
            .parent()
            .map(|value| value.as_u32())
            .unwrap_or_default(),
        process_name: process_name.clone(),
        command: if command.is_empty() {
            process_name
        } else {
            command
        },
        cwd: process
            .cwd()
            .map(|path| path.to_string_lossy().into_owned()),
        start_token: process.start_time().to_string(),
    }
}

#[cfg(target_os = "linux")]
fn linux_manager_unit(pid: u32) -> Option<String> {
    let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    parse_linux_manager_unit(&cgroup)
}

#[cfg(target_os = "linux")]
fn parse_linux_manager_unit(cgroup: &str) -> Option<String> {
    cgroup.lines().find_map(|line| {
        let path = line.splitn(3, ':').nth(2)?;
        path.split('/')
            .rev()
            .find(|segment| !segment.is_empty())
            .filter(|segment| is_safe_service_unit(segment))
            .map(str::to_owned)
    })
}

#[cfg(target_os = "windows")]
fn terminate_host_process(request: &TerminateRequest) -> Result<(), EngineError> {
    let system = refreshed_process(request.pid);
    ensure_terminable_identity(&system, request)?;

    let output = quiet_command("taskkill.exe")
        .args(["/PID", &request.pid.to_string(), "/T", "/F"])
        .output()
        .map_err(|error| EngineError::Spawn {
            program: "taskkill.exe".into(),
            detail: error.to_string(),
        })?;
    ensure_success(EngineAction::StopWindowsProcess, &output)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn terminate_host_process(request: &TerminateRequest) -> Result<(), EngineError> {
    let mut system = refreshed_processes();
    ensure_terminable_identity(&system, request)?;

    #[cfg(target_os = "linux")]
    if let Some(unit) = request.manager_unit.as_deref() {
        return stop_linux_service(request.pid, unit);
    }

    let targets = process_tree(&system, Pid::from_u32(request.pid));
    let mut signaled = false;
    for (pid, _) in targets.iter().rev() {
        if let Some(process) = system.process(*pid) {
            signaled |= process.kill_with(Signal::Term) == Some(true);
        }
    }
    if !signaled {
        return Err(EngineError::SignalDenied);
    }

    thread::sleep(Duration::from_millis(800));
    system.refresh_processes(ProcessesToUpdate::All, true);
    for (pid, start_token) in targets.iter().rev() {
        if let Some(process) = system.process(*pid)
            && process.start_time().to_string() == *start_token
        {
            process.kill();
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn refreshed_process(pid: u32) -> System {
    let mut system = System::new();
    let pid = Pid::from_u32(pid);
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    system
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn refreshed_processes() -> System {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    system
}

fn ensure_terminable_identity(
    system: &System,
    request: &TerminateRequest,
) -> Result<(), EngineError> {
    let pid = Pid::from_u32(request.pid);
    let actual = system
        .process(pid)
        .map(|process| process.start_time().to_string());
    ensure_process_identity(&request.start_token, actual.as_deref()).map_err(
        |error| match error {
            port_deck_core::IdentityError::Missing => EngineError::ProcessMissing,
            port_deck_core::IdentityError::Changed => EngineError::ProcessChanged,
        },
    )?;

    let process = system.process(pid).ok_or(EngineError::ProcessMissing)?;
    let command = process
        .cmd()
        .iter()
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    if classify_runtime(&process.name().to_string_lossy(), &command) == RuntimeKind::Sshd {
        return Err(EngineError::ProtectedSshd);
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn process_tree(system: &System, root: Pid) -> Vec<(Pid, String)> {
    let mut pending = vec![root];
    let mut visited = BTreeSet::new();
    let mut result = Vec::new();
    while let Some(parent) = pending.pop() {
        if !visited.insert(parent) {
            continue;
        }
        if let Some(process) = system.process(parent) {
            result.push((parent, process.start_time().to_string()));
        }
        pending.extend(
            system
                .processes()
                .iter()
                .filter_map(|(pid, process)| (process.parent() == Some(parent)).then_some(*pid)),
        );
    }
    result
}

#[cfg(target_os = "linux")]
fn stop_linux_service(pid: u32, unit: &str) -> Result<(), EngineError> {
    let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .map_err(|_| EngineError::ManagedRelationChanged)?;
    let belongs_to_unit = cgroup.lines().any(|line| {
        line.splitn(3, ':')
            .nth(2)
            .is_some_and(|path| path.split('/').any(|segment| segment == unit))
    });
    if !belongs_to_unit {
        return Err(EngineError::ManagedRelationChanged);
    }

    let is_user_unit = cgroup.lines().any(|line| {
        line.splitn(3, ':')
            .nth(2)
            .is_some_and(|path| path.contains("/user.slice/"))
    });
    let mut command = quiet_command("systemctl");
    if is_user_unit {
        command.arg("--user");
    }
    let output = command
        .args(["stop", "--", unit])
        .output()
        .map_err(|error| EngineError::Spawn {
            program: "systemctl".into(),
            detail: error.to_string(),
        })?;
    ensure_success(EngineAction::StopSystemdService, &output)
}

#[cfg(target_os = "windows")]
fn open_command(url: &str) -> Command {
    let mut command = quiet_command("cmd.exe");
    command.args(["/C", "start", "", url]);
    command
}

#[cfg(target_os = "macos")]
fn open_command(url: &str) -> Command {
    let mut command = Command::new("open");
    command.arg(url);
    command
}

#[cfg(target_os = "linux")]
fn open_command(url: &str) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(url);
    command
}

fn platform_label(platform: HostPlatform) -> &'static str {
    match platform {
        HostPlatform::Windows => "Windows",
        HostPlatform::Linux => "Linux",
        HostPlatform::MacOs => "macOS",
    }
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn ensure_success(action: EngineAction, output: &Output) -> Result<(), EngineError> {
    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(action, output))
    }
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn command_error(action: EngineAction, output: &Output) -> EngineError {
    let stderr = decode_command_output(&output.stderr);
    let detail = stderr.trim_matches(['\0', '\r', '\n', ' ']);
    EngineError::CommandFailed {
        action,
        exit_code: output.status.code(),
        detail: (!detail.is_empty()).then(|| detail.to_owned()),
    }
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn quiet_command(program: &str) -> Command {
    #[cfg(target_os = "linux")]
    return Command::new(program);

    #[cfg(target_os = "windows")]
    let mut command = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    #[cfg(target_os = "windows")]
    command
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::parse_linux_manager_unit;

    #[test]
    fn only_treats_the_leaf_cgroup_as_the_managed_unit() {
        let app_scope =
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.browser.scope\n";
        let tunnel_service =
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/demo-tunnel.service\n";

        assert_eq!(parse_linux_manager_unit(app_scope), None);
        assert_eq!(
            parse_linux_manager_unit(tunnel_service).as_deref(),
            Some("demo-tunnel.service")
        );
    }
}
