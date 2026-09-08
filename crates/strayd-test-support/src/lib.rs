pub mod app_mock;
pub mod browser;
pub mod cases;
pub mod process;
pub mod pty;
pub mod replay;
pub mod report;
pub mod runtimes;
pub mod special;
pub mod tunnel;

use anyhow::{Result, bail};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn until(timeout: Duration, mut predicate: impl FnMut() -> Result<bool>) -> Result<()> {
    let start = Instant::now();
    loop {
        if predicate()? {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            bail!("condition did not become true within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}
