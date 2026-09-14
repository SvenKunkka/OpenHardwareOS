//! Start with Windows, and privilege reporting.
//!
//! Autostart is a per-user registry value under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. That is deliberately the
//! *user* hive: a `Run` entry can never start an elevated process, so pretending
//! otherwise would produce a silently useless setting.
//!
//! The consequence is documented in the UI: OpenHardwareOS starts unprivileged,
//! which is fine for monitoring and for talking to LibreHardwareMonitor's web
//! server (LHM itself is the elevated part). Only paths that need SuperIO access
//! *directly* require the user to start it as Administrator.
//!
//! On macOS and Linux this is a no-op that reports why, instead of failing
//! silently.

use crate::state::{CommandError, CommandResult};

/// Registry value name.
pub const RUN_VALUE: &str = "OpenHardwareOS";

/// Is the current process elevated?
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        // `net session` requires an elevated token and is available everywhere.
        std::process::Command::new("net")
            .arg("session")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        // A desktop app on Unix has no elevation concept; effective uid 0 is the
        // closest equivalent, and running the GUI as root is not supported.
        false
    }
}

/// Is autostart currently registered?
pub fn is_enabled() -> bool {
    #[cfg(windows)]
    {
        query_run_value().is_some()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Register or remove the autostart entry.
pub fn set_enabled(enable: bool) -> CommandResult<bool> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|error| {
            CommandError::new(
                "autostart_failed",
                format!("could not determine the executable path: {error}"),
                "Start OpenHardwareOS from its installed location.",
            )
        })?;
        let exe = exe.display().to_string();

        let status = if enable {
            std::process::Command::new("reg")
                .args([
                    "add",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                    "/v",
                    RUN_VALUE,
                    "/t",
                    "REG_SZ",
                    "/d",
                    &format!("\"{exe}\""),
                    "/f",
                ])
                .status()
        } else {
            std::process::Command::new("reg")
                .args([
                    "delete",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                    "/v",
                    RUN_VALUE,
                    "/f",
                ])
                .status()
        }
        .map_err(|error| {
            CommandError::new(
                "autostart_failed",
                format!("could not run `reg`: {error}"),
                "Set the startup entry manually in Task Manager -> Startup apps.",
            )
        })?;

        if enable && !status.success() {
            return Err(CommandError::new(
                "autostart_failed",
                "the registry entry could not be written",
                "Check whether a policy blocks per-user Run entries.",
            ));
        }
        // Deleting a missing value is not an error.
        Ok(is_enabled())
    }
    #[cfg(not(windows))]
    {
        if enable {
            Err(CommandError::unavailable(
                "starting with the operating system is implemented for Windows only",
                "On macOS add OpenHardwareOS to Login Items; on Linux use a desktop autostart \
                 entry.",
            ))
        } else {
            Ok(false)
        }
    }
}

#[cfg(windows)]
fn query_run_value() -> Option<String> {
    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            RUN_VALUE,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    text.lines()
        .find(|line| line.contains(RUN_VALUE))
        .map(|line| line.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevation_probe_never_panics() {
        // The answer depends on the machine; the call must simply succeed.
        let _ = is_elevated();
    }

    #[cfg(not(windows))]
    #[test]
    fn autostart_is_reported_as_windows_only() {
        assert!(!is_enabled());
        let error = set_enabled(true).unwrap_err();
        assert!(error.unsupported);
        assert!(error.hint.contains("Login Items"));
        assert!(!set_enabled(false).unwrap());
    }

    #[test]
    fn run_value_name_is_stable() {
        assert_eq!(RUN_VALUE, "OpenHardwareOS");
    }
}
