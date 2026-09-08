use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, UdpSocket},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Ready {
    event: String,
    pid: u32,
    ports: Vec<u16>,
    children: Vec<u32>,
}

struct Children(Vec<Child>);
impl Drop for Children {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let has = |key: &str| args.iter().any(|arg| arg == key);
    let value = |key: &str| {
        args.windows(2)
            .find(|pair| pair[0] == key)
            .map(|pair| pair[1].clone())
    };
    let lifetime: u64 = value("--fixture-lifetime-ms")
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(120_000);
    ensure!(
        lifetime <= 300_000,
        "fixture lifetime must be at most five minutes"
    );
    if has("--fixture-fail") {
        anyhow::bail!("requested fixture startup failure");
    }
    let stop = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    {
        let term = if has("--fixture-ignore-term") {
            Arc::new(AtomicBool::new(false))
        } else {
            stop.clone()
        };
        signal_hook::flag::register(signal_hook::consts::SIGTERM, term)?;
        signal_hook::flag::register(signal_hook::consts::SIGINT, stop.clone())?;
    }
    let mut sockets = Vec::new();
    let mut ports = Vec::new();
    let count: usize = value("--fixture-ports")
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(1);
    ensure!(count <= 64, "fixture supports at most 64 listeners");
    let host = if has("--fixture-ipv6") {
        "[::1]:0"
    } else if has("--fixture-wildcard") {
        "0.0.0.0:0"
    } else {
        "127.0.0.1:0"
    };
    for _ in 0..count {
        let listener = if let Some(port) = value("--fixture-bind-port") {
            ensure!(count == 1, "explicit binding needs one port");
            let bind_host = value("--fixture-bind-host").unwrap_or_else(|| "127.0.0.1".into());
            ensure!(
                bind_host.parse::<std::net::IpAddr>()?.is_loopback(),
                "explicit binding must be loopback"
            );
            TcpListener::bind((bind_host.as_str(), port.parse::<u16>()?))
        } else {
            TcpListener::bind(host)
        }
        .context("fixture TCP bind")?;
        ports.push(listener.local_addr()?.port());
        sockets.push(listener);
    }
    if has("--fixture-same-port") {
        let port = *ports.first().context("same-port needs one listener")?;
        sockets.push(TcpListener::bind(("127.0.0.2", port))?);
    }
    let _udp = if has("--fixture-udp") {
        Some(UdpSocket::bind("127.0.0.1:0")?)
    } else {
        None
    };
    let mut children = Children(Vec::new());
    let mut child_ids = Vec::new();
    let child_count: usize = value("--fixture-children")
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(0);
    ensure!(child_count <= 8, "fixture supports at most eight children");
    for _ in 0..child_count {
        let mut child = Command::new(std::env::current_exe()?)
            .args([
                "--fixture-lifetime-ms",
                &value("--fixture-child-lifetime-ms").unwrap_or_else(|| lifetime.to_string()),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdout = child.stdout.take().context("child stdout")?;
        let pid = child.id();
        children.0.push(child);
        let (send, receive) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
            let _ = send.send(result);
        });
        let line = receive
            .recv_timeout(Duration::from_secs(5))
            .context("child readiness timeout")??;
        let ready: Ready = serde_json::from_str(&line).context("child readiness")?;
        ensure!(ready.pid == pid, "child PID mismatch");
        child_ids.push(pid);
    }
    if !has("--fixture-no-ready") {
        let ready = serde_json::to_string(&Ready {
            event: "ready".into(),
            pid: std::process::id(),
            ports,
            children: child_ids,
        })?;
        if let Some(path) = value("--fixture-ready-file") {
            let path = PathBuf::from(path);
            let temporary = path.with_extension("pending");
            std::fs::write(&temporary, &ready)?;
            std::fs::rename(temporary, path)?;
        }
        println!("{ready}");
        std::io::stdout().flush()?;
    }
    if !has("--fixture-detached") {
        let stop = stop.clone();
        thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                if line.is_err() || matches!(line.as_deref(), Ok("quit")) {
                    break;
                }
            }
            stop.store(true, Ordering::SeqCst);
        });
    }
    let started = Instant::now();
    while !stop.load(Ordering::SeqCst) && started.elapsed() < Duration::from_millis(lifetime) {
        thread::sleep(Duration::from_millis(20));
    }
    drop(sockets);
    Ok(())
}
