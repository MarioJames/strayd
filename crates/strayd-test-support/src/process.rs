use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::{unix_seconds, until};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Ready {
    pub event: String,
    pub pid: u32,
    pub ports: Vec<u16>,
    #[serde(default)]
    pub children: Vec<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Identity {
    pub pid: u32,
    pub start: u64,
    pub executable: Option<PathBuf>,
}

fn system(pid: u32) -> System {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
    );
    system
}

pub fn identity(pid: u32) -> Result<Identity> {
    let system = system(pid);
    let process = system
        .process(Pid::from_u32(pid))
        .context("owned process disappeared")?;
    Ok(Identity {
        pid,
        start: process.start_time(),
        executable: process.exe().map(Path::to_path_buf),
    })
}

pub fn alive(identity: &Identity) -> bool {
    let system = system(identity.pid);
    system
        .process(Pid::from_u32(identity.pid))
        .is_some_and(|process| {
            process.start_time() == identity.start
                && !matches!(
                    process.status(),
                    sysinfo::ProcessStatus::Zombie | sysinfo::ProcessStatus::Dead
                )
        })
}

pub fn kill_owned(identity: &Identity) -> Result<()> {
    ensure!(
        identity.pid > 4 && identity.pid != std::process::id(),
        "invalid cleanup target"
    );
    let system = system(identity.pid);
    if let Some(process) = system.process(Pid::from_u32(identity.pid)) {
        if process.start_time() != identity.start {
            return Ok(());
        }
        if matches!(
            process.status(),
            sysinfo::ProcessStatus::Zombie | sysinfo::ProcessStatus::Dead
        ) {
            return Ok(());
        }
        ensure!(
            process.start_time() == identity.start
                && process.exe().map(Path::to_path_buf) == identity.executable,
            "cleanup identity changed for PID {}",
            identity.pid
        );
        if !matches!(
            process.status(),
            sysinfo::ProcessStatus::Zombie | sysinfo::ProcessStatus::Dead
        ) {
            ensure!(process.kill(), "could not clean PID {}", identity.pid);
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct Registry {
    path: PathBuf,
    identities: Arc<Mutex<Vec<Identity>>>,
}

impl Registry {
    pub fn new(path: PathBuf) -> Result<Self> {
        let registry = Self {
            path,
            identities: Arc::new(Mutex::new(Vec::new())),
        };
        registry.save()?;
        Ok(registry)
    }
    pub fn register(&self, pid: u32) -> Result<Identity> {
        let identity = identity(pid)?;
        self.identities.lock().unwrap().push(identity.clone());
        self.save()?;
        Ok(identity)
    }
    fn save(&self) -> Result<()> {
        let pending = self.path.with_extension("pending.json");
        fs::write(
            &pending,
            serde_json::to_vec_pretty(&*self.identities.lock().unwrap())?,
        )?;
        fs::rename(pending, &self.path)?;
        Ok(())
    }
    pub fn cleanup(&self) -> Result<()> {
        cleanup_identities(&self.identities.lock().unwrap())
    }
    pub fn clean_file(path: &Path) -> Result<()> {
        let identities: Vec<Identity> = serde_json::from_slice(&fs::read(path)?)?;
        cleanup_identities(&identities)
    }
}

fn cleanup_identities(identities: &[Identity]) -> Result<()> {
    // Capture descendants while their owned parent still exists. This also covers
    // npm's Node wrapper and runtimes that start helper processes.
    let mut targets = identities.to_vec();
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
    );
    let mut index = 0;
    while index < targets.len() {
        let parent = &targets[index];
        if system.process(Pid::from_u32(parent.pid)).is_some_and(|p| {
            p.start_time() == parent.start && p.exe().map(Path::to_path_buf) == parent.executable
        }) {
            let parent_pid = Pid::from_u32(parent.pid);
            for (pid, child) in system.processes() {
                if child.parent() == Some(parent_pid)
                    && !targets.iter().any(|known| known.pid == pid.as_u32())
                {
                    targets.push(Identity {
                        pid: pid.as_u32(),
                        start: child.start_time(),
                        executable: child.exe().map(Path::to_path_buf),
                    });
                }
            }
        }
        index += 1;
    }
    let mut errors = Vec::new();
    for target in targets.iter().rev() {
        if let Err(error) = kill_owned(target) {
            errors.push(error.to_string());
        }
    }
    until(Duration::from_secs(3), || {
        Ok(targets.iter().all(|target| !alive(target)))
    })?;
    ensure!(errors.is_empty(), "cleanup errors: {}", errors.join("; "));
    Ok(())
}

pub struct ManagedChild {
    pub log: PathBuf,
    pub child: Child,
    pub identity: Identity,
    pub ready: Ready,
    pub before: u64,
    pub after: u64,
    children: Vec<Identity>,
}

impl ManagedChild {
    pub fn spawn_unready(command: &mut Command, registry: &Registry, log: &Path) -> Result<Self> {
        let before = unix_seconds();
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::from(File::create(log)?))
            .spawn()?;
        let identity = match registry.register(child.id()) {
            Ok(identity) => identity,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self {
            ready: Ready {
                event: "spawned".into(),
                pid: child.id(),
                ports: Vec::new(),
                children: Vec::new(),
            },
            child,
            identity,
            log: log.to_path_buf(),
            before,
            after: unix_seconds(),
            children: Vec::new(),
        })
    }
    pub fn start(
        command: &mut Command,
        registry: &Registry,
        log: &Path,
        timeout: Duration,
    ) -> Result<Self> {
        let before = unix_seconds();
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(File::create(log)?))
            .spawn()
            .context("spawn test process")?;
        let identity = match registry.register(child.id()) {
            Ok(identity) => identity,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let stdout = child.stdout.take().context("test stdout")?;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
            let _ = sender.send(result);
        });
        let ready = receiver
            .recv_timeout(timeout)
            .context("process readiness timed out")
            .and_then(|result| result.context("readiness pipe"))
            .and_then(|line| {
                serde_json::from_str::<Ready>(&line).context("invalid readiness event")
            });
        let ready = match ready {
            Ok(ready) if ready.event == "ready" && ready.pid == child.id() => ready,
            result => {
                // EOF may arrive just before the child's final error is flushed.
                let _ = until(Duration::from_millis(100), || {
                    Ok(child.try_wait()?.is_some())
                });
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!(
                    "test process did not become ready: {}; stderr: {}",
                    result
                        .err()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "PID/event mismatch".into()),
                    fs::read_to_string(log)
                        .unwrap_or_default()
                        .chars()
                        .take(4096)
                        .collect::<String>()
                );
            }
        };
        let mut managed = Self {
            log: log.to_path_buf(),
            child,
            identity,
            ready,
            before,
            after: unix_seconds(),
            children: Vec::new(),
        };
        for pid in &managed.ready.children {
            managed.children.push(registry.register(*pid)?);
        }
        Ok(managed)
    }

    pub fn assert_listening(&self) -> Result<()> {
        for port in &self.ready.ports {
            let v4: SocketAddr = format!("127.0.0.1:{port}").parse()?;
            let v6: SocketAddr = format!("[::1]:{port}").parse()?;
            ensure!(
                TcpStream::connect_timeout(&v4, Duration::from_millis(200)).is_ok()
                    || TcpStream::connect_timeout(&v6, Duration::from_millis(200)).is_ok(),
                "fixture port {port} is not listening"
            );
        }
        Ok(())
    }

    pub fn cleanup(&mut self) -> Result<()> {
        // Closing the owned control pipe asks a healthy fixture to stop and reap its children.
        drop(self.child.stdin.take());
        if until(Duration::from_millis(500), || {
            Ok(self.child.try_wait()?.is_some())
        })
        .is_err()
        {
            for target in self.children.iter().rev() {
                kill_owned(target)?;
            }
            if self.child.try_wait()?.is_none() {
                self.child.kill()?;
            }
            self.child.wait()?;
        }
        for target in &self.children {
            if alive(target) {
                kill_owned(target)?;
            }
        }
        until(Duration::from_secs(2), || {
            Ok(!alive(&self.identity) && self.children.iter().all(|target| !alive(target)))
        })?;
        Ok(())
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
