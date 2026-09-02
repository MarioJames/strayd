use std::{
    process::{Command, Output},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use port_deck_core::{ProcessOrigin, ServiceProcess, decode_command_output, parse_wsl_snapshot};
use serde::{Deserialize, Serialize};

#[cfg(target_os = "windows")]
use netstat2::{AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, TcpState, get_sockets_info};
#[cfg(target_os = "windows")]
use port_deck_core::{NativeListenerRecord, ensure_process_identity, group_native_listeners};
#[cfg(target_os = "windows")]
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

const WSL_SNAPSHOT_SCRIPT: &str = r#"
printf 'PORTDECK/1\036'
if ! command -v ss >/dev/null 2>&1; then
  printf 'E\037iproute2-not-installed\036'
  exit 0
fi
ss -H -ltnp 2>/dev/null | while IFS= read -r line; do
  clean=$(printf '%s' "$line" | tr '\036\037' '  ')
  printf 'S\037%s\036' "$clean"
done
for proc in /proc/[0-9]*; do
  pid=${proc##*/}
  stat=$(cat "$proc/stat" 2>/dev/null) || continue
  cwd=$(readlink "$proc/cwd" 2>/dev/null | tr '\036\037' '  ')
  command=$(tr '\000\036\037' '   ' < "$proc/cmdline" 2>/dev/null)
  [ -n "$command" ] || continue
  printf 'P\037%s\037%s\037%s\037%s\036' "$pid" "$stat" "$cwd" "$command"
done
"#;

const WSL_TERMINATE_SCRIPT: &str = r#"
pid=$1
expected=$2
case "$pid" in (*[!0-9]*|'') exit 46;; esac
[ "$pid" -gt 1 ] || exit 46
stat=$(cat "/proc/$pid/stat" 2>/dev/null) || exit 45
rest=${stat##*) }
set -- $rest
shift 19
current=$1
[ "$current" = "$expected" ] || exit 44
children_of() {
  wanted=$1
  for status in /proc/[0-9]*/status; do
    child=${status#/proc/}; child=${child%/status}
    parent=$(sed -n 's/^PPid:[[:space:]]*//p' "$status" 2>/dev/null)
    [ "$parent" = "$wanted" ] && printf '%s\n' "$child"
  done
}
collect_tree() {
  target=$1
  for child in $(children_of "$target"); do collect_tree "$child"; done
  printf '%s\n' "$target"
}
targets=$(collect_tree "$pid")
kill -TERM $targets 2>/dev/null || true
sleep 0.8
for target in $targets; do
  [ -d "/proc/$target" ] && kill -KILL "$target" 2>/dev/null || true
done
"#;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSnapshot {
    pub services: Vec<ServiceProcess>,
    pub warnings: Vec<String>,
    pub scanned_at: u128,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminateRequest {
    pub origin: ProcessOrigin,
    pub distribution: Option<String>,
    pub pid: u32,
    pub start_token: String,
}

pub fn scan_all() -> ScanSnapshot {
    let native_scan = thread::spawn(scan_native_services);
    let mut warnings = Vec::new();
    let mut services = Vec::new();

    match running_wsl_distributions() {
        Ok(distributions) => {
            let scans = distributions
                .into_iter()
                .filter(|distribution| !is_internal_distribution(distribution))
                .map(|distribution| {
                    thread::spawn(move || {
                        let result = scan_wsl_distribution(&distribution);
                        (distribution, result)
                    })
                })
                .collect::<Vec<_>>();

            for scan in scans {
                match scan.join() {
                    Ok((_, Ok(mut found))) => services.append(&mut found),
                    Ok((distribution, Err(error))) => {
                        warnings.push(format!("{distribution}: {error}"));
                    }
                    Err(_) => warnings.push("某个 WSL 扫描任务意外退出".into()),
                }
            }
        }
        Err(error) => warnings.push(format!("WSL: {error}")),
    }

    match native_scan.join() {
        Ok(Ok(mut found)) => services.append(&mut found),
        Ok(Err(error)) => warnings.push(format!("Windows: {error}")),
        Err(_) => warnings.push("Windows 扫描任务意外退出".into()),
    }

    services.sort_by_key(|service| {
        (
            !service.is_dev_server,
            service.ports.first().copied().unwrap_or_default(),
        )
    });

    ScanSnapshot {
        services,
        warnings,
        scanned_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    }
}

pub fn terminate(request: TerminateRequest) -> Result<(), String> {
    if request.pid <= 4 || request.pid == std::process::id() {
        return Err("拒绝结束受保护的系统进程".into());
    }
    if request.start_token.is_empty() {
        return Err("缺少进程启动标识，请重新扫描".into());
    }

    match request.origin {
        ProcessOrigin::Windows => terminate_native(&request),
        ProcessOrigin::Wsl => terminate_wsl(&request),
    }
}

pub fn open_local_service(port: u16) -> Result<(), String> {
    let url = format!("http://localhost:{port}");
    Command::new("explorer.exe")
        .arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开 {url}: {error}"))
}

fn running_wsl_distributions() -> Result<Vec<String>, String> {
    let output = quiet_command("wsl.exe")
        .args(["--list", "--running", "--quiet"])
        .output()
        .map_err(|error| format!("无法运行 wsl.exe: {error}"))?;
    ensure_success("读取运行中的发行版", &output)?;

    Ok(decode_command_output(&output.stdout)
        .lines()
        .map(|line| line.trim_matches(['\0', '\r', ' ', '\t']))
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

fn scan_wsl_distribution(distribution: &str) -> Result<Vec<ServiceProcess>, String> {
    let output = quiet_command("wsl.exe")
        .args([
            "--distribution",
            distribution,
            "--user",
            "root",
            "--exec",
            "sh",
            "-c",
            WSL_SNAPSHOT_SCRIPT,
        ])
        .output()
        .map_err(|error| format!("无法启动扫描: {error}"))?;
    ensure_success("扫描监听端口", &output)?;
    let snapshot = decode_command_output(&output.stdout);
    if snapshot.contains("E\x1fiproute2-not-installed") {
        return Err("缺少 ss 命令，请在该发行版安装 iproute2".into());
    }
    Ok(parse_wsl_snapshot(distribution, &snapshot))
}

fn terminate_wsl(request: &TerminateRequest) -> Result<(), String> {
    let distribution = request
        .distribution
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "缺少 WSL 发行版名称".to_string())?;
    let pid = request.pid.to_string();
    let output = quiet_command("wsl.exe")
        .args([
            "--distribution",
            distribution,
            "--user",
            "root",
            "--exec",
            "sh",
            "-c",
            WSL_TERMINATE_SCRIPT,
            "port-deck",
            &pid,
            &request.start_token,
        ])
        .output()
        .map_err(|error| format!("无法调用 WSL: {error}"))?;

    match output.status.code() {
        Some(0) => Ok(()),
        Some(44) => Err("进程已经变化，请重新扫描后再操作".into()),
        Some(45) => Err("进程已经结束".into()),
        Some(46) => Err("进程号无效".into()),
        _ => Err(command_error("结束 WSL 进程", &output)),
    }
}

#[cfg(target_os = "windows")]
fn scan_native_services() -> Result<Vec<ServiceProcess>, String> {
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
    let mut records = Vec::new();

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
            let Some(process) = system.process(sysinfo::Pid::from_u32(pid)) else {
                continue;
            };
            let command = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");
            records.push(NativeListenerRecord {
                port: tcp.local_port,
                host: tcp.local_addr.to_string(),
                pid,
                parent_pid: process
                    .parent()
                    .map(|value| value.as_u32())
                    .unwrap_or_default(),
                process_name: process.name().to_string_lossy().into_owned(),
                command,
                cwd: process
                    .cwd()
                    .map(|path| path.to_string_lossy().into_owned()),
                start_token: process.start_time().to_string(),
            });
        }
    }

    Ok(group_native_listeners(records))
}

#[cfg(not(target_os = "windows"))]
fn scan_native_services() -> Result<Vec<ServiceProcess>, String> {
    Ok(Vec::new())
}

#[cfg(target_os = "windows")]
fn terminate_native(request: &TerminateRequest) -> Result<(), String> {
    let mut system = System::new();
    let pid = sysinfo::Pid::from_u32(request.pid);
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    let actual = system
        .process(pid)
        .map(|process| process.start_time().to_string());
    ensure_process_identity(&request.start_token, actual.as_deref())
        .map_err(|error| format!("{error}，请重新扫描"))?;

    let output = quiet_command("taskkill.exe")
        .args(["/PID", &request.pid.to_string(), "/T", "/F"])
        .output()
        .map_err(|error| format!("无法运行 taskkill.exe: {error}"))?;
    ensure_success("结束 Windows 进程", &output)
}

#[cfg(not(target_os = "windows"))]
fn terminate_native(_request: &TerminateRequest) -> Result<(), String> {
    Err("只能在 Windows 版本中结束 Windows 进程".into())
}

fn is_internal_distribution(distribution: &str) -> bool {
    matches!(
        distribution.to_ascii_lowercase().as_str(),
        "docker-desktop" | "docker-desktop-data"
    )
}

fn ensure_success(action: &str, output: &Output) -> Result<(), String> {
    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(action, output))
    }
}

fn command_error(action: &str, output: &Output) -> String {
    let stderr = decode_command_output(&output.stderr);
    let detail = stderr.trim_matches(['\0', '\r', '\n', ' ']);
    if detail.is_empty() {
        format!("{action}失败，退出码 {:?}", output.status.code())
    } else {
        format!("{action}失败: {detail}")
    }
}

fn quiet_command(program: &str) -> Command {
    #[cfg(not(target_os = "windows"))]
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
