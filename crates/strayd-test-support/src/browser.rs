use crate::{cases::Environment, report::Report};
use serde_json::json;
use std::{fs, io::BufRead};

/// Supplies owned real processes and an isolated config to the browser's PTY bridge.
/// EOF closes the fixtures. PID registration lets outer cleanup recover a killed bridge.
pub fn run(env: &Environment, report: &mut Report) {
    report.case("browser-fixture-lifecycle", "native-fixture", |observed| {
        let argument = "long-browser-fixture-argument-".repeat(24);
        let mut first = env.fixture(
            "browser-one",
            "browser-one",
            None,
            &[&argument, "--fixture-lifetime-ms", "300000"],
        )?;
        let mut second = env.fixture(
            "browser-two",
            "browser-two",
            None,
            &[&argument, "--fixture-lifetime-ms", "300000"],
        )?;
        let own = [first.child.id(), second.child.id()];
        let snapshot = port_deck_engine::scan_all();
        let hidden = snapshot
            .groups
            .iter()
            .flat_map(|g| &g.services)
            .filter(|s| !own.contains(&s.pid))
            .map(|s| &s.id)
            .collect::<Vec<_>>();
        let mut services = snapshot
            .groups
            .iter()
            .flat_map(|g| &g.services)
            .filter(|s| own.contains(&s.pid))
            .collect::<Vec<_>>();
        services.sort_by_key(|s| s.ports[0]);
        let config = env.root.join("browser.toml");
        fs::write(
            &config,
            if hidden.is_empty() {
                "version = 1\n".into()
            } else {
                format!(
                    "version = 1\n[[display.hide]]\nids = {}\n",
                    serde_json::to_string(&hidden)?
                )
            },
        )?;
        fs::write(
            env.output.join("browser.json"),
            serde_json::to_vec(&json!({"config":config,"services":services,"cli":env.cli}))?,
        )?;
        for line in std::io::stdin().lock().lines() {
            let line = line?;
            if line == "quit" {
                break;
            }
            let value: serde_json::Value = serde_json::from_str(&line)?;
            if let Some(pid) = value["register_pid"].as_u64() {
                env.registry.register(u32::try_from(pid)?)?;
            }
        }
        first.cleanup()?;
        second.cleanup()?;
        observed.push(json!({"fixture_pids":own,"released":true}));
        Ok(())
    });
}
