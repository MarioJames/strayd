use anyhow::ensure;
use serde_json::json;
use std::{
    fs,
    process::{Command, Stdio},
    time::Duration,
};

use crate::{cases::Environment, process::ManagedChild, report::Report};

pub fn available(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn run(env: &Environment, report: &mut Report, extended: bool) {
    for runtime in [
        "node",
        "node-title",
        "bun",
        "python",
        "python-module",
        "deno",
        "java",
        "dotnet",
    ] {
        if !extended && matches!(runtime, "deno" | "java" | "dotnet") {
            continue;
        }
        if runtime == "node-title" && !cfg!(target_os = "linux") {
            report.unavailable(
                "runtime-node-title",
                "native-fixture",
                false,
                "Linux process.title contract",
            );
            continue;
        }
        let executable = match runtime {
            "node-title" => "node",
            "python" | "python-module" => {
                if cfg!(windows) {
                    "python"
                } else {
                    "python3"
                }
            }
            other => other,
        };
        if !available(executable)
            || (runtime == "java" && (!available("javac") || !available("jar")))
        {
            report.unavailable(&format!("runtime-{runtime}"), "native-fixture", true, &format!("required runtime/toolchain {executable} is not installed (Java requires javac and jar)"));
            continue;
        }
        report.case(&format!("runtime-{runtime}"), "native-fixture", |observed| {
            let root = env.root.join(format!("runtime-{runtime}"));
            let directory = root.join("ai-coding-trace/bin");
            fs::create_dir_all(&directory)?;
            let mut command = Command::new(executable);
            command.current_dir(&directory);
            command.env("DENO_DIR", root.join("deno-cache"));
            let expected = match runtime {
                "node" | "node-title" | "bun" | "deno" => {
                    let script = directory.join("server.mjs");
                    fs::copy(env.repository.join("tests/apps/server.mjs"), &script)?;
                    if runtime == "deno" { command.args(["run", "--allow-net", "--allow-env"]); }
                    if runtime == "node-title" { command.env("STRAYD_FIXTURE_TITLE", "next-server (fixture)"); }
                    command.arg(script);
                    "ai-coding-trace"
                }
                "python" => {
                    let script = directory.join("server.py");
                    fs::copy(env.repository.join("tests/apps/server.py"), &script)?;
                    command.arg(script);
                    "ai-coding-trace"
                }
                "python-module" => {
                    fs::create_dir_all(directory.join("trace_probe"))?;
                    fs::copy(env.repository.join("tests/apps/server.py"), directory.join("trace_probe/__main__.py"))?;
                    command.args(["-m", "trace_probe"]);
                    "trace_probe"
                }
                "java" => {
                    let source = directory.join("FixtureServer.java");
                    fs::copy(env.repository.join("tests/apps/FixtureServer.java"), &source)?;
                    ensure!(Command::new("javac").arg(&source).status()?.success(), "javac failed");
                    let jar = directory.join("orders.jar");
                    ensure!(Command::new("jar").current_dir(&directory).args(["--create", "--file"]).arg(&jar)
                        .args(["--main-class", "FixtureServer", "FixtureServer.class"]).status()?.success(), "jar failed");
                    command.args(["-Djava.net.preferIPv4Stack=true", "-jar"]).arg(jar);
                    "orders"
                }
                "dotnet" => {
                    for name in ["Fixture.csproj", "Program.cs"] { fs::copy(env.repository.join("tests/apps/dotnet").join(name), directory.join(name))?; }
                    let output = Command::new("dotnet").current_dir(&directory).args(["build", "Fixture.csproj", "--nologo", "--disable-build-servers", "-p:UseSharedCompilation=false", "--output", "out"])
                        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1").env("DOTNET_CLI_HOME", root.join("dotnet-home")).env("MSBUILDDISABLENODEREUSE", "1").output()?;
                    ensure!(output.status.success(), "dotnet fixture build failed: {}", String::from_utf8_lossy(&output.stdout));
                    command.arg(directory.join("out/Orders.dll"));
                    "Orders"
                }
                _ => unreachable!(),
            };
            let mut child = ManagedChild::start(&mut command, &env.registry, &root.join("stderr.log"), Duration::from_secs(10))?;
            child.assert_listening()?;
            let service = env.scan(&child, observed)?;
            ensure!(service.display_name == expected, "{runtime}: expected {expected}, got {}", service.display_name);
            if runtime == "node-title" { ensure!(service.process_name.starts_with("next-server") && service.runtime == port_deck_core::RuntimeKind::NextJs, "OS process title change was not observed and classified"); }
            let version = Command::new(executable).arg("--version").output()?;
            observed.push(json!({"runtime":runtime,"version":String::from_utf8_lossy(&version.stdout).trim()}));
            child.cleanup()
        });
    }
}

pub fn macos_app(env: &Environment, report: &mut Report) {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = env;
        report.unavailable(
            "macos-app-bundle",
            "native-fixture",
            false,
            "requires macOS",
        );
    }
    #[cfg(target_os = "macos")]
    report.case("macos-app-bundle", "native-fixture", |observed| {
        let contents = env.root.join("测试应用.app/Contents");
        fs::create_dir_all(contents.join("MacOS"))?;
        fs::write(contents.join("Info.plist"), r#"<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>fixture</string><key>CFBundleIdentifier</key><string>test.strayd.fixture</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>"#)?;
        fs::copy(&env.fixture, contents.join("MacOS/fixture"))?;
        let mut command = Command::new(contents.join("MacOS/fixture"));
        command.current_dir(&contents);
        let mut child = ManagedChild::start(&mut command, &env.registry, &env.root.join("macos-app.log"), Duration::from_secs(5))?;
        let service = env.scan(&child, observed)?;
        ensure!(service.display_name == "测试应用", "bundle name was not used");
        child.cleanup()
    });
}
