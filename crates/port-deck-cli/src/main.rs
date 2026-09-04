mod tui;

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use clap::Parser;
use port_deck_cli::{
    Cli, Command, ConfigAction, ConfigArgs, Language, LanguageSetting, ListArgs, StopArgs,
    StraydConfig, Translator, TuiArgs, apply_visibility_config, build_stop_plan, filter_groups,
    format_config, initialize_config, load_config, resolve_config_path, resolve_system_language,
    runtime_slug,
};
use port_deck_core::{HostPlatform, ResourceGroup, ResourceKind, ServiceProcess};
use port_deck_engine::{ScanSnapshot, scan_all, terminate_service};
use serde::Serialize;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let fallback_language = resolve_system_language(cli.language, LanguageSetting::Auto);
    match run(cli, fallback_language) {
        Ok(()) => ExitCode::SUCCESS,
        Err((language, error)) => {
            let tr = Translator::new(language);
            eprintln!("{}: {error}", tr.text("error_prefix"));
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli, fallback_language: Language) -> Result<(), (Language, String)> {
    let fallback_tr = Translator::new(fallback_language);
    let config_path = resolve_config_path(cli.config.as_deref())
        .map_err(|error| (fallback_language, fallback_tr.config_error(&error)))?;
    if let Some(Command::Config(args)) = &cli.command {
        return config(args.clone(), &config_path, fallback_tr)
            .map_err(|error| (fallback_language, error));
    }
    let config = if cli.no_config {
        StraydConfig::default()
    } else {
        load_config(&config_path)
            .map_err(|error| (fallback_language, fallback_tr.config_error(&error)))?
    };
    let language = resolve_system_language(cli.language, config.language);
    let tr = Translator::new(language);
    let editable_config_path = (!cli.no_config).then_some(config_path);

    let result = match cli.command {
        None => tui::run(TuiArgs::default(), config, editable_config_path, language),
        Some(Command::Tui(args)) => tui::run(args, config, editable_config_path, language),
        Some(Command::List(args)) => list(args, &config, tr),
        Some(Command::Stop(args)) => stop(args, &config, tr),
        Some(Command::Config(_)) => unreachable!(),
    };
    result.map_err(|error| (language, error))
}

fn config(args: ConfigArgs, path: &std::path::Path, tr: Translator) -> Result<(), String> {
    match args.action {
        ConfigAction::Path => println!("{}", path.display()),
        ConfigAction::Init { force } => {
            initialize_config(path, force).map_err(|error| tr.config_error(&error))?;
            println!(
                "{}",
                tr.format("config_created", &[("path", path.display().to_string())])
            );
        }
        ConfigAction::Show => {
            let config = load_config(path).map_err(|error| tr.config_error(&error))?;
            print!(
                "{}",
                format_config(&config).map_err(|error| tr.config_error(&error))?
            );
        }
    }
    Ok(())
}

fn list(args: ListArgs, config: &StraydConfig, tr: Translator) -> Result<(), String> {
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
                .map_err(|error| tr.format("json_error", &[("error", error.to_string())]))?
        );
    } else {
        print_groups(&groups, tr);
        print_warnings(&snapshot, tr);
    }
    Ok(())
}

fn stop(args: StopArgs, config: &StraydConfig, tr: Translator) -> Result<(), String> {
    let snapshot = scan_all();
    print_warnings(&snapshot, tr);
    let visible = apply_visibility_config(&snapshot.groups, config);
    let plan = build_stop_plan(&visible, args.target, &args.filters, args.all)
        .map_err(|error| tr.plan_error(&error))?;

    println!(
        "{}",
        tr.format("stop_plan", &[("count", plan.len().to_string())])
    );
    for service in &plan {
        println!("  {}", describe_service(service));
    }

    if args.dry_run {
        println!("{}", tr.text("dry_run"));
        return Ok(());
    }
    if !args.yes {
        confirm_stop(plan.len(), tr)?;
    }

    let mut stopped = 0;
    let mut errors = Vec::new();
    for service in &plan {
        match terminate_service(service) {
            Ok(()) => stopped += 1,
            Err(error) => errors.push(format!(
                "{}: {}",
                describe_service(service),
                tr.engine_error(&error)
            )),
        }
    }

    println!("{}", tr.stopped(stopped));
    if errors.is_empty() {
        Ok(())
    } else {
        Err(tr.format(
            "stop_failed",
            &[
                ("count", errors.len().to_string()),
                ("details", errors.join("\n")),
            ],
        ))
    }
}

fn confirm_stop(count: usize, tr: Translator) -> Result<(), String> {
    if !io::stdin().is_terminal() {
        return Err(tr.text("noninteractive_yes").into());
    }
    print!(
        "{}",
        tr.format("confirm_stop", &[("count", count.to_string())])
    );
    io::stdout()
        .flush()
        .map_err(|error| tr.format("terminal_write_error", &[("error", error.to_string())]))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| tr.format("terminal_read_error", &[("error", error.to_string())]))?;
    if answer.trim().eq_ignore_ascii_case("yes") {
        Ok(())
    } else {
        Err(tr.text("cancelled").into())
    }
}

fn print_groups(groups: &[ResourceGroup], tr: Translator) {
    if groups.is_empty() {
        println!("{}", tr.no_matching_resources());
        return;
    }
    for group in groups {
        let port = group
            .primary_port
            .map(|port| format!(":{port}"))
            .unwrap_or_else(|| "-".into());
        println!(
            "{port:<7} {}  {}",
            group_label(group),
            tr.format(
                "group_items",
                &[("count", group.services.len().to_string())]
            )
        );
        for service in &group.services {
            println!("          {}", describe_service(service));
        }
    }
}

fn print_warnings(snapshot: &ScanSnapshot, tr: Translator) {
    for warning in &snapshot.warnings {
        eprintln!("{}: {warning}", tr.text("warning_prefix"));
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
