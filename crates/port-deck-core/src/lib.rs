use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

mod naming;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeKind {
    NextJs,
    Vite,
    Nuxt,
    Astro,
    SvelteKit,
    Remix,
    Angular,
    Storybook,
    Webpack,
    Parcel,
    Rspack,
    Node,
    Bun,
    Deno,
    Cloudflared,
    Ngrok,
    SshTunnel,
    Frp,
    LocalTunnel,
    Bore,
    Sshd,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceKind {
    Development,
    Tunnel,
    System,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HostPlatform {
    Windows,
    Linux,
    MacOs,
}

impl HostPlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::MacOs => "macos",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceProcess {
    pub id: String,
    pub platform: HostPlatform,
    pub pid: u32,
    pub parent_pid: u32,
    pub ports: Vec<u16>,
    pub hosts: Vec<String>,
    pub process_name: String,
    pub display_name: String,
    pub command: String,
    pub cwd: Option<String>,
    pub project_name: Option<String>,
    pub runtime: RuntimeKind,
    pub resource_kind: ResourceKind,
    pub can_terminate: bool,
    pub manager_unit: Option<String>,
    pub tunnel_target: Option<TunnelTarget>,
    /// Process creation time in Unix seconds; None when unavailable.
    pub started_at: Option<u64>,
    pub start_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelTarget {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceGroup {
    pub id: String,
    pub primary_port: Option<u16>,
    pub services: Vec<ServiceProcess>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeListenerRecord {
    pub port: u16,
    pub host: String,
    pub pid: u32,
    pub parent_pid: u32,
    pub process_name: String,
    pub executable: Option<String>,
    pub arguments: Vec<String>,
    pub command: String,
    pub cwd: Option<String>,
    /// Process creation time in Unix seconds; None when unavailable.
    pub started_at: Option<u64>,
    pub start_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeProcessRecord {
    pub pid: u32,
    pub parent_pid: u32,
    pub process_name: String,
    pub executable: Option<String>,
    pub arguments: Vec<String>,
    pub command: String,
    pub cwd: Option<String>,
    /// Process creation time in Unix seconds; None when unavailable.
    pub started_at: Option<u64>,
    pub start_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    #[error("process no longer exists")]
    Missing,
    #[error("process identity changed")]
    Changed,
}

pub fn decode_command_output(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    let has_le_bom = bytes.starts_with(&[0xff, 0xfe]);
    let has_be_bom = bytes.starts_with(&[0xfe, 0xff]);
    let looks_little_endian = bytes.len().is_multiple_of(2)
        && bytes
            .iter()
            .skip(1)
            .step_by(2)
            .filter(|byte| **byte == 0)
            .count()
            > bytes.len() / 8;
    let looks_big_endian = bytes.len().is_multiple_of(2)
        && bytes.iter().step_by(2).filter(|byte| **byte == 0).count() > bytes.len() / 8;

    if has_le_bom || has_be_bom || looks_little_endian || looks_big_endian {
        let little_endian = has_le_bom || (!has_be_bom && looks_little_endian);
        let offset = usize::from(has_le_bom || has_be_bom) * 2;
        let (pairs, _) = bytes[offset..].as_chunks::<2>();
        let units = pairs.iter().map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        });
        return String::from_utf16_lossy(&units.collect::<Vec<_>>());
    }

    String::from_utf8_lossy(bytes).into_owned()
}

pub fn classify_runtime(process_name: &str, command: &str) -> RuntimeKind {
    let value = format!("{process_name} {command}").to_ascii_lowercase();
    let executable = process_name
        .trim_matches(['"', '\''])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(process_name)
        .to_ascii_lowercase();
    let command_executable = command
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(['"', '\''])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    if matches!(executable.as_str(), "cloudflared" | "cloudflared.exe") {
        RuntimeKind::Cloudflared
    } else if matches!(executable.as_str(), "ngrok" | "ngrok.exe") {
        RuntimeKind::Ngrok
    } else if matches!(executable.as_str(), "frpc" | "frpc.exe") {
        RuntimeKind::Frp
    } else if matches!(executable.as_str(), "bore" | "bore.exe") && value.contains("bore local") {
        RuntimeKind::Bore
    } else if (matches!(executable.as_str(), "lt" | "lt.cmd" | "lt.exe")
        || contains_any(
            &value,
            &[
                "localtunnel",
                "node_modules/.bin/lt",
                "node_modules\\.bin\\lt",
            ],
        ))
        && contains_any(&value, &[" --port", " -p "])
    {
        RuntimeKind::LocalTunnel
    } else if matches!(
        executable.as_str(),
        "ssh" | "ssh.exe" | "autossh" | "autossh.exe"
    ) && has_reverse_forward(&value)
    {
        RuntimeKind::SshTunnel
    } else if matches!(executable.as_str(), "sshd" | "sshd.exe") {
        RuntimeKind::Sshd
    } else if contains_any(
        &value,
        &[
            "next-server",
            "next dev",
            "next start",
            "/next/dist/bin/next",
            "\\next\\dist\\bin\\next",
        ],
    ) {
        RuntimeKind::NextJs
    } else if contains_any(
        &value,
        &["storybook dev", "start-storybook", "storybook-server"],
    ) {
        RuntimeKind::Storybook
    } else if contains_any(
        &value,
        &[
            "nuxi",
            "nuxt dev",
            "nuxt start",
            ".output/server/index.mjs nuxt",
        ],
    ) {
        RuntimeKind::Nuxt
    } else if contains_any(
        &value,
        &[
            "astro dev",
            "astro preview",
            "/astro/astro.js",
            "\\astro\\astro.js",
        ],
    ) {
        RuntimeKind::Astro
    } else if contains_any(&value, &["svelte-kit", "sveltekit", "@sveltejs/kit"]) {
        RuntimeKind::SvelteKit
    } else if contains_any(&value, &["remix dev", "remix-serve", "@remix-run/dev"]) {
        RuntimeKind::Remix
    } else if contains_any(&value, &["ng serve", "@angular/cli", "angular dev-server"]) {
        RuntimeKind::Angular
    } else if contains_any(&value, &["webpack-dev-server", "webpack serve"]) {
        RuntimeKind::Webpack
    } else if contains_any(&value, &["rspack serve", "rspack dev", "rspack-cli"]) {
        RuntimeKind::Rspack
    } else if contains_any(
        &value,
        &[
            "parcel serve",
            "parcel watch",
            "/parcel/lib/bin.js",
            "\\parcel\\lib\\bin.js",
        ],
    ) {
        RuntimeKind::Parcel
    } else if contains_any(
        &value,
        &[
            "vite.js",
            "vite dev",
            "vite preview",
            "/.bin/vite",
            "\\.bin\\vite",
            "/vite/bin/vite",
            "\\vite\\bin\\vite",
        ],
    ) {
        RuntimeKind::Vite
    } else if matches!(executable.as_str(), "bun" | "bun.exe")
        || matches!(command_executable.as_str(), "bun" | "bun.exe")
        || value.contains(" bun run ")
    {
        RuntimeKind::Bun
    } else if matches!(executable.as_str(), "deno" | "deno.exe")
        || matches!(command_executable.as_str(), "deno" | "deno.exe")
    {
        RuntimeKind::Deno
    } else if matches!(executable.as_str(), "node" | "node.exe")
        || matches!(command_executable.as_str(), "node" | "node.exe")
    {
        RuntimeKind::Node
    } else {
        RuntimeKind::Other
    }
}

pub fn resource_kind_for(runtime: &RuntimeKind) -> ResourceKind {
    match runtime {
        RuntimeKind::Cloudflared
        | RuntimeKind::Ngrok
        | RuntimeKind::SshTunnel
        | RuntimeKind::Frp
        | RuntimeKind::LocalTunnel
        | RuntimeKind::Bore => ResourceKind::Tunnel,
        RuntimeKind::Sshd => ResourceKind::System,
        RuntimeKind::Other => ResourceKind::Other,
        _ => ResourceKind::Development,
    }
}

pub fn can_terminate_runtime(runtime: &RuntimeKind) -> bool {
    !matches!(runtime, RuntimeKind::Sshd)
}

pub fn extract_tunnel_target(runtime: &RuntimeKind, command: &str) -> Option<TunnelTarget> {
    let arguments = command_arguments(command);
    match runtime {
        RuntimeKind::Cloudflared => flag_value(&arguments, "--url").and_then(parse_origin_target),
        RuntimeKind::Ngrok => subcommand_target(&arguments, &["http", "http2", "tcp", "tls"]),
        RuntimeKind::SshTunnel => ssh_reverse_target(&arguments),
        RuntimeKind::LocalTunnel => flag_value(&arguments, "--port")
            .or_else(|| flag_value(&arguments, "-p"))
            .and_then(parse_local_port),
        RuntimeKind::Bore => subcommand_target(&arguments, &["local"]),
        _ => None,
    }
}

pub fn group_native_listeners(
    platform: HostPlatform,
    records: Vec<NativeListenerRecord>,
) -> Vec<ServiceProcess> {
    group_native_resources(platform, records, Vec::new())
}

pub fn group_native_resources(
    platform: HostPlatform,
    records: Vec<NativeListenerRecord>,
    processes: Vec<NativeProcessRecord>,
) -> Vec<ServiceProcess> {
    let mut grouped = BTreeMap::<(u32, String), ServiceProcess>::new();

    for record in records {
        let key = (record.pid, record.start_token.clone());
        let runtime = classify_runtime(&record.process_name, &record.command);
        let resource_kind = resource_kind_for(&runtime);
        let can_terminate = can_terminate_runtime(&runtime);
        let tunnel_target = extract_tunnel_target(&runtime, &record.command);
        let project_name = record.cwd.as_deref().and_then(naming::project_name);
        let display_name = naming::display_name(
            record.pid,
            &record.process_name,
            record.executable.as_deref(),
            &record.arguments,
            &runtime,
            project_name.as_deref(),
        );
        let service = grouped.entry(key).or_insert_with(|| ServiceProcess {
            id: format!(
                "{}:{}:{}",
                platform.as_str(),
                record.pid,
                record.start_token
            ),
            platform,
            pid: record.pid,
            parent_pid: record.parent_pid,
            ports: Vec::new(),
            hosts: Vec::new(),
            display_name,
            process_name: record.process_name.clone(),
            command: record.command.clone(),
            cwd: record.cwd.clone(),
            project_name,
            runtime,
            resource_kind,
            can_terminate,
            manager_unit: None,
            tunnel_target,
            started_at: record.started_at,
            start_token: record.start_token.clone(),
        });
        push_listener(service, record.port, record.host);
    }

    for process in processes {
        let key = (process.pid, process.start_token.clone());
        if grouped.contains_key(&key) {
            continue;
        }
        let runtime = classify_runtime(&process.process_name, &process.command);
        let resource_kind = resource_kind_for(&runtime);
        if !matches!(resource_kind, ResourceKind::Tunnel | ResourceKind::System) {
            continue;
        }
        let can_terminate = can_terminate_runtime(&runtime);
        let tunnel_target = extract_tunnel_target(&runtime, &process.command);
        let project_name = process.cwd.as_deref().and_then(naming::project_name);
        let display_name = naming::display_name(
            process.pid,
            &process.process_name,
            process.executable.as_deref(),
            &process.arguments,
            &runtime,
            project_name.as_deref(),
        );
        grouped.insert(
            key,
            ServiceProcess {
                id: format!(
                    "{}:{}:{}",
                    platform.as_str(),
                    process.pid,
                    process.start_token
                ),
                platform,
                pid: process.pid,
                parent_pid: process.parent_pid,
                ports: Vec::new(),
                hosts: Vec::new(),
                display_name,
                process_name: process.process_name,
                command: process.command,
                cwd: process.cwd.clone(),
                project_name,
                runtime,
                resource_kind,
                can_terminate,
                manager_unit: None,
                tunnel_target,
                started_at: process.started_at,
                start_token: process.start_token,
            },
        );
    }

    finish_grouping(grouped)
}

pub fn group_related_services(services: Vec<ServiceProcess>) -> Vec<ResourceGroup> {
    let mut listeners = BTreeMap::<(HostPlatform, u16), Vec<usize>>::new();
    for (index, service) in services.iter().enumerate() {
        if service.resource_kind == ResourceKind::Tunnel {
            continue;
        }
        for port in &service.ports {
            listeners
                .entry(execution_scope(service, *port))
                .or_default()
                .push(index);
        }
    }

    let mut tunnel_links = BTreeMap::<usize, Vec<usize>>::new();
    let mut linked_tunnels = vec![false; services.len()];
    for (tunnel_index, tunnel) in services.iter().enumerate() {
        if tunnel.resource_kind != ResourceKind::Tunnel {
            continue;
        }
        let Some(target) = tunnel
            .tunnel_target
            .as_ref()
            .filter(|target| is_loopback_host(&target.host))
        else {
            continue;
        };
        let Some(candidates) = listeners.get(&execution_scope(tunnel, target.port)) else {
            continue;
        };
        if let [service_index] = candidates.as_slice() {
            tunnel_links
                .entry(*service_index)
                .or_default()
                .push(tunnel_index);
            linked_tunnels[tunnel_index] = true;
        }
    }

    let mut consumed = vec![false; services.len()];
    let mut groups = Vec::new();
    for (index, service) in services.iter().enumerate() {
        if consumed[index] || linked_tunnels[index] {
            continue;
        }
        consumed[index] = true;
        let linked_tunnels = tunnel_links.get(&index).cloned().unwrap_or_default();
        let primary_port = linked_tunnels
            .first()
            .and_then(|tunnel_index| services[*tunnel_index].tunnel_target.as_ref())
            .map(|target| target.port)
            .or_else(|| service.ports.first().copied())
            .or_else(|| service.tunnel_target.as_ref().map(|target| target.port));
        let mut members = vec![service.clone()];
        for tunnel_index in linked_tunnels {
            consumed[tunnel_index] = true;
            members.push(services[tunnel_index].clone());
        }
        groups.push(ResourceGroup {
            id: format!("group:{}", service.id),
            primary_port,
            services: members,
        });
    }
    groups
}

pub fn ensure_process_identity(
    expected_start_token: &str,
    actual_start_token: Option<&str>,
) -> Result<(), IdentityError> {
    match actual_start_token {
        None => Err(IdentityError::Missing),
        Some(actual) if actual != expected_start_token => Err(IdentityError::Changed),
        Some(_) => Ok(()),
    }
}

pub fn is_safe_service_unit(unit: &str) -> bool {
    !unit.is_empty()
        && unit.len() <= 255
        && unit.ends_with(".service")
        && unit
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && unit.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '_' | '-' | '.' | '@' | ':' | '\\')
        })
}

fn contains_any(value: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| value.contains(pattern))
}

fn has_reverse_forward(command: &str) -> bool {
    command.split_whitespace().any(|argument| {
        argument == "-r"
            || (argument.starts_with("-r") && argument.len() > 2)
            || argument.starts_with("remoteforward=")
            || argument.starts_with("-oremoteforward=")
    })
}

fn command_arguments(command: &str) -> Vec<&str> {
    command
        .split_whitespace()
        .map(|argument| argument.trim_matches(['"', '\'']))
        .collect()
}

fn flag_value<'a>(arguments: &[&'a str], flag: &str) -> Option<&'a str> {
    arguments.iter().enumerate().find_map(|(index, argument)| {
        if *argument == flag {
            arguments.get(index + 1).copied()
        } else {
            argument
                .strip_prefix(flag)
                .and_then(|value| value.strip_prefix('='))
        }
    })
}

fn subcommand_target(arguments: &[&str], subcommands: &[&str]) -> Option<TunnelTarget> {
    let index = arguments.iter().position(|argument| {
        subcommands
            .iter()
            .any(|value| argument.eq_ignore_ascii_case(value))
    })?;
    arguments[index + 1..]
        .iter()
        .filter(|argument| !argument.starts_with('-'))
        .find_map(|argument| parse_origin_target(argument))
}

fn parse_origin_target(value: &str) -> Option<TunnelTarget> {
    if let Some(target) = parse_local_port(value) {
        return Some(target);
    }
    let normalized;
    let value = if value.contains("://") {
        value
    } else {
        normalized = format!("http://{value}");
        &normalized
    };
    let url = url::Url::parse(value).ok()?;
    Some(TunnelTarget {
        host: url
            .host_str()?
            .trim_matches(['[', ']'])
            .to_ascii_lowercase(),
        port: url.port_or_known_default()?,
    })
}

fn parse_local_port(value: &str) -> Option<TunnelTarget> {
    value.parse::<u16>().ok().map(|port| TunnelTarget {
        host: "localhost".into(),
        port,
    })
}

fn ssh_reverse_target(arguments: &[&str]) -> Option<TunnelTarget> {
    let spec = arguments.iter().enumerate().find_map(|(index, argument)| {
        let lower = argument.to_ascii_lowercase();
        if lower == "-r" {
            arguments.get(index + 1).copied()
        } else if lower.starts_with("-r") && argument.len() > 2 {
            Some(&argument[2..])
        } else if let Some(value) = lower.strip_prefix("-oremoteforward=") {
            let offset = argument.len() - value.len();
            Some(&argument[offset..])
        } else if let Some(value) = lower.strip_prefix("remoteforward=") {
            let offset = argument.len() - value.len();
            Some(&argument[offset..])
        } else {
            None
        }
    })?;
    let (host_part, port) = spec.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?;
    let host = if host_part.ends_with(']') {
        let start = host_part.rfind('[')?;
        &host_part[start + 1..host_part.len() - 1]
    } else {
        host_part.rsplit_once(':')?.1
    };
    Some(TunnelTarget {
        host: host.to_ascii_lowercase(),
        port,
    })
}

fn execution_scope(service: &ServiceProcess, port: u16) -> (HostPlatform, u16) {
    (service.platform, port)
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']);
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn push_listener(service: &mut ServiceProcess, port: u16, host: String) {
    if service.ports.contains(&port) {
        return;
    }
    service.ports.push(port);
    service.hosts.push(host);
}

fn finish_grouping<K: Ord>(grouped: BTreeMap<K, ServiceProcess>) -> Vec<ServiceProcess> {
    let mut services = grouped.into_values().collect::<Vec<_>>();
    for service in &mut services {
        let mut listeners = service
            .ports
            .drain(..)
            .zip(service.hosts.drain(..))
            .collect::<Vec<_>>();
        listeners.sort_unstable_by_key(|(port, _)| *port);
        (service.ports, service.hosts) = listeners.into_iter().unzip();
    }
    services.sort_by_key(|service| {
        let kind = match service.resource_kind {
            ResourceKind::Development => 0,
            ResourceKind::Tunnel => 1,
            ResourceKind::System => 2,
            ResourceKind::Other => 3,
        };
        (kind, service.ports.first().copied().unwrap_or_default())
    });
    services
}
