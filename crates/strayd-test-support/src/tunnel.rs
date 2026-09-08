use crate::{
    cases::Environment,
    process::{ManagedChild, alive},
    report::Report,
    until,
};
use anyhow::{Context, ensure};
use serde_json::json;
use std::{
    fs,
    io::Read,
    net::TcpStream,
    process::{Command, Stdio},
    time::Duration,
};

pub fn run(env: &Environment, report: &mut Report) {
    let python = if cfg!(windows) { "python" } else { "python3" };
    if !Command::new(python)
        .args(["-c", "import paramiko"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
        || Command::new("ssh")
            .arg("-V")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_err()
    {
        report.unavailable(
            "local-ssh-reverse-forward",
            "native-fixture",
            true,
            "requires OpenSSH, Node and Python Paramiko; use the Linux test image",
        );
        return;
    }
    report.case("local-ssh-reverse-forward", "native-fixture", |observed| {
        let mut source_command = Command::new("node"); source_command.arg(env.repository.join("tests/apps/server.mjs"));
        let mut source = ManagedChild::start(&mut source_command, &env.registry, &env.root.join("source.log"), Duration::from_secs(5))?;
        let forward_file = env.root.join("forward.json");
        let mut server_command = Command::new(python); server_command.arg(env.repository.join("tests/apps/ssh_server.py")).arg(&forward_file);
        let mut server = ManagedChild::start(&mut server_command, &env.registry, &env.root.join("ssh-peer.log"), Duration::from_secs(5))?;
        let mut command = Command::new("ssh");
        command.args(["-F", if cfg!(windows) {"NUL"} else {"/dev/null"}, "-N", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null", "-o", "ExitOnForwardFailure=yes", "-o", "ConnectTimeout=5", "-p", &server.ready.ports[0].to_string(), "-R", &format!("0:127.0.0.1:{}", source.ready.ports[0]), "strayd-fixture@127.0.0.1"]);
        let mut client = ManagedChild::spawn_unready(&mut command, &env.registry, &env.root.join("ssh-client.log"))?;
        until(Duration::from_secs(8), || Ok(forward_file.exists()))
            .with_context(|| fs::read_to_string(env.root.join("ssh-client.log")).unwrap_or_default())?;
        let ready: serde_json::Value = serde_json::from_slice(&fs::read(&forward_file)?)?;
        let remote_port = u16::try_from(ready["port"].as_u64().context("forwarded port")?)?;
        until(Duration::from_secs(5), || Ok(fs::read_to_string(env.root.join("ssh-client.log")).unwrap_or_default().contains(&format!("Allocated port {remote_port}"))))?;
        let mut socket = TcpStream::connect(("127.0.0.1", remote_port))?;
        socket.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut response = vec![0; b"strayd-fixture\n".len()];
        socket.read_exact(&mut response).with_context(|| format!("SSH payload failed; peer: {}; client: {}", fs::read_to_string(env.root.join("ssh-peer.log")).unwrap_or_default(), fs::read_to_string(env.root.join("ssh-client.log")).unwrap_or_default()))?;
        ensure!(response == b"strayd-fixture\n", "real SSH tunnel did not reach the owned source");
        let snapshot = port_deck_engine::scan_all();
        let group = snapshot.groups.iter().find(|g| g.services.iter().any(|s| s.pid == client.child.id())).context("OpenSSH tunnel not discovered")?;
        ensure!(group.services.iter().any(|s| s.pid == source.child.id()), "real reverse tunnel was not grouped with its source");
        let tunnel = group.services.iter().find(|s| s.pid == client.child.id()).unwrap();
        ensure!(tunnel.runtime == port_deck_core::RuntimeKind::SshTunnel, "OpenSSH classification failed");
        env.terminate(tunnel, &client)?;
        ensure!(alive(&source.identity) && alive(&server.identity), "stopping tunnel killed its source or peer");
        observed.push(json!({"source_pid":source.child.id(),"tunnel_pid":client.child.id(),"ssh_peer_pid":server.child.id(),"remote_port":remote_port,"end_to_end_payload":String::from_utf8_lossy(&response),"grouped":true,"source_preserved":true}));
        client.cleanup()?; server.cleanup()?; source.cleanup()
    });
}
