use anyhow::{Context, Result};
use clap::Parser;
use std::{fs, path::PathBuf, process::ExitCode};
use strayd_test_support::{
    cases::{self, Environment},
    process::Registry,
    pty, replay,
    report::Report,
    runtimes, special,
};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "smoke")]
    suite: String,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    scratch_root: Option<PathBuf>,
    #[arg(long)]
    fixture: Option<PathBuf>,
    #[arg(long)]
    strayd: Option<PathBuf>,
    #[arg(long)]
    node_wrapper: Option<PathBuf>,
    #[arg(long)]
    repository: PathBuf,
    #[arg(long)]
    cleanup: bool,
    #[arg(long)]
    scenario: Option<String>,
    #[arg(long, hide = true)]
    permission_probe: Option<PathBuf>,
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<bool> {
    if let Some(path) = args.permission_probe {
        return special::permission_probe(&path).map(|()| true);
    }
    fs::create_dir_all(&args.output)?;
    let ledger = args.output.join("resources.json");
    if args.cleanup {
        special::cleanup_units(&args.output.join("units.json"))?;
        if ledger.exists() {
            Registry::clean_file(&ledger)?;
        }
        return Ok(true);
    }
    anyhow::ensure!(
        !ledger.exists(),
        "output already contains an ownership ledger; choose a fresh run directory"
    );
    let registry = Registry::new(ledger)?;
    let scratch = tempfile::Builder::new()
        .prefix("scratch-")
        .tempdir_in(args.scratch_root.as_ref().unwrap_or(&args.output))?;
    let installed_wrapper = args.node_wrapper.is_some();
    let cli = if let Some(wrapper) = args.node_wrapper {
        vec!["node".into(), wrapper.to_string_lossy().into_owned()]
    } else {
        vec![
            args.strayd
                .context("--strayd is required")?
                .to_string_lossy()
                .into_owned(),
        ]
    };
    let env = Environment {
        output: args.output.clone(),
        fixture: args.fixture.context("--fixture is required")?,
        cli,
        root: scratch.path().to_path_buf(),
        repository: args.repository,
        registry: registry.clone(),
    };
    let mut report = Report::new(&args.suite);
    let catalog: serde_json::Value = serde_json::from_slice(&fs::read(
        env.repository.join("tests/scenarios/catalog.json"),
    )?)?;
    report.specifications = catalog["cases"]
        .as_array()
        .context("scenario catalog cases")?
        .clone();
    report.selected = args.scenario;
    match args.suite.as_str() {
        "smoke" => {
            cases::native(&env, &mut report, false);
            cases::cli(&env, &mut report);
        }
        "native" => {
            cases::native(&env, &mut report, true);
            cases::cli(&env, &mut report);
            runtimes::macos_app(&env, &mut report);
        }
        "runtime-smoke" => runtimes::run(&env, &mut report, false),
        "runtimes" => runtimes::run(&env, &mut report, true),
        "cli" => cases::cli(&env, &mut report),
        "package" if installed_wrapper => cases::cli(&env, &mut report),
        "package" => report.unavailable(
            "package-installed-wrapper",
            "native-fixture",
            true,
            "use the unified runner with --artifact to install and verify the actual package",
        ),
        "lifecycle" => cases::lifecycle(&env, &mut report),
        "faults" => cases::faults(&env, &mut report),
        "tui" => pty::run(&env, &mut report),
        "runner-interrupt" => pty::runner_interrupt(&env, &mut report),
        "browser-setup" => strayd_test_support::browser::run(&env, &mut report),
        "stability" => special::stability(&env, &mut report),
        "systemd" => special::systemd(&env, &mut report),
        "permissions" => special::permissions(&env, &mut report),
        "desktop" => special::desktop(&env, &mut report),
        "app-mock" => strayd_test_support::app_mock::run(&env, &mut report),
        "wsl" => special::wsl(&env, &mut report),
        "tunnel" => strayd_test_support::tunnel::run(&env, &mut report),
        "replay" => replay::run(&env.repository.join("tests/recordings"), &mut report)?,
        other => anyhow::bail!("unknown suite {other}"),
    }
    if report.cases.is_empty() {
        let id = report
            .selected
            .clone()
            .unwrap_or_else(|| "empty-suite".into());
        report.case(&id, "synthetic", |_| {
            anyhow::bail!("No matching scenario in the selected suite")
        });
    }
    if report
        .cases
        .iter()
        .any(|case| matches!(case.status, strayd_test_support::report::Status::Fail))
    {
        preserve_logs(scratch.path(), &args.output.join("diagnostics"), 0)?;
    }
    let cleanup = registry
        .cleanup()
        .and_then(|()| scratch.close().context("remove scratch directory"));
    report.cleanup = match cleanup {
        Ok(()) => "pass".into(),
        Err(error) => format!("fail: {error:#}"),
    };
    report.write(&args.output.join("report.json"))?;
    Ok(report.success())
}

fn preserve_logs(
    source: &std::path::Path,
    destination: &std::path::Path,
    depth: usize,
) -> Result<()> {
    if depth > 6 {
        return Ok(());
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            preserve_logs(
                &entry.path(),
                &destination.join(entry.file_name()),
                depth + 1,
            )?;
        } else if entry.file_type()?.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "log")
        {
            use std::io::Read;
            let mut contents = Vec::new();
            fs::File::open(entry.path())?
                .take(16384)
                .read_to_end(&mut contents)?;
            if !contents.is_empty() {
                fs::create_dir_all(destination)?;
                fs::write(destination.join(entry.file_name()), contents)?;
            }
        }
    }
    Ok(())
}
