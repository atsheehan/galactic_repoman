//! Logging setup. Every record carries a `run=<id>` field so that runs appended to
//! the same file (Steam captures stdout/stderr per launch) stay distinguishable.

use std::io::Write;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

/// Identifies this process: nanoseconds since the epoch, plus the pid to separate
/// runs that start within the same clock tick.
fn run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    format!("{:x}-{:x}", nanos, process::id())
}

/// Installs the global logger, respecting `RUST_LOG`. The run id is written ahead of
/// any key-values from the call site, so it trails the message on every line.
pub fn init() {
    let run = run_id();

    env_logger::Builder::from_default_env()
        .format_key_values(move |buf, kvs| {
            write!(buf, " run={run}")?;
            env_logger::fmt::default_kv_format(buf, kvs)
        })
        .init();

    log::info!("run started");
}
