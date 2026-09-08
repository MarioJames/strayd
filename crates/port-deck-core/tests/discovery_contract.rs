use port_deck_core::{
    HostPlatform, IdentityError, NativeListenerRecord, NativeProcessRecord, ResourceKind,
    RuntimeKind, TunnelTarget, classify_runtime, decode_command_output, ensure_process_identity,
    extract_tunnel_target, group_native_resources, group_related_services, is_safe_service_unit,
};

#[test]
fn decodes_utf16le_command_output() {
    let bytes = [
        b'D', 0, b'e', 0, b'b', 0, b'i', 0, b'a', 0, b'n', 0, b'\r', 0, b'\n', 0,
    ];

    assert_eq!(decode_command_output(&bytes), "Debian\r\n");
}

#[test]
fn groups_native_listeners_for_the_same_process() {
    let base = NativeListenerRecord {
        port: 5173,
        host: "0.0.0.0".into(),
        pid: 8200,
        parent_pid: 7900,
        process_name: "node.exe".into(),
        executable: None,
        arguments: Vec::new(),
        command: r#""C:\Program Files\nodejs\node.exe" C:\Projects\app-shell\node_modules\vite\bin\vite.js"#.into(),
        cwd: Some(r"C:\Projects\app-shell".into()),
        started_at: Some(1720000000),
        start_token: "13432622".into(),
    };
    let services = group_native_resources(
        HostPlatform::Windows,
        vec![
            base.clone(),
            NativeListenerRecord {
                port: 24678,
                host: "127.0.0.1".into(),
                ..base
            },
        ],
        Vec::new(),
    );

    assert_eq!(services.len(), 1);
    let service = &services[0];
    assert_eq!(service.platform, HostPlatform::Windows);
    assert_eq!(service.ports, vec![5173, 24678]);
    assert_eq!(service.runtime, RuntimeKind::Vite);
    assert_eq!(service.resource_kind, ResourceKind::Development);
    assert!(service.can_terminate);
    assert_eq!(service.cwd.as_deref(), Some(r"C:\Projects\app-shell"));
    assert_eq!(service.project_name.as_deref(), Some("app-shell"));
}

#[test]
fn recognizes_common_frontend_dev_runtimes() {
    let cases = [
        (
            "node",
            "node node_modules/next/dist/bin/next dev",
            RuntimeKind::NextJs,
        ),
        (
            "node",
            "node /repo/node_modules/vite/bin/vite.js --host",
            RuntimeKind::Vite,
        ),
        (
            "MainThread",
            "node /repo/node_modules/.bin/vite",
            RuntimeKind::Vite,
        ),
        (
            "MainThread",
            "/usr/bin/node -e require('http').createServer()",
            RuntimeKind::Node,
        ),
        (
            "node",
            "node .output/server/index.mjs nuxt",
            RuntimeKind::Nuxt,
        ),
        ("bun", "bunx astro dev", RuntimeKind::Astro),
        ("node", "ng serve --port 4200", RuntimeKind::Angular),
        ("node", "storybook dev -p 6006", RuntimeKind::Storybook),
    ];

    for (name, command, expected) in cases {
        assert_eq!(classify_runtime(name, command), expected, "{command}");
    }
}

#[test]
fn discovers_tunnels_without_listening_ports() {
    let services = group_native_resources(
        HostPlatform::MacOs,
        Vec::new(),
        vec![NativeProcessRecord {
            pid: 9400,
            parent_pid: 8012,
            process_name: "cloudflared.exe".into(),
            executable: None,
            arguments: Vec::new(),
            command: "cloudflared.exe tunnel --url http://localhost:3000".into(),
            cwd: Some(r"C:\Tools\cloudflared".into()),
            started_at: Some(1720000000),
            start_token: "452110".into(),
        }],
    );

    assert_eq!(services.len(), 1);
    let tunnel = &services[0];
    assert_eq!(tunnel.platform, HostPlatform::MacOs);
    assert_eq!(tunnel.runtime, RuntimeKind::Cloudflared);
    assert_eq!(tunnel.resource_kind, ResourceKind::Tunnel);
    assert!(tunnel.ports.is_empty());
    assert!(tunnel.can_terminate);
    assert_eq!(
        tunnel.tunnel_target,
        Some(TunnelTarget {
            host: "localhost".into(),
            port: 3000,
        })
    );
}

#[test]
fn extracts_local_targets_from_supported_tunnel_commands() {
    let cases = [
        (
            RuntimeKind::Cloudflared,
            "cloudflared tunnel --url http://localhost:5000",
            "localhost",
            5000,
        ),
        (
            RuntimeKind::Cloudflared,
            "cloudflared tunnel --url=https://127.0.0.1:7443/api",
            "127.0.0.1",
            7443,
        ),
        (RuntimeKind::Ngrok, "ngrok http 5173", "localhost", 5173),
        (
            RuntimeKind::SshTunnel,
            "ssh -NT -R 0.0.0.0:33:127.0.0.1:22 host",
            "127.0.0.1",
            22,
        ),
        (
            RuntimeKind::LocalTunnel,
            "lt --port 6006",
            "localhost",
            6006,
        ),
        (
            RuntimeKind::Bore,
            "bore local 8080 --to bore.pub",
            "localhost",
            8080,
        ),
    ];

    for (runtime, command, host, port) in cases {
        assert_eq!(
            extract_tunnel_target(&runtime, command),
            Some(TunnelTarget {
                host: host.into(),
                port,
            }),
            "{command}"
        );
    }
}

#[test]
fn groups_a_tunnel_with_its_source_service_in_the_same_scope() {
    let listeners = vec![NativeListenerRecord {
        port: 5000,
        host: "127.0.0.1".into(),
        pid: 8100,
        parent_pid: 8000,
        process_name: "node.exe".into(),
        executable: None,
        arguments: Vec::new(),
        command: "node.exe server.js".into(),
        cwd: Some(r"C:\Projects\catalog-api".into()),
        started_at: Some(1720000000),
        start_token: "31000".into(),
    }];
    let tunnels = vec![NativeProcessRecord {
        pid: 8200,
        parent_pid: 8000,
        process_name: "cloudflared.exe".into(),
        executable: None,
        arguments: Vec::new(),
        command: "cloudflared.exe tunnel --url http://localhost:5000".into(),
        cwd: Some(r"C:\Projects\catalog-api".into()),
        started_at: Some(1720000000),
        start_token: "31010".into(),
    }];

    let groups = group_related_services(group_native_resources(
        HostPlatform::Windows,
        listeners,
        tunnels,
    ));

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].primary_port, Some(5000));
    assert_eq!(groups[0].services.len(), 2);
    assert_eq!(
        groups[0].services[0].resource_kind,
        ResourceKind::Development
    );
    assert_eq!(groups[0].services[1].resource_kind, ResourceKind::Tunnel);
}

#[test]
fn does_not_group_same_numbered_ports_across_execution_scopes() {
    let windows = group_native_resources(
        HostPlatform::Windows,
        vec![NativeListenerRecord {
            port: 5000,
            host: "127.0.0.1".into(),
            pid: 8100,
            parent_pid: 8000,
            process_name: "node.exe".into(),
            executable: None,
            arguments: Vec::new(),
            command: "node.exe server.js".into(),
            cwd: Some(r"C:\Projects\catalog-api".into()),
            started_at: Some(1720000000),
            start_token: "31000".into(),
        }],
        Vec::new(),
    );
    let linux = group_native_resources(
        HostPlatform::Linux,
        Vec::new(),
        vec![NativeProcessRecord {
            pid: 8300,
            parent_pid: 1,
            process_name: "cloudflared".into(),
            executable: None,
            arguments: Vec::new(),
            command: "cloudflared tunnel --url http://localhost:5000".into(),
            cwd: Some("/home/mocha/catalog-api".into()),
            started_at: Some(1720000000),
            start_token: "32000".into(),
        }],
    );

    let groups = group_related_services(windows.into_iter().chain(linux).collect());

    assert_eq!(groups.len(), 2);
    assert!(groups.iter().all(|group| group.services.len() == 1));
}

#[test]
fn recognizes_tunnel_clients_and_protects_sshd() {
    let cases = [
        (
            "cloudflared",
            "cloudflared tunnel --url http://127.0.0.1:3000",
            RuntimeKind::Cloudflared,
        ),
        ("ngrok", "ngrok http 5173", RuntimeKind::Ngrok),
        (
            "ssh",
            "ssh -NT -R 0.0.0.0:33:127.0.0.1:22 host",
            RuntimeKind::SshTunnel,
        ),
        (
            "autossh",
            "autossh -M 0 -R 8080:localhost:80 host",
            RuntimeKind::SshTunnel,
        ),
        ("frpc", "frpc -c /etc/frp/frpc.toml", RuntimeKind::Frp),
        ("sshd", "/usr/sbin/sshd -D", RuntimeKind::Sshd),
    ];

    for (name, command, expected) in cases {
        assert_eq!(classify_runtime(name, command), expected, "{command}");
    }

    let sshd = group_native_resources(
        HostPlatform::Linux,
        Vec::new(),
        vec![NativeProcessRecord {
            pid: 222,
            parent_pid: 1,
            process_name: "sshd".into(),
            executable: None,
            arguments: Vec::new(),
            command: "/usr/sbin/sshd -D".into(),
            cwd: Some("/".into()),
            started_at: Some(1720000000),
            start_token: "6000".into(),
        }],
    )
    .remove(0);
    assert_eq!(sshd.resource_kind, ResourceKind::System);
    assert!(!sshd.can_terminate);
}

#[test]
fn groups_linux_listeners_with_process_metadata() {
    let base = NativeListenerRecord {
        port: 3000,
        host: "0.0.0.0".into(),
        pid: 412,
        parent_pid: 1,
        process_name: "next-server".into(),
        executable: None,
        arguments: Vec::new(),
        command: "node node_modules/next/dist/bin/next dev".into(),
        cwd: Some("/home/mocha/workspaces/shop-ui".into()),
        started_at: Some(1720000000),
        start_token: "9876".into(),
    };
    let services = group_native_resources(
        HostPlatform::Linux,
        vec![
            base.clone(),
            NativeListenerRecord {
                port: 3001,
                host: "::1".into(),
                ..base
            },
        ],
        Vec::new(),
    );

    assert_eq!(services.len(), 1);
    let service = &services[0];
    assert_eq!(service.platform, HostPlatform::Linux);
    assert_eq!(service.pid, 412);
    assert_eq!(service.parent_pid, 1);
    assert_eq!(service.ports, vec![3000, 3001]);
    assert_eq!(service.hosts, vec!["0.0.0.0", "::1"]);
    assert_eq!(
        service.cwd.as_deref(),
        Some("/home/mocha/workspaces/shop-ui")
    );
    assert_eq!(service.project_name.as_deref(), Some("shop-ui"));
    assert_eq!(service.runtime, RuntimeKind::NextJs);
    assert_eq!(service.resource_kind, ResourceKind::Development);
    assert!(service.can_terminate);
    assert_eq!(service.start_token, "9876");
    assert_eq!(service.started_at, Some(1720000000));
    assert_eq!(service.display_name, "shop-ui");
}

#[test]
fn accepts_only_safe_systemd_service_names() {
    assert!(is_safe_service_unit("aliyunhost-reverse-tunnel.service"));
    assert!(is_safe_service_unit("openviking-ssh-tunnel@dev.service"));
    assert!(!is_safe_service_unit("../../sshd.service"));
    assert!(!is_safe_service_unit("tunnel.service; reboot"));
    assert!(!is_safe_service_unit("tunnel.socket"));
}

#[test]
fn rejects_a_reused_pid_before_terminating_it() {
    assert_eq!(
        ensure_process_identity("9876", None),
        Err(IdentityError::Missing)
    );
    assert_eq!(
        ensure_process_identity("9876", Some("10001")),
        Err(IdentityError::Changed)
    );
    assert_eq!(ensure_process_identity("9876", Some("9876")), Ok(()));
}

#[test]
fn derives_useful_names_instead_of_home_and_binary_directories() {
    let cases = [
        (
            HostPlatform::MacOs,
            "/Users/mocha",
            "cfuse",
            "/Users/mocha/.local/share/codefuse-cli/versions/v2.6.39/cfuse --cfuse-hub-daemon-runner",
            None,
        ),
        (
            HostPlatform::MacOs,
            "/Users/mocha/",
            "cfuse",
            "/Users/mocha/.local/bin/cfuse proxy --port 9792",
            None,
        ),
        (
            HostPlatform::Linux,
            "/home/mocha",
            "cfuse",
            "cfuse proxy --port 9792",
            None,
        ),
        (
            HostPlatform::Windows,
            r"C:\Users\mocha",
            "cfuse.exe",
            "cfuse.exe proxy --port 9792",
            None,
        ),
        (
            HostPlatform::MacOs,
            "/Users/mocha/.r2c/runtime/ai-coding-trace/bin",
            "node",
            "node server.js",
            Some("ai-coding-trace"),
        ),
        (
            HostPlatform::MacOs,
            "/Applications/波点音乐.app/Contents/MacOS",
            "波点音乐",
            "/Applications/波点音乐.app/Contents/MacOS/波点音乐",
            Some("波点音乐"),
        ),
        (
            HostPlatform::MacOs,
            "/Users/mocha/.local/bin",
            "cfuse",
            "cfuse proxy --port 9792",
            None,
        ),
        (
            HostPlatform::Linux,
            "/usr/local/bin",
            "daemon",
            "daemon",
            None,
        ),
        (HostPlatform::Linux, "/root", "daemon", "daemon", None),
        (
            HostPlatform::Linux,
            "/home/mocha/projects/shop",
            "node",
            "node server.js",
            Some("shop"),
        ),
    ];
    for (platform, cwd, name, command, expected) in cases {
        let services = group_native_resources(
            platform,
            vec![NativeListenerRecord {
                port: 9792,
                host: "127.0.0.1".into(),
                pid: 42,
                parent_pid: 1,
                process_name: name.into(),
                executable: None,
                arguments: Vec::new(),
                command: command.into(),
                cwd: Some(cwd.into()),
                started_at: Some(1720000000),
                start_token: "1720000000".into(),
            }],
            Vec::new(),
        );
        assert_eq!(services[0].project_name.as_deref(), expected, "cwd={cwd}");
        assert_eq!(services[0].process_name, name);
    }
}
