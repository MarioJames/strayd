use clap::Parser;
use port_deck_cli::{
    Cli, Command, Filters, PlanError, PlatformFilter, StopTarget, build_stop_plan,
};
use port_deck_core::{
    HostPlatform, ResourceGroup, ResourceKind, RuntimeKind, ServiceProcess, TunnelTarget,
};

#[test]
fn parses_tunnel_stop_with_host_platform_filter_and_safety_flags() {
    let cli = Cli::try_parse_from([
        "strayd",
        "stop",
        "tunnel",
        "--port",
        "5000",
        "--platform",
        "linux",
        "--all",
        "--yes",
    ])
    .expect("command should parse");

    let Some(Command::Stop(args)) = cli.command else {
        panic!("expected stop command");
    };
    assert_eq!(args.target, StopTarget::Tunnel);
    assert_eq!(args.filters.port, Some(5000));
    assert_eq!(args.filters.platform, Some(PlatformFilter::Linux));
    assert!(args.all);
    assert!(args.yes);
}

#[test]
fn tunnel_stop_matches_the_tunnels_origin_port() {
    let groups = vec![linked_group(false)];

    let plan = build_stop_plan(
        &groups,
        StopTarget::Tunnel,
        &Filters {
            port: Some(5000),
            ..Filters::default()
        },
        false,
    )
    .expect("one tunnel should match");

    assert_eq!(
        plan.iter().map(|service| service.pid).collect::<Vec<_>>(),
        [8200]
    );
}

#[test]
fn whole_group_stops_tunnels_before_their_source_service() {
    let groups = vec![linked_group(false)];

    let plan = build_stop_plan(
        &groups,
        StopTarget::Group,
        &Filters {
            port: Some(5000),
            ..Filters::default()
        },
        false,
    )
    .expect("one safe group should match");

    assert_eq!(
        plan.iter().map(|service| service.pid).collect::<Vec<_>>(),
        [8200, 8100]
    );
}

#[test]
fn whole_group_rejects_a_protected_service() {
    let groups = vec![linked_group(true)];

    let error = build_stop_plan(
        &groups,
        StopTarget::Group,
        &Filters {
            port: Some(5000),
            ..Filters::default()
        },
        false,
    )
    .expect_err("protected groups must never be partially stopped");

    assert_eq!(error, PlanError::ProtectedGroup("group:5000".into()));
}

#[test]
fn bulk_resource_matches_require_an_explicit_all_flag() {
    let groups = vec![linked_group(false), development_group(5173, 8300)];

    let error = build_stop_plan(&groups, StopTarget::Dev, &Filters::default(), false)
        .expect_err("more than one match needs --all");

    assert_eq!(error, PlanError::MultipleMatches(2));
}

fn linked_group(protected: bool) -> ResourceGroup {
    let mut source = service(8100, ResourceKind::Development, RuntimeKind::NextJs, 5000);
    if protected {
        source.runtime = RuntimeKind::Sshd;
        source.resource_kind = ResourceKind::System;
        source.can_terminate = false;
        source.process_name = "sshd".into();
    }
    let mut tunnel = service(8200, ResourceKind::Tunnel, RuntimeKind::Cloudflared, 0);
    tunnel.ports.clear();
    tunnel.tunnel_target = Some(TunnelTarget {
        host: "localhost".into(),
        port: 5000,
    });
    ResourceGroup {
        id: "group:5000".into(),
        primary_port: Some(5000),
        services: vec![source, tunnel],
    }
}

fn development_group(port: u16, pid: u32) -> ResourceGroup {
    ResourceGroup {
        id: format!("group:{port}"),
        primary_port: Some(port),
        services: vec![service(
            pid,
            ResourceKind::Development,
            RuntimeKind::Vite,
            port,
        )],
    }
}

fn service(
    pid: u32,
    resource_kind: ResourceKind,
    runtime: RuntimeKind,
    port: u16,
) -> ServiceProcess {
    ServiceProcess {
        id: format!("linux:{pid}:100"),
        platform: HostPlatform::Linux,
        pid,
        parent_pid: 1,
        ports: vec![port],
        hosts: vec!["127.0.0.1".into()],
        process_name: "node".into(),
        command: "node server.js".into(),
        cwd: Some("/workspaces/shop".into()),
        project_name: Some("shop".into()),
        runtime,
        resource_kind,
        can_terminate: true,
        manager_unit: None,
        tunnel_target: None,
        start_token: "100".into(),
    }
}
