// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! OpenHardwareOS desktop entry point.
//!
//! Flags:
//!
//! * `--selftest` — start the runtime headlessly, run one automation cycle,
//!   print a report and exit. Used by CI and by installation checks.
//! * `--mock` — register the simulated hardware providers even if Settings has
//!   them switched off.
//! * `--dry-run` — never write to hardware, whatever the saved settings say.

fn main() {
    if let Err(error) = ohm_desktop_lib::run() {
        eprintln!("OpenHardwareOS failed to start: {error}");
        std::process::exit(1);
    }
}
