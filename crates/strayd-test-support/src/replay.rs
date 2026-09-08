use anyhow::{Result, ensure};
use port_deck_core::{
    HostPlatform, NativeListenerRecord, NativeProcessRecord, group_native_resources,
    group_related_services,
};
use serde::Deserialize;
use serde_json::json;
use std::{fs, path::Path};

use crate::report::Report;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Recording {
    schema_version: u32,
    id: String,
    source: String,
    reason: String,
    platform: HostPlatform,
    processes: Vec<RecordedProcess>,
    #[serde(default)]
    metadata: Option<CaptureMetadata>,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CaptureMetadata {
    arch: String,
    collector_version: String,
    application_version: String,
    captured_at: String,
    missing_fields: Vec<String>,
    redaction: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecordedProcess {
    pid: u32,
    parent_pid: u32,
    process_name: String,
    executable: Option<String>,
    arguments: Vec<String>,
    cwd: Option<String>,
    started_at: Option<u64>,
    ports: Vec<u16>,
    expected_name: String,
}

pub fn run(directory: &Path, report: &mut Report) -> Result<()> {
    let mut files = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.sort();
    for file in files.into_iter().filter(|path| {
        path.extension()
            .is_some_and(|extension| extension == "json")
    }) {
        let recording: Recording = serde_json::from_slice(&fs::read(&file)?)?;
        ensure!(
            recording.schema_version == 1,
            "unsupported recording schema"
        );
        ensure!(
            matches!(recording.source.as_str(), "synthetic" | "captured"),
            "recording must declare provenance"
        );
        if recording.source == "captured" {
            let metadata = recording
                .metadata
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("captured recording requires capture metadata"))?;
            ensure!(
                [
                    &metadata.arch,
                    &metadata.collector_version,
                    &metadata.application_version,
                    &metadata.captured_at,
                    &metadata.redaction
                ]
                .iter()
                .all(|value| !value.is_empty()),
                "capture metadata must state platform/version/date/redaction"
            );
        }
        report.case(&recording.id, &recording.source, |observed| {
            let mut listeners = Vec::new();
            let mut processes = Vec::new();
            for record in &recording.processes {
                let metadata = NativeProcessRecord {
                    pid: record.pid,
                    parent_pid: record.parent_pid,
                    process_name: record.process_name.clone(),
                    executable: record.executable.clone(),
                    arguments: record.arguments.clone(),
                    command: record.arguments.join(" "),
                    cwd: record.cwd.clone(),
                    started_at: record.started_at,
                    start_token: record.started_at.unwrap_or_default().to_string(),
                };
                for port in &record.ports {
                    listeners.push(NativeListenerRecord {
                        pid: metadata.pid,
                        parent_pid: metadata.parent_pid,
                        process_name: metadata.process_name.clone(),
                        executable: metadata.executable.clone(),
                        arguments: metadata.arguments.clone(),
                        command: metadata.command.clone(),
                        cwd: metadata.cwd.clone(),
                        started_at: metadata.started_at,
                        start_token: metadata.start_token.clone(),
                        port: *port,
                        host: "127.0.0.1".into(),
                    });
                }
                processes.push(metadata);
            }
            let groups = group_related_services(group_native_resources(
                recording.platform,
                listeners,
                processes,
            ));
            for record in &recording.processes {
                let service = groups
                    .iter()
                    .flat_map(|group| &group.services)
                    .find(|service| service.pid == record.pid)
                    .ok_or_else(|| anyhow::anyhow!("recorded PID {} missing", record.pid))?;
                observed.push(serde_json::to_value(service)?);
                ensure!(
                    service.display_name == record.expected_name,
                    "expected {}, got {}",
                    record.expected_name,
                    service.display_name
                );
                ensure!(
                    service.started_at == record.started_at,
                    "creation time lost during grouping"
                );
            }
            observed.push(json!({"reason": recording.reason,"actions_enabled":false}));
            if let Some(metadata) = &recording.metadata {
                observed.push(json!({"capture_metadata":metadata,"execution":"read-only-replay"}));
            }
            // This path never calls scan_all, terminate, a browser opener, or a shell.
            Ok(())
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_inputs_require_provenance_before_replay() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recording.json");
        let mut value = json!({"schemaVersion":1,"id":"capture-contract","source":"captured","reason":"test capture","platform":"linux","processes":[{"pid":12345,"parentPid":1,"processName":"tool","executable":"/tmp/tool","arguments":["/tmp/tool"],"cwd":"/tmp","startedAt":1720000000u64,"ports":[32145],"expectedName":"tool"}]});
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let error = run(directory.path(), &mut Report::new("replay")).unwrap_err();
        assert!(error.to_string().contains("capture metadata"));
        value["metadata"] = json!({"arch":"x86_64","collectorVersion":"test-1","applicationVersion":"test-1","capturedAt":"2026-09-08T00:00:00Z","missingFields":[],"redaction":"only a synthetic test path"});
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        let mut report = Report::new("replay");
        run(directory.path(), &mut report).unwrap();
        assert!(matches!(
            report.cases[0].status,
            crate::report::Status::Pass
        ));
        assert_eq!(report.cases[0].source, "captured");
        assert!(
            report.cases[0]
                .observations
                .iter()
                .any(|value| value["actions_enabled"] == false)
        );
    }
}
