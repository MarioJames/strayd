use port_deck_core::{
    IdentityError, NativeListenerRecord, NativeProcessRecord, ProcessOrigin, ResourceKind,
    RuntimeKind, ServiceProcess, TunnelTarget, classify_runtime, decode_command_output,
    ensure_process_identity, extract_tunnel_target, group_native_resources, group_related_services,
    is_application_dev_service, is_safe_service_unit, parse_wsl_snapshot,
};

#[test]
fn decodes_utf16le_output_emitted_by_wsl_exe() {
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
        command: r#""C:\Program Files\nodejs\node.exe" C:\Projects\app-shell\node_modules\vite\bin\vite.js"#.into(),
        cwd: Some(r"C:\Projects\app-shell".into()),
        start_token: "13432622".into(),
    };
    let services = group_native_resources(
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
    assert_eq!(service.origin, ProcessOrigin::Windows);
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
        Vec::new(),
        vec![NativeProcessRecord {
            pid: 9400,
            parent_pid: 8012,
            process_name: "cloudflared.exe".into(),
            command: "cloudflared.exe tunnel --url http://localhost:3000".into(),
            cwd: Some(r"C:\Tools\cloudflared".into()),
            start_token: "452110".into(),
        }],
    );

    assert_eq!(services.len(), 1);
    let tunnel = &services[0];
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
        command: "node.exe server.js".into(),
        cwd: Some(r"C:\Projects\catalog-api".into()),
        start_token: "31000".into(),
    }];
    let tunnels = vec![NativeProcessRecord {
        pid: 8200,
        parent_pid: 8000,
        process_name: "cloudflared.exe".into(),
        command: "cloudflared.exe tunnel --url http://localhost:5000".into(),
        cwd: Some(r"C:\Projects\catalog-api".into()),
        start_token: "31010".into(),
    }];

    let groups = group_related_services(group_native_resources(listeners, tunnels));

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
    let native = group_native_resources(
        vec![NativeListenerRecord {
            port: 5000,
            host: "127.0.0.1".into(),
            pid: 8100,
            parent_pid: 8000,
            process_name: "node.exe".into(),
            command: "node.exe server.js".into(),
            cwd: Some(r"C:\Projects\catalog-api".into()),
            start_token: "31000".into(),
        }],
        Vec::new(),
    );
    let wsl = parse_wsl_snapshot(
        "Debian",
        concat!(
            "PORTDECK/2\x1e",
            "P\x1f8300\x1f8300 (cloudflared) S 1 8300 8300 0 -1 4194560 10 0 0 0 1 0 0 0 20 0 1 0 32000\x1f/home/mocha/catalog-api\x1f\x1fcloudflared tunnel --url http://localhost:5000\x1e",
        ),
    );

    let groups = group_related_services(native.into_iter().chain(wsl).collect());

    assert_eq!(groups.len(), 2);
    assert!(groups.iter().all(|group| group.services.len() == 1));
}

#[test]
fn identifies_only_the_application_own_dev_service() {
    let port_deck = group_native_resources(
        vec![NativeListenerRecord {
            port: 1420,
            host: "127.0.0.1".into(),
            pid: 8800,
            parent_pid: 8700,
            process_name: "node.exe".into(),
            command: r#"node.exe node_modules\vite\bin\vite.js"#.into(),
            cwd: Some(r"C:\Projects\port-deck".into()),
            start_token: "12500".into(),
        }],
        Vec::new(),
    )
    .remove(0);
    let another_project = ServiceProcess {
        project_name: Some("customer-portal".into()),
        ..port_deck.clone()
    };
    let another_port = ServiceProcess {
        ports: vec![5173],
        ..port_deck.clone()
    };

    assert!(is_application_dev_service(&port_deck, "port-deck", 1420));
    assert!(!is_application_dev_service(
        &another_project,
        "port-deck",
        1420
    ));
    assert!(!is_application_dev_service(
        &another_port,
        "port-deck",
        1420
    ));
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

    let snapshot = concat!(
        "PORTDECK/2\x1e",
        "S\x1fLISTEN 0 128 0.0.0.0:22 0.0.0.0:* users:((\"sshd\",pid=222,fd=3))\x1e",
        "P\x1f222\x1f222 (sshd) S 1 222 222 0 -1 4194560 10 0 0 0 1 0 0 0 20 0 1 0 6000\x1f/\x1fssh.service\x1f/usr/sbin/sshd -D\x1e",
    );
    let sshd = parse_wsl_snapshot("Debian", snapshot).remove(0);
    assert_eq!(sshd.resource_kind, ResourceKind::System);
    assert!(!sshd.can_terminate);
    assert_eq!(sshd.manager_unit.as_deref(), Some("ssh.service"));
}

#[test]
fn joins_wsl_listeners_with_process_metadata_and_groups_ports() {
    let snapshot = concat!(
        "PORTDECK/2\x1e",
        "S\x1fLISTEN 0 511 0.0.0.0:3000 0.0.0.0:* users:((\"next-server\",pid=412,fd=21))\x1e",
        "S\x1fLISTEN 0 511 [::]:3000 [::]:* users:((\"next-server\",pid=412,fd=23))\x1e",
        "S\x1fLISTEN 0 511 [::1]:3001 [::]:* users:((\"next-server\",pid=412,fd=22))\x1e",
        "S\x1fLISTEN 0 128 127.0.0.1:5432 0.0.0.0:*\x1e",
        "P\x1f412\x1f412 (next-server) S 1 412 412 0 -1 4194560 10 0 0 0 1 0 0 0 20 0 1 0 9876\x1f/home/mocha/workspaces/shop-ui\x1f\x1fnode node_modules/next/dist/bin/next dev\x1e",
    );

    let services = parse_wsl_snapshot("Debian", snapshot);

    assert_eq!(services.len(), 1);
    let service = &services[0];
    assert_eq!(service.origin, ProcessOrigin::Wsl);
    assert_eq!(service.distribution.as_deref(), Some("Debian"));
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
}

#[test]
fn wsl_snapshot_keeps_unbound_tunnels_and_their_systemd_unit() {
    let snapshot = concat!(
        "PORTDECK/2\x1e",
        "P\x1f730\x1f730 (ssh) S 1 730 730 0 -1 4194560 10 0 0 0 1 0 0 0 20 0 1 0 44000\x1f/root\x1faliyunhost-reverse-tunnel.service\x1f/usr/bin/ssh -NT -R 0.0.0.0:33:127.0.0.1:22 aliyun\x1e",
        "P\x1f731\x1f731 (sleep) S 1 731 731 0 -1 4194560 10 0 0 0 1 0 0 0 20 0 1 0 44001\x1f/tmp\x1f\x1fsleep 30\x1e",
    );

    let services = parse_wsl_snapshot("Debian", snapshot);

    assert_eq!(services.len(), 1);
    let tunnel = &services[0];
    assert_eq!(tunnel.runtime, RuntimeKind::SshTunnel);
    assert_eq!(tunnel.resource_kind, ResourceKind::Tunnel);
    assert!(tunnel.ports.is_empty());
    assert_eq!(
        tunnel.manager_unit.as_deref(),
        Some("aliyunhost-reverse-tunnel.service")
    );
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
