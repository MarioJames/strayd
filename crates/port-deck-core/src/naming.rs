use crate::RuntimeKind;

fn segments(path: &str) -> Vec<&str> {
    path.split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect()
}

fn basename(path: &str) -> Option<&str> {
    let path = path.strip_suffix(" (deleted)").unwrap_or(path);
    segments(path)
        .last()
        .copied()
        .filter(|name| !name.ends_with(':'))
}

fn app_name(path: &str) -> Option<&str> {
    segments(path)
        .into_iter()
        .find_map(|part| part.strip_suffix(".app").filter(|name| !name.is_empty()))
}

pub(crate) fn project_name(path: &str) -> Option<String> {
    if let Some(name) = app_name(path) {
        return Some(name.into());
    }
    let mut parts = segments(path);
    // A dependency directory belongs to the containing project, not to node_modules.
    if let Some(index) = parts.iter().position(|part| *part == "node_modules") {
        parts.truncate(index);
    }
    while parts.last().is_some_and(|part| {
        matches!(
            part.to_ascii_lowercase().as_str(),
            "bin"
                | "sbin"
                | ".bin"
                | "src"
                | "lib"
                | "lib64"
                | "dist"
                | "build"
                | "target"
                | "debug"
                | "release"
                | "out"
                | "classes"
                | "__pycache__"
                | "venv"
                | ".venv"
                | "env"
                | ".env"
                | "projects"
                | "workspaces"
        )
    }) {
        parts.pop();
    }
    let name = *parts.last()?;
    let is_home = matches!(parts.as_slice(), ["Users" | "home", _] | ["root"])
        || (parts.len() == 3 && parts[0].ends_with(':') && parts[1].eq_ignore_ascii_case("Users"));
    let is_system = matches!(
        parts.as_slice(),
        ["usr"
            | "opt"
            | "Applications"
            | "tmp"
            | "private"
            | "var"
            | "etc"
            | "run"
            | "dev"
            | "proc"
            | "sys"
            | "home"
            | "Users"]
            | ["usr", "local"]
            | ["var", "tmp" | "lib" | "run"]
            | ["private", "tmp" | "var"]
    ) || (parts.len() == 2
        && parts[0].ends_with(':')
        && matches!(
            name.to_ascii_lowercase().as_str(),
            "windows" | "program files" | "program files (x86)" | "users"
        ))
        || ((path.starts_with("\\\\") || path.starts_with("//")) && parts.len() <= 2);
    if is_home || is_system || name.starts_with('.') || name.ends_with(':') {
        None
    } else {
        Some(name.into())
    }
}

pub(crate) fn display_name(
    pid: u32,
    process_name: &str,
    executable: Option<&str>,
    arguments: &[String],
    runtime: &RuntimeKind,
    project: Option<&str>,
) -> String {
    // Use structured argv, never split the joined command: paths can contain spaces.
    let executable = executable.filter(|path| !path.is_empty()).or_else(|| {
        arguments.first().map(String::as_str).filter(|path| {
            !path.is_empty()
                && (path.starts_with(['/', '\\'])
                    || path.as_bytes().get(1) == Some(&b':')
                    || !path.contains(char::is_whitespace))
        })
    });
    if let Some(name) = executable.and_then(app_name) {
        return name.into();
    }
    let binary = executable
        .and_then(basename)
        .or_else(|| basename(process_name))
        .unwrap_or("");
    let lower = binary.to_ascii_lowercase();
    let kind = lower.strip_suffix(".exe").unwrap_or(&lower);
    let is_python = kind == "python"
        || kind.strip_prefix("python").is_some_and(|version| {
            !version.is_empty() && version.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
        });
    let is_js = matches!(kind, "node" | "nodejs" | "bun" | "deno");
    let is_interpreter = is_js
        || is_python
        || matches!(
            kind,
            "ruby" | "perl" | "php" | "java" | "dotnet" | "sh" | "bash" | "zsh" | "fish"
        );

    // Framework processes can replace argv/the OS title (e.g. next-server).
    if !matches!(
        runtime,
        RuntimeKind::Other
            | RuntimeKind::Node
            | RuntimeKind::Bun
            | RuntimeKind::Deno
            | RuntimeKind::Cloudflared
            | RuntimeKind::Ngrok
            | RuntimeKind::SshTunnel
            | RuntimeKind::Frp
            | RuntimeKind::LocalTunnel
            | RuntimeKind::Bore
            | RuntimeKind::Sshd
    ) && let Some(project) = project
    {
        return project.into();
    }
    if is_interpreter {
        if let Some(entry) = entry_name(kind, arguments) {
            return entry;
        }
        if let Some(project) = project {
            return project.into();
        }
    }
    if binary.is_empty() {
        format!("PID {pid}")
    } else {
        binary.into()
    }
}

fn entry_name(kind: &str, arguments: &[String]) -> Option<String> {
    let mut args = arguments.iter().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            // Inline code is not a filename or a module name.
            "-c" | "-e" | "--eval" | "-p" | "--print" | "-Command" | "-EncodedCommand" => {
                return None;
            }
            "-m" | "--module" if kind.starts_with("python") || kind == "java" => {
                return args.next().cloned();
            }
            "-jar" => return args.next().and_then(|path| script_name(path)),
            "-r"
            | "--require"
            | "--import"
            | "--loader"
            | "--experimental-loader"
            | "-cp"
            | "-classpath"
            | "--class-path" => {
                args.next();
            }
            "--" => return args.next().and_then(|path| script_name(path)),
            "task" if kind == "deno" => return None,
            "run" if kind == "bun" => {
                let script = args.next()?;
                return (script.contains(['/', '\\', '.']))
                    .then(|| script_name(script))
                    .flatten();
            }
            "run" | "serve" if kind == "deno" => {}
            "-u" | "-B" | "-E" | "-I" | "-O" | "-OO" | "-s" | "-S" | "-q" | "-v"
                if kind.starts_with("python") => {}
            "-W" | "-X" | "--check-hash-based-pycs" if kind.starts_with("python") => {
                args.next();
            }
            "--inspect"
            | "--inspect-brk"
            | "--watch"
            | "--enable-source-maps"
            | "--no-warnings"
                if matches!(kind, "node" | "nodejs" | "bun") => {}
            _ if kind == "java" && (arg.starts_with("-X") || arg.starts_with("-D")) => {}
            _ if kind == "deno" && arg.starts_with("--allow-") => {}
            _ if matches!(kind, "node" | "nodejs" | "bun") && arg.starts_with("--inspect=") => {}
            // Unknown flags may consume a value: do not mistake that value for a script.
            _ if arg.starts_with('-') => return None,
            _ => return script_name(arg),
        }
    }
    None
}

fn script_name(path: &str) -> Option<String> {
    if let Some(name) = app_name(path) {
        return Some(name.into());
    }
    let parts = segments(path);
    if let Some(index) = parts.iter().rposition(|part| *part == "node_modules")
        && let Some(package) = parts.get(index + 1)
    {
        if matches!(*package, "npm" | "pnpm" | "yarn") {
            return None;
        }
        if *package == ".bin" {
            return basename(path).map(str::to_owned);
        }
        return if package.starts_with('@') {
            parts.get(index + 2).map(|name| format!("{package}/{name}"))
        } else {
            Some((*package).into())
        };
    }
    let filename = basename(path)?;
    let stem = [
        ".js", ".cjs", ".mjs", ".ts", ".py", ".rb", ".php", ".pl", ".sh", ".jar", ".dll",
    ]
    .iter()
    .find_map(|extension| filename.strip_suffix(extension))
    .unwrap_or(filename);
    if stem.is_empty()
        || matches!(
            stem,
            "index" | "main" | "server" | "app" | "cli" | "__main__" | "npm-cli" | "pnpm" | "yarn"
        )
    {
        path.rfind(['/', '\\'])
            .and_then(|index| project_name(&path[..index]))
    } else {
        Some(stem.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_applications_native_binaries_and_interpreter_entries() {
        type Case<'a> = (
            &'a str,
            Option<&'a str>,
            &'a [&'a str],
            Option<&'a str>,
            &'a str,
        );
        let cases: &[Case<'_>] = &[
            (
                "cfuse",
                Some("/Users/mocha/.local/bin/cfuse"),
                &["cfuse", "proxy", "--port", "9792"],
                None,
                "cfuse",
            ),
            (
                "cfuse",
                Some("/Users/mocha/.local/share/codefuse-cli/versions/v2.6.39/cfuse"),
                &["cfuse", "--cfuse-hub-daemon-runner"],
                Some("unrelated"),
                "cfuse",
            ),
            (
                "MainThread",
                Some("/usr/bin/node"),
                &["node", "server.js"],
                Some("ai-coding-trace"),
                "ai-coding-trace",
            ),
            (
                "波点音乐",
                Some("/Applications/波点音乐.app/Contents/MacOS/波点音乐"),
                &[],
                Some("mocha"),
                "波点音乐",
            ),
            (
                "Electron Helper",
                Some(
                    "/Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper.app/Contents/MacOS/Code Helper",
                ),
                &[],
                None,
                "Visual Studio Code",
            ),
            (
                "nginx: master",
                Some("/usr/sbin/nginx"),
                &[],
                Some("shop"),
                "nginx",
            ),
            (
                "cloudflared",
                Some("/usr/bin/cloudflared"),
                &[],
                Some("shop"),
                "cloudflared",
            ),
            (
                "thread-name",
                Some("/opt/very-long-service-name (deleted)"),
                &[],
                None,
                "very-long-service-name",
            ),
            (
                "service",
                Some(r"C:\Program Files\Example\service.exe"),
                &[],
                Some("work"),
                "service.exe",
            ),
            (
                "node",
                None,
                &["/usr/bin/node", "/opt/My Tool/worker.js"],
                Some("work"),
                "worker",
            ),
            (
                "python3",
                Some("/usr/bin/python3.12"),
                &["python3.12", "-m", "http.server", "9792"],
                Some("work"),
                "http.server",
            ),
            (
                "python3",
                None,
                &["python3", "-u", "/srv/api-service.py"],
                Some("work"),
                "api-service",
            ),
            (
                "java",
                None,
                &["java", "-Xmx1g", "-jar", "/srv/orders.jar"],
                Some("work"),
                "orders",
            ),
            (
                "java",
                None,
                &["java", "-cp", "/srv/classes", "com.example.Server"],
                None,
                "com.example.Server",
            ),
            (
                "dotnet",
                None,
                &["dotnet", "C:\\Apps\\Orders.dll"],
                None,
                "Orders",
            ),
            ("bash", None, &["bash", "-c", "echo secret"], None, "bash"),
            (
                "node",
                None,
                &["node", "--require", "loader.js", "/opt/tool/daemon.js"],
                None,
                "daemon",
            ),
            (
                "node",
                None,
                &["node", "-e", "require('http').createServer()"],
                None,
                "node",
            ),
            (
                "node",
                None,
                &["node", "/opt/node_modules/@scope/tool/dist/index.js"],
                Some("work"),
                "@scope/tool",
            ),
            (
                "node",
                None,
                &[
                    "node",
                    "/usr/lib/node_modules/npm/bin/npm-cli.js",
                    "run",
                    "dev",
                ],
                Some("shop"),
                "shop",
            ),
            (
                "bun",
                None,
                &["bun", "run", "/srv/my-proxy.ts"],
                None,
                "my-proxy",
            ),
            (
                "deno",
                None,
                &["deno", "run", "--allow-net", "/srv/my-proxy.ts"],
                None,
                "my-proxy",
            ),
            ("sshd: mocha", None, &[], None, "sshd: mocha"),
            (
                "sshd",
                None,
                &["sshd: /usr/sbin/sshd -D [listener] 0 of 10-100 startups"],
                None,
                "sshd",
            ),
            ("", None, &[], None, "PID 42"),
        ];
        for (process, exe, args, project, expected) in cases {
            let args = args.iter().map(|value| (*value).into()).collect::<Vec<_>>();
            assert_eq!(
                display_name(42, process, *exe, &args, &RuntimeKind::Other, *project),
                *expected,
                "process={process}, args={args:?}"
            );
        }
    }

    #[test]
    fn framework_names_use_the_project_and_runtime_options_are_not_entry_names() {
        let args = ["node", "/opt/node_modules/vite/bin/vite.js"].map(String::from);
        assert_eq!(
            display_name(42, "node", None, &args, &RuntimeKind::Vite, Some("shop")),
            "shop"
        );
        for args in [
            vec!["bun", "run", "dev"],
            vec!["deno", "task", "dev"],
            vec!["node", "--unknown-option", "not-a-script"],
        ] {
            let args = args.into_iter().map(String::from).collect::<Vec<_>>();
            assert_eq!(
                display_name(42, &args[0], None, &args, &RuntimeKind::Other, Some("shop")),
                "shop"
            );
        }
    }

    #[test]
    fn ignores_container_directories_but_keeps_project_roots() {
        for (path, expected) in [
            ("/srv/shop/target/release", Some("shop")),
            ("/srv/shop/node_modules/vite/bin", Some("shop")),
            ("/srv/shop/.venv/bin", Some("shop")),
            ("/Users/mocha/projects", None),
            ("/", None),
            (r"C:\", None),
            (r"C:\users\mocha", None),
            (r"\\server\share", None),
            (r"\\server\share\shop\bin", Some("shop")),
            ("/var/tmp", None),
            ("/srv/shop-ui/dist", Some("shop-ui")),
            ("/home/mocha/我的项目/src", Some("我的项目")),
        ] {
            assert_eq!(project_name(path).as_deref(), expected, "{path}");
        }
    }
}
