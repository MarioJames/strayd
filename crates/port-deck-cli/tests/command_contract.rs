use clap::Parser;
use port_deck_cli::{
    Cli, Command, ConfigAction, DisplayConfig, Filters, HideRule, PlanError, PlatformFilter,
    StopTarget, StraydConfig, apply_visibility_config, build_stop_plan, initialize_config,
    load_config, parse_config,
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

#[test]
fn visibility_rules_hide_only_matching_services_and_keep_related_tunnels() {
    let mut source = service(8100, ResourceKind::System, RuntimeKind::Sshd, 22);
    source.can_terminate = false;
    source.process_name = "sshd".into();
    let mut tunnel = service(8200, ResourceKind::Tunnel, RuntimeKind::Cloudflared, 0);
    tunnel.ports.clear();
    tunnel.tunnel_target = Some(TunnelTarget {
        host: "localhost".into(),
        port: 22,
    });
    let groups = vec![ResourceGroup {
        id: "group:ssh".into(),
        primary_port: Some(22),
        services: vec![source, tunnel],
    }];
    let config = StraydConfig {
        display: DisplayConfig {
            hide: vec![HideRule {
                ports: vec![22],
                runtimes: vec!["sshd".into()],
                ..HideRule::default()
            }],
        },
        ..StraydConfig::default()
    };

    let visible = apply_visibility_config(&groups, &config);

    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].primary_port, Some(22));
    assert_eq!(visible[0].services.len(), 1);
    assert_eq!(visible[0].services[0].pid, 8200);
}

#[test]
fn visibility_rule_fields_are_conjunctive_and_empty_rules_hide_nothing() {
    let groups = vec![ResourceGroup {
        id: "group:22".into(),
        primary_port: Some(22),
        services: vec![
            service(8100, ResourceKind::System, RuntimeKind::Sshd, 22),
            service(8300, ResourceKind::Development, RuntimeKind::Node, 22),
        ],
    }];
    let config = StraydConfig {
        display: DisplayConfig {
            hide: vec![
                HideRule::default(),
                HideRule {
                    ports: vec![22],
                    runtimes: vec!["sshd".into()],
                    ..HideRule::default()
                },
            ],
        },
        ..StraydConfig::default()
    };

    let visible = apply_visibility_config(&groups, &config);

    assert_eq!(visible[0].services.len(), 1);
    assert_eq!(visible[0].services[0].runtime, RuntimeKind::Node);
}

#[test]
fn parses_a_versioned_toml_visibility_configuration() {
    let config = parse_config(
        r#"
version = 1

[[display.hide]]
ports = [22, 5432]
runtimes = ["sshd"]
platforms = ["linux"]
process_names = ["openssh"]
projects = ["legacy-api"]
commands = ["--internal-only"]
ids = ["linux:42:100"]
kinds = ["system"]
"#,
    )
    .expect("valid config should parse");

    let rule = &config.display.hide[0];
    assert_eq!(config.version, 1);
    assert_eq!(rule.ports, [22, 5432]);
    assert_eq!(rule.runtimes, ["sshd"]);
    assert_eq!(rule.process_names, ["openssh"]);
    assert_eq!(rule.projects, ["legacy-api"]);
    assert_eq!(rule.commands, ["--internal-only"]);
    assert_eq!(rule.ids, ["linux:42:100"]);
    assert_eq!(rule.kinds.len(), 1);
    assert_eq!(rule.platforms.len(), 1);
}

#[test]
fn rejects_unknown_config_versions() {
    let error = parse_config("version = 99").expect_err("unknown versions must fail closed");

    assert!(error.to_string().contains("99"));
}

#[test]
fn initializes_and_loads_a_persistent_config_without_overwriting_by_default() {
    let directory = std::env::temp_dir().join(format!(
        "strayd-config-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be available")
            .as_nanos()
    ));
    let path = directory.join("nested/config.toml");

    initialize_config(&path, false).expect("config should be initialized");
    assert_eq!(
        load_config(&path).expect("initialized config should load"),
        StraydConfig::default()
    );
    assert!(initialize_config(&path, false).is_err());

    std::fs::remove_dir_all(&directory).expect("temporary config should be removed");
}

#[test]
fn parses_config_commands_and_global_config_overrides() {
    let cli = Cli::try_parse_from([
        "strayd",
        "--config",
        "/tmp/strayd.toml",
        "config",
        "init",
        "--force",
    ])
    .expect("config command should parse");

    assert_eq!(
        cli.config.as_deref(),
        Some(std::path::Path::new("/tmp/strayd.toml"))
    );
    assert!(matches!(
        cli.command,
        Some(Command::Config(args)) if matches!(args.action, ConfigAction::Init { force: true })
    ));
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
