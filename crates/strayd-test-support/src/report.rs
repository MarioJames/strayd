use anyhow::Result;
use serde::Serialize;
use std::{fs, path::Path, time::Instant};

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Pass,
    Fail,
    Blocked,
    NotApplicable,
}

#[derive(Debug, Serialize)]
pub struct CaseResult {
    pub contract: Option<serde_json::Value>,
    pub id: String,
    pub source: String,
    pub status: Status,
    pub elapsed_ms: u128,
    pub reason: Option<String>,
    pub observations: Vec<serde_json::Value>,
}

#[derive(Serialize)]
pub struct Report {
    #[serde(skip)]
    pub specifications: Vec<serde_json::Value>,
    #[serde(skip)]
    pub selected: Option<String>,
    pub schema_version: u32,
    pub suite: String,
    pub os: String,
    pub arch: String,
    pub kernel: String,
    pub cases: Vec<CaseResult>,
    pub cleanup: String,
}

impl Report {
    pub fn new(suite: &str) -> Self {
        Self {
            specifications: Vec::new(),
            selected: None,
            schema_version: 1,
            suite: suite.into(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            kernel: sysinfo::System::kernel_version().unwrap_or_default(),
            cases: Vec::new(),
            cleanup: "pending".into(),
        }
    }
    pub fn case(
        &mut self,
        id: &str,
        source: &str,
        run: impl FnOnce(&mut Vec<serde_json::Value>) -> Result<()>,
    ) {
        if self
            .selected
            .as_deref()
            .is_some_and(|selected| selected != id)
        {
            return;
        }
        let start = Instant::now();
        let mut observations = Vec::new();
        let result = run(&mut observations);
        let reason = result.as_ref().err().map(|error| format!("{error:#}"));
        let status = if result.is_ok() {
            Status::Pass
        } else {
            Status::Fail
        };
        println!("{id}: {}", if result.is_ok() { "pass" } else { "fail" });
        self.cases.push(CaseResult {
            contract: self
                .specifications
                .iter()
                .find(|case| case["id"] == id)
                .cloned(),
            id: id.into(),
            source: source.into(),
            status,
            elapsed_ms: start.elapsed().as_millis(),
            reason,
            observations,
        });
    }
    pub fn unavailable(&mut self, id: &str, source: &str, applicable: bool, reason: &str) {
        if self
            .selected
            .as_deref()
            .is_some_and(|selected| selected != id)
        {
            return;
        }
        self.cases.push(CaseResult {
            contract: self
                .specifications
                .iter()
                .find(|case| case["id"] == id)
                .cloned(),
            id: id.into(),
            source: source.into(),
            status: if applicable {
                Status::Blocked
            } else {
                Status::NotApplicable
            },
            elapsed_ms: 0,
            reason: Some(reason.into()),
            observations: Vec::new(),
        });
    }
    pub fn success(&self) -> bool {
        self.cleanup == "pass"
            && !self.cases.is_empty()
            && self
                .cases
                .iter()
                .all(|case| matches!(case.status, Status::Pass | Status::NotApplicable))
    }
    pub fn write(&self, path: &Path) -> Result<()> {
        fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}
