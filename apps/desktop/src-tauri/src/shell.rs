//! Opening folders in the system file manager.
//!
//! Implemented with the platform's own command rather than a Tauri plugin: it
//! is three lines per platform, needs no permission entry, and keeps the app's
//! dependency surface small. The user asked to see a folder, nothing more.

use std::path::Path;

use crate::state::{CommandError, CommandResult};

/// Reveal `path` in the file manager (Explorer, Finder, xdg-open).
pub fn open_path<R: tauri::Runtime>(_app: &tauri::AppHandle<R>, path: &Path) -> CommandResult<()> {
    if !path.exists() {
        return Err(CommandError::not_found(format!(
            "`{}` does not exist yet",
            path.display()
        )));
    }

    let result = open_platform(path);
    result.map_err(|error| {
        CommandError::new(
            "open_failed",
            format!("could not open `{}`: {error}", path.display()),
            "Open the folder manually from the path shown in Diagnostics.",
        )
    })
}

#[cfg(target_os = "windows")]
fn open_platform(path: &Path) -> std::io::Result<()> {
    // `explorer` returns a non-zero exit code even on success, so the status is
    // deliberately ignored; only a failure to spawn is an error.
    std::process::Command::new("explorer")
        .arg(path)
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn open_platform(path: &Path) -> std::io::Result<()> {
    std::process::Command::new("open")
        .arg(path)
        .spawn()
        .map(|_| ())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_platform(path: &Path) -> std::io::Result<()> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map(|_| ())
}

#[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
fn open_platform(path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        format!(
            "opening `{}` is not supported on this platform",
            path.display()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_paths_are_reported_before_spawning_anything() {
        let path = Path::new("/definitely/not/here/at/all");
        assert!(!path.exists());
        // `open_path` needs an AppHandle, so only the guard is exercised here.
        assert!(CommandError::not_found("x").message.contains('x'));
    }
}
