use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde_json::json;
use std::{
    fs,
    io::{Read, Write},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

use crate::{cases::Environment, process::alive, report::Report};

pub struct Session {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    receiver: Receiver<Vec<u8>>,
    parser: vt100::Parser,
    pub bytes: Vec<u8>,
    #[cfg(unix)]
    original_flags: libc::tcflag_t,
}

impl Session {
    pub fn start(env: &Environment, args: &[&str], rows: u16, cols: u16) -> Result<Self> {
        let pair = native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut command = CommandBuilder::new(&env.cli[0]);
        command.args(&env.cli[1..]);
        command.args(args);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env_remove("NO_COLOR");
        #[cfg(unix)]
        let original_flags = terminal_flags(pair.master.as_ref())?;
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        if let Some(pid) = child.process_id() {
            env.registry.register(pid)?;
        }
        let writer = pair.master.take_writer()?;
        let mut reader = pair.master.try_clone_reader()?;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0; 8192];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 || sender.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            master: pair.master,
            child,
            writer,
            receiver,
            parser: vt100::Parser::new(rows, cols, 0),
            bytes: Vec::new(),
            #[cfg(unix)]
            original_flags,
        })
    }
    fn receive(&mut self) {
        if let Ok(bytes) = self.receiver.recv_timeout(Duration::from_millis(40)) {
            self.parser.process(&bytes);
            self.bytes.extend(bytes);
        }
        while let Ok(bytes) = self.receiver.try_recv() {
            self.parser.process(&bytes);
            self.bytes.extend(bytes);
        }
    }
    pub fn screen(&self) -> String {
        self.parser.screen().contents()
    }
    pub fn wait(&mut self, predicate: impl Fn(&str) -> bool) -> Result<()> {
        let start = Instant::now();
        loop {
            self.receive();
            if predicate(&self.screen()) {
                return Ok(());
            }
            ensure!(
                start.elapsed() < Duration::from_secs(7),
                "PTY condition timed out; fixture screen:\n{}",
                self.screen()
            );
        }
    }
    pub fn send(&mut self, value: &[u8]) -> Result<()> {
        self.writer.write_all(value)?;
        self.writer.flush()?;
        Ok(())
    }
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        self.parser.set_size(rows, cols);
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        // Wait for the application to redraw and update its input regions at the new size.
        self.wait(|screen| {
            screen
                .lines()
                .nth(usize::from(rows.saturating_sub(3)))
                .is_some_and(|line| line.trim_start().starts_with('└'))
        })
    }
    pub fn copy(&mut self, label: &str, command: &str, cols: u16) -> Result<()> {
        self.wait(|screen| screen.contains(label))?;
        let row = self
            .screen()
            .lines()
            .position(|line| line.contains(label))
            .context("copy row")?
            + 1;
        let col = cols * 3 / 4;
        let offset = self.bytes.len();
        self.send(format!("\x1b[<0;{col};{row}M\x1b[<0;{col};{row}m").as_bytes())?;
        let expected = format!("\x1b]52;c;{}\x07", STANDARD.encode(command));
        let start = Instant::now();
        loop {
            self.receive();
            if self.bytes[offset..]
                .windows(expected.len())
                .any(|bytes| bytes == expected.as_bytes())
            {
                return Ok(());
            }
            ensure!(
                start.elapsed() < Duration::from_secs(5),
                "copy button did not emit the complete command via OSC 52"
            );
        }
    }
    pub fn scroll_to(&mut self, label: &str) -> Result<()> {
        for _ in 0..20 {
            if self.screen().contains(label) {
                return Ok(());
            }
            self.send(b"\x1b[6~")?;
            let until = Instant::now() + Duration::from_millis(150);
            while Instant::now() < until {
                self.receive();
            }
        }
        anyhow::bail!("detail scrolling did not reach {label}: {}", self.screen())
    }
    pub fn quit(&mut self, control_c: bool) -> Result<()> {
        self.send(if control_c { b"\x03" } else { b"q" })?;
        let start = Instant::now();
        loop {
            self.receive();
            if let Some(status) = self.child.try_wait()? {
                ensure!(status.success(), "TUI exited unsuccessfully");
                break;
            }
            ensure!(
                start.elapsed() < Duration::from_secs(4),
                "TUI failed to quit"
            );
        }
        self.receive();
        ensure!(
            self.bytes.windows(8).any(|bytes| bytes == b"\x1b[?1049l"),
            "alternate screen was not released"
        );
        #[cfg(unix)]
        ensure!(
            terminal_flags(self.master.as_ref())? == self.original_flags,
            "terminal mode was not restored"
        );
        Ok(())
    }
}

#[cfg(unix)]
fn terminal_flags(master: &dyn MasterPty) -> Result<libc::tcflag_t> {
    let fd = master.as_raw_fd().context("PTY file descriptor")?;
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    // tcgetattr initializes termios on success; the descriptor is owned by master.
    ensure!(
        unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) } == 0,
        "read PTY attributes"
    );
    Ok(unsafe { termios.assume_init() }.c_lflag)
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn run(env: &Environment, report: &mut Report) {
    for (language, started, uptime, copy, confirm) in [
        ("en", "Started", "Uptime", "COPY COMMAND", "CONFIRM STOP"),
        ("zh-cn", "创建时间", "运行时长", "复制命令", "确认关闭"),
    ] {
        report.case(&format!("tui-{language}-lifecycle"), "native-fixture", |observed| {
            let long_arg = "a-long-fixture-argument-".repeat(18);
            let mut first = env.fixture(&format!("tui-{language}-one"), "tui-one", None, &[&long_arg])?;
            let mut second = env.fixture(&format!("tui-{language}-two"), "tui-two", None, &[&long_arg])?;
            let snapshot = port_deck_engine::scan_all();
            let owned = [first.child.id(), second.child.id()];
            let hidden = snapshot.groups.iter().flat_map(|group| &group.services).filter(|service| !owned.contains(&service.pid)).map(|service| &service.id).collect::<Vec<_>>();
            let mut services = snapshot.groups.iter().flat_map(|group| &group.services).filter(|service| owned.contains(&service.pid)).collect::<Vec<_>>();
            services.sort_by_key(|service| service.ports[0]);
            ensure!(services.len() == 2, "TUI fixtures missing");
            let config = env.root.join(format!("tui-{language}.toml"));
            // Isolate visible resources without modifying the scanner or a user's config.
            fs::write(&config, if hidden.is_empty() { "version = 1\n".into() } else {format!("version = 1\n[[display.hide]]\nids = {}\n", serde_json::to_string(&hidden)?)} )?;
            let mut pty = Session::start(env, &["--config", config.to_str().unwrap(), "--language", language, "tui", "--refresh", "0"], 40, 132)?;
            pty.wait(|screen| screen.contains(started) && screen.contains(uptime))?;
            let initial_time = pty.screen().lines().find(|line| line.contains(uptime)).context("uptime line")?.to_owned();
            pty.wait(|screen| screen.lines().find(|line| line.contains(uptime)).is_some_and(|line| line != initial_time))?;
            pty.send(b"\x1b[F")?;
            pty.wait(|screen| screen.contains(&format!("PID {}", services[1].pid)))?;
            pty.copy(copy, &services[1].command, 132)?;
            pty.resize(18, 100)?;
            pty.scroll_to(copy)?;
            pty.copy(copy, &services[1].command, 100)?;
            fs::write(env.output.join(format!("tui-{language}-scrolled.txt")), pty.screen())?;
            pty.send(b"\x1b[A")?;
            pty.wait(|screen| screen.contains(&format!("PID {}", services[0].pid)) && screen.contains(started))?;
            pty.resize(24, 80)?;
            pty.send(b"s")?;
            pty.wait(|screen| screen.contains(confirm))?;
            pty.send(b"\x1b")?;
            pty.wait(|screen| !screen.contains(confirm))?;
            ensure!(alive(&first.identity) && alive(&second.identity), "cancel stopped a fixture");
            pty.resize(40, 132)?;
            pty.send(b"h")?;
            pty.wait(|screen| screen.contains(if language == "en" {"CREATE HIDE RULE"} else {"创建隐藏规则"}))?;
            pty.send(b"\x1b")?;
            pty.wait(|screen| !screen.contains(if language == "en" {"CREATE HIDE RULE"} else {"创建隐藏规则"}))?;
            pty.send(b",")?;
            pty.wait(|screen| screen.contains(if language == "en" {"SETTINGS / HIDDEN RULES"} else {"设置 / 隐藏规则"}))?;
            pty.send(b"\x1b")?;
            pty.wait(|screen| screen.contains(started))?;
            pty.send(b"h")?;
            pty.wait(|screen| screen.contains(if language == "en" {"CREATE HIDE RULE"} else {"创建隐藏规则"}))?;
            pty.send(b"\r")?;
            pty.wait(|screen| !screen.contains(if language == "en" {"CREATE HIDE RULE"} else {"创建隐藏规则"}) && screen.contains(&format!("PID {}", services[1].pid)))?;
            ensure!(env.cli_services(&["--config", config.to_str().unwrap(), "list", "--port", &services[0].ports[0].to_string(), "--json"])?.is_empty(), "TUI hide rule was not persisted for a new CLI process");
            ensure!(alive(&first.identity) && alive(&second.identity), "hiding killed a process");
            ensure!(services[1].manager_unit.is_none(), "refusing a confirmed TUI stop for an inherited systemd unit");
            pty.send(b"s")?;
            pty.wait(|screen| screen.contains(confirm))?;
            pty.send(b"\r")?;
            pty.wait(|screen| !screen.contains(confirm) && !screen.contains(&format!("PID {}", services[1].pid)))?;
            let (stopped, control) = if services[1].pid == first.child.id() {(&first.identity, &second.identity)} else {(&second.identity, &first.identity)};
            ensure!(!alive(stopped) && alive(control), "confirmed TUI stop did not preserve the hidden control process");
            pty.quit(language == "zh-cn")?;
            observed.push(json!({"language": language,"sizes":[[132,40],[100,18],[80,24]],"uptime_advanced_without_scan":true,"full_command_copied_after_scroll":true,"selection_reset":true,"cancel_preserved_processes":true,"hide_persisted":true,"confirmed_stop_preserved_control":true,"terminal_restored":true}));
            first.cleanup()?; second.cleanup()
        });
    }
}
