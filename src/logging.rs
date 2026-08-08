//! Logging setup. Every record carries a `run=<id>` field so that runs appended to
//! the same file (Steam captures stdout/stderr per launch) stay distinguishable.

use std::env;
use std::io::Write;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

/// Filter used when `RUST_LOG` is unset. Steam launch options are easy to forget, and
/// a launch that logs nothing is exactly the launch we most want to read afterwards.
const DEFAULT_FILTER: &str = "info,galactic_repoman=debug";

/// Environment that decides which windowing backend, Vulkan driver, and Steam/gamescope
/// shims a launch ends up with. When a relaunch behaves differently from the first
/// launch after a reboot, the difference usually shows up here before it shows up
/// anywhere else, so record it verbatim on every run.
const REPORTED_ENV: &[&str] = &[
    "RUST_LOG",
    // Windowing backend selection.
    "XDG_SESSION_TYPE",
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "WINIT_UNIX_BACKEND",
    // gamescope / Steam session.
    "GAMESCOPE_WAYLAND_DISPLAY",
    "ENABLE_GAMESCOPE_WSI",
    "STEAM_MULTIPLE_XWAYLANDS",
    "STEAM_GAMESCOPE_HDR_SUPPORTED",
    "SteamAppId",
    "SteamGameId",
    "STEAM_COMPAT_DATA_PATH",
    "LD_PRELOAD",
    // Vulkan loader, driver, and injected layers.
    "VK_ICD_FILENAMES",
    "VK_DRIVER_FILES",
    "VK_INSTANCE_LAYERS",
    "VK_LOADER_LAYERS_ENABLE",
    "VK_LAYER_PATH",
    "AMD_VULKAN_ICD",
    "MESA_LOADER_DRIVER_OVERRIDE",
    "MESA_VK_DEVICE_SELECT",
    "RADV_DEBUG",
    "RADV_PERFTEST",
    "MANGOHUD",
    "OBS_VKCAPTURE",
];

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

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(DEFAULT_FILTER))
        .format_key_values(move |buf, kvs| {
            write!(buf, " run={run}")?;
            env_logger::fmt::default_kv_format(buf, kvs)
        })
        .init();

    let exe = env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|e| format!("<unknown: {e}>"));
    log::info!("run started: pid {}, exe {exe}", process::id());

    log_environment();
}

/// Dump the launch environment we care about. Absent variables are simply missing
/// from the log — comparing two runs' blocks shows both changed and vanished values.
fn log_environment() {
    REPORTED_ENV
        .iter()
        .filter_map(|name| env::var(name).ok().map(|value| (name, value)))
        .for_each(|(name, value)| log::info!("env {name}={value}"));
}
