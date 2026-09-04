mod tui;

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use clap::Parser;
use port_deck_cli::{
    Cli, Command, ConfigAction, ConfigArgs, ListArgs, StopArgs, StraydConfig, TuiArgs,
    apply_visibility_config, build_stop_plan, filter_groups, format_config, initialize_config,
    load_config, resolve_config_path, runtime_slug,
};
use port_deck_core::{HostPlatform, ResourceGroup, ResourceKind, ServiceProcess};
use port_deck_engine::{ScanSnapshot, scan_all, terminate_service};
use serde::Serialize;

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("错误: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let config_path =
        resolve_config_path(cli.config.as_deref()).map_err(|error| error.to_string())?;
    if let Some(Command::Config(args)) = &cli.command {
        return config(args.clone(), &config_path);
    }
    let config = if cli.no_config {
        StraydConfig::default()
    } else {
        load_config(&config_path).map_err(|error| error.to_string())?
    };

    match cli.command {
        None => tui::run(TuiArgs::default(), config),
        Some(Command::Tui(args)) => tui::run(args, config),
        Some(Command::List(args)) => list(args, &config),
        Some(Command::Stop(args)) => stop(args, &config),
        Some(Command::Config(_)) => unreachable!(),
    }
}

fn config(args: ConfigArgs, path: &std::path::Path) -> Result<(), String> {
    match args.action {
        ConfigAction::Path => println!("{}", path.display()),
        ConfigAction::Init { force } => {
            initialize_config(path, force).map_err(|error| error.to_string())?;
            println!("已创建配置：{}", path.display());
        }
        ConfigAction::Show => {
            let config = load_config(path).map_err(|error| error.to_string())?;
            print!(
                "{}",
                format_config(&config).map_err(|error| error.to_string())?
            );
        }
    }
    Ok(())
}

fn list(args: ListArgs, config: &StraydConfig) -> Result<(), String> {
    let snapshot = scan_all();
    let visible = apply_visibility_config(&snapshot.groups, config);
    let groups = filter_groups(&visible, &args.filters, args.kind);
    if args.json {
        let output = ListOutput {
            groups: &groups,
            warnings: &snapshot.warnings,
            scanned_at: snapshot.scanned_at,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&output)
                .map_err(|error| format!("无法生成 JSON: {error}"))?
        );
    } else {
        print_groups(&groups);
        print_warnings(&snapshot);
    }
    Ok(())
}

fn stop(args: StopArgs, config: &StraydConfig) -> Result<(), String> {
    let snapshot = scan_all();
    print_warnings(&snapshot);
    let visible = apply_visibility_config(&snapshot.groups, config);
    let plan = build_stop_plan(&visible, args.target, &args.filters, args.all)
        .map_err(|error| error.to_string())?;

    println!("将停止 {} 项：", plan.len());
    for service in &plan {
        println!("  {}", describe_service(service));
    }

    if args.dry_run {
        println!("dry-run：未结束任何进程");
        return Ok(());
    }
    if !args.yes {
        confirm_stop(plan.len())?;
    }

    let mut stopped = 0;
    let mut errors = Vec::new();
    for service in &plan {
        match terminate_service(service) {
            Ok(()) => stopped += 1,
            Err(error) => errors.push(format!("{}: {error}", describe_service(service))),
        }
    }

    println!("已停止 {stopped} 项");
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} 项停止失败\n{}",
            errors.len(),
            errors.join("\n")
        ))
    }
}

fn confirm_stop(count: usize) -> Result<(), String> {
    if !io::stdin().is_terminal() {
        return Err("非交互终端必须传入 --yes，或先使用 --dry-run 查看计划".into());
    }
    print!("确认停止 {count} 项？输入 yes 继续: ");
    io::stdout()
        .flush()
        .map_err(|error| format!("无法写入终端: {error}"))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| format!("无法读取确认: {error}"))?;
    if answer.trim().eq_ignore_ascii_case("yes") {
        Ok(())
    } else {
        Err("操作已取消".into())
    }
}

fn print_groups(groups: &[ResourceGroup]) {
    if groups.is_empty() {
        println!("没有匹配的资源");
        return;
    }
    for group in groups {
        let port = group
            .primary_port
            .map(|port| format!(":{port}"))
            .unwrap_or_else(|| "-".into());
        println!(
            "{port:<7} {}  {} 项",
            group_label(group),
            group.services.len()
        );
        for service in &group.services {
            println!("          {}", describe_service(service));
        }
    }
}

fn print_warnings(snapshot: &ScanSnapshot) {
    for warning in &snapshot.warnings {
        eprintln!("警告: {warning}");
    }
}

fn group_label(group: &ResourceGroup) -> String {
    group
        .services
        .iter()
        .find(|service| service.resource_kind != ResourceKind::Tunnel)
        .or_else(|| group.services.first())
        .map(|service| {
            service
                .project_name
                .clone()
                .unwrap_or_else(|| service.process_name.clone())
        })
        .unwrap_or_else(|| group.id.clone())
}

fn describe_service(service: &ServiceProcess) -> String {
    let platform = match service.platform {
        HostPlatform::Windows => "Windows",
        HostPlatform::Linux => "Linux",
        HostPlatform::MacOs => "macOS",
    };
    let target = service
        .tunnel_target
        .as_ref()
        .map(|target| format!(" -> {}:{}", target.host, target.port))
        .unwrap_or_default();
    format!(
        "{} pid={} {} {}{}",
        runtime_slug(&service.runtime),
        service.pid,
        platform,
        service.id,
        target
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListOutput<'a> {
    groups: &'a [ResourceGroup],
    warnings: &'a [String],
    scanned_at: u128,
}
