//! Where OpenHardwareOS keeps its files.
//!
//! Local-first by design: everything lives in the user's config directory, no
//! account, no cloud, no telemetry.
//!
//! | Platform | Root |
//! |----------|------|
//! | Windows  | `%APPDATA%\OpenHardwareOS` |
//! | macOS    | `~/Library/Application Support/OpenHardwareOS` |
//! | Linux    | `~/.config/OpenHardwareOS` (or `$XDG_CONFIG_HOME`) |
//!
//! `OHM_CONFIG_DIR` overrides the root, which is what the test-suite and the
//! portable/`--portable` build use.

use std::path::{Path, PathBuf};

use crate::PRODUCT_SLUG;
use crate::error::{OhmError, Result};

/// Environment variable that overrides the whole configuration root.
pub const ENV_CONFIG_DIR: &str = "OHM_CONFIG_DIR";

/// Resolved layout of the on-disk state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    root: PathBuf,
}

impl ConfigPaths {
    /// Resolve the platform default root (or `$OHM_CONFIG_DIR`).
    pub fn discover() -> Result<Self> {
        Self::discover_with(std::env::var_os(ENV_CONFIG_DIR).map(PathBuf::from))
    }

    /// Resolution with an explicit override, kept separate so it is testable
    /// without mutating the process environment.
    pub fn discover_with(override_root: Option<PathBuf>) -> Result<Self> {
        if let Some(dir) = override_root {
            return Ok(Self::from_root(dir));
        }
        let base = dirs::config_dir().ok_or_else(|| {
            OhmError::Config("could not determine the platform config directory".into())
        })?;
        Ok(Self::from_root(base.join(PRODUCT_SLUG)))
    }

    /// Use an explicit root. Used by tests and the portable build.
    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Configuration root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Main settings file (`settings.json`).
    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    /// Directory holding one YAML file per automation rule.
    pub fn rules_dir(&self) -> PathBuf {
        self.root.join("rules")
    }

    /// Directory holding rotating log files.
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// What a running background service says about itself.
    ///
    /// One writer at a time: the file is created atomically by whoever starts the
    /// service, rewritten on a heartbeat, and removed on a clean stop. Its presence
    /// with a fresh heartbeat is how a second process — another service, or the
    /// desktop application — knows that something else is already driving the
    /// channels. See `ohm_runtime::service`.
    pub fn service_state_file(&self) -> PathBuf {
        self.root.join("service.json")
    }

    /// Append-only audit trail of every hardware write.
    pub fn audit_log(&self) -> PathBuf {
        self.root.join("audit.jsonl")
    }

    /// Create the directories the runtime needs. Idempotent.
    pub fn ensure(&self) -> Result<()> {
        for dir in [self.root.clone(), self.rules_dir(), self.logs_dir()] {
            std::fs::create_dir_all(&dir).map_err(|e| OhmError::io(&dir, e))?;
        }
        Ok(())
    }

    /// Render the layout for `--print-config-paths` style diagnostics.
    pub fn describe(&self) -> String {
        format!(
            "root:      {}\nsettings:  {}\nrules:     {}\nlogs:      {}\nservice:   {}\naudit:     {}",
            self.root.display(),
            self.settings_file().display(),
            self.rules_dir().display(),
            self.logs_dir().display(),
            self.service_state_file().display(),
            self.audit_log().display(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_derived_from_root() {
        let paths = ConfigPaths::from_root("/tmp/ohm-test");
        assert_eq!(
            paths.settings_file(),
            PathBuf::from("/tmp/ohm-test/settings.json")
        );
        assert_eq!(paths.rules_dir(), PathBuf::from("/tmp/ohm-test/rules"));
        assert_eq!(
            paths.audit_log(),
            PathBuf::from("/tmp/ohm-test/audit.jsonl")
        );
        assert!(paths.describe().contains("/tmp/ohm-test"));
    }

    #[test]
    fn ensure_creates_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::from_root(tmp.path().join("nested"));
        paths.ensure().unwrap();
        assert!(paths.rules_dir().is_dir());
        assert!(paths.logs_dir().is_dir());
        // idempotent
        paths.ensure().unwrap();
    }

    #[test]
    fn env_override_wins() {
        let paths = ConfigPaths::discover_with(Some(PathBuf::from("/tmp/ohm-env-root"))).unwrap();
        assert_eq!(paths.root(), Path::new("/tmp/ohm-env-root"));
        // Without an override we must land on a platform config directory that
        // ends with the product slug.
        let default = ConfigPaths::discover_with(None).unwrap();
        assert!(default.root().ends_with(PRODUCT_SLUG));
    }
}
