use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessOrigin {
    Windows,
    Wsl,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceProcess {
    pub id: String,
    pub origin: ProcessOrigin,
    pub distribution: Option<String>,
    pub pid: u32,
    pub parent_pid: u32,
    pub ports: Vec<u16>,
    pub hosts: Vec<String>,
    pub process_name: String,
    pub command: String,
    pub cwd: Option<String>,
    pub project_name: Option<String>,
    pub runtime: RuntimeKind,
    pub is_dev_server: bool,
    pub start_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeListenerRecord {
    pub port: u16,
    pub host: String,
    pub pid: u32,
    pub parent_pid: u32,
    pub process_name: String,
    pub command: String,
    pub cwd: Option<String>,
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

    if contains_any(
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
    } else if process_name.eq_ignore_ascii_case("bun")
        || process_name.eq_ignore_ascii_case("bun.exe")
        || value.contains(" bun run ")
    {
        RuntimeKind::Bun
    } else if process_name.eq_ignore_ascii_case("deno")
        || process_name.eq_ignore_ascii_case("deno.exe")
    {
        RuntimeKind::Deno
    } else if process_name.eq_ignore_ascii_case("node")
        || process_name.eq_ignore_ascii_case("node.exe")
    {
        RuntimeKind::Node
    } else {
        RuntimeKind::Other
    }
}

pub fn parse_wsl_snapshot(distribution: &str, snapshot: &str) -> Vec<ServiceProcess> {
    #[derive(Debug)]
    struct ProcessMetadata {
        parent_pid: u32,
        process_name: String,
        command: String,
        cwd: Option<String>,
        start_token: String,
    }

    let mut processes = BTreeMap::<u32, ProcessMetadata>::new();
    let mut listeners = Vec::<(u32, u16, String)>::new();

    for record in snapshot.split('\x1e') {
        if let Some(payload) = record.strip_prefix("P\x1f") {
            let fields = payload.splitn(4, '\x1f').collect::<Vec<_>>();
            if fields.len() != 4 {
                continue;
            }
            let Ok(pid) = fields[0].parse::<u32>() else {
                continue;
            };
            let Some((parent_pid, process_name, start_token)) = parse_proc_stat(fields[1]) else {
                continue;
            };
            let cwd = normalized_optional(fields[2]);
            processes.insert(
                pid,
                ProcessMetadata {
                    parent_pid,
                    process_name,
                    command: fields[3].trim().to_owned(),
                    cwd,
                    start_token,
                },
            );
        } else if let Some(line) = record.strip_prefix("S\x1f") {
            let Some((host, port)) = parse_socket_address(line) else {
                continue;
            };
            for pid in parse_socket_pids(line) {
                listeners.push((pid, port, host.clone()));
            }
        }
    }

    let mut grouped = BTreeMap::<u32, ServiceProcess>::new();
    for (pid, port, host) in listeners {
        let Some(metadata) = processes.get(&pid) else {
            continue;
        };
        let runtime = classify_runtime(&metadata.process_name, &metadata.command);
        let service = grouped.entry(pid).or_insert_with(|| ServiceProcess {
            id: format!("wsl:{distribution}:{pid}:{}", metadata.start_token),
            origin: ProcessOrigin::Wsl,
            distribution: Some(distribution.to_owned()),
            pid,
            parent_pid: metadata.parent_pid,
            ports: Vec::new(),
            hosts: Vec::new(),
            process_name: metadata.process_name.clone(),
            command: metadata.command.clone(),
            cwd: metadata.cwd.clone(),
            project_name: metadata.cwd.as_deref().and_then(project_name),
            is_dev_server: runtime != RuntimeKind::Other,
            runtime,
            start_token: metadata.start_token.clone(),
        });
        push_listener(service, port, host);
    }

    finish_grouping(grouped)
}

pub fn group_native_listeners(records: Vec<NativeListenerRecord>) -> Vec<ServiceProcess> {
    let mut grouped = BTreeMap::<(u32, String), ServiceProcess>::new();

    for record in records {
        let key = (record.pid, record.start_token.clone());
        let runtime = classify_runtime(&record.process_name, &record.command);
        let service = grouped.entry(key).or_insert_with(|| ServiceProcess {
            id: format!("windows:{}:{}", record.pid, record.start_token),
            origin: ProcessOrigin::Windows,
            distribution: None,
            pid: record.pid,
            parent_pid: record.parent_pid,
            ports: Vec::new(),
            hosts: Vec::new(),
            process_name: record.process_name.clone(),
            command: record.command.clone(),
            cwd: record.cwd.clone(),
            project_name: record.cwd.as_deref().and_then(project_name),
            is_dev_server: runtime != RuntimeKind::Other,
            runtime,
            start_token: record.start_token.clone(),
        });
        push_listener(service, record.port, record.host);
    }

    finish_grouping(grouped)
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

fn contains_any(value: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| value.contains(pattern))
}

fn normalized_optional(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn project_name(path: &str) -> Option<String> {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .map(str::to_owned)
}

fn parse_proc_stat(stat: &str) -> Option<(u32, String, String)> {
    let command_start = stat.find('(')? + 1;
    let command_end = stat.rfind(") ")?;
    let process_name = stat[command_start..command_end].to_owned();
    let fields = stat[command_end + 2..]
        .split_whitespace()
        .collect::<Vec<_>>();
    let parent_pid = fields.get(1)?.parse::<u32>().ok()?;
    let start_token = fields.get(19)?.to_string();
    Some((parent_pid, process_name, start_token))
}

fn parse_socket_address(line: &str) -> Option<(String, u16)> {
    let local = line.split_whitespace().nth(3)?;
    let split_at = local.rfind(':')?;
    let host = local[..split_at]
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = local[split_at + 1..].parse::<u16>().ok()?;
    Some((host, port))
}

fn parse_socket_pids(line: &str) -> Vec<u32> {
    let mut remaining = line;
    let mut pids = Vec::new();
    while let Some(offset) = remaining.find("pid=") {
        remaining = &remaining[offset + 4..];
        let digits = remaining
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        if let Ok(pid) = digits.parse::<u32>()
            && !pids.contains(&pid)
        {
            pids.push(pid);
        }
    }
    pids
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
    services.sort_by_key(|service| service.ports.first().copied().unwrap_or_default());
    services
}
