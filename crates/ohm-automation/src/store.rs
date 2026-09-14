//! Rule persistence: one YAML file per rule in `<config>/rules`.
//!
//! Files, not a database, because rules are user artefacts: they can be read,
//! diffed, version-controlled and shared. A rule that fails to parse never
//! prevents the app from starting — it is reported and skipped.

use std::path::{Path, PathBuf};

use ohm_core::{ConfigPaths, OhmError, Result, RuleId};
use serde::{Deserialize, Serialize};

use crate::rule::Rule;

/// A rule file that could not be loaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleFileError {
    pub path: PathBuf,
    pub message: String,
}

/// A rule file that loaded, but whose content had to be adjusted in memory.
///
/// The file itself is never rewritten: the user decides what to do with it. This
/// exists so that a substitution is never silent — and it is structured rather than a
/// single sentence, so the app can say *which field* of *which file* held what, what is
/// actually in force, and what to do about it. A user told only "something was
/// adjusted" has been told nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleFileNote {
    pub path: PathBuf,
    pub rule_id: RuleId,
    /// The setting that was adjusted, e.g. `fallback.on_sensor_missing`.
    pub field: String,
    /// What the file on disk says.
    pub original: String,
    /// What is in force instead.
    pub effective: String,
    /// One sentence describing the substitution.
    pub message: String,
    /// What the user can do about it.
    pub hint: String,
}

/// Result of loading the rules directory.
#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    pub rules: Vec<Rule>,
    pub errors: Vec<RuleFileError>,
    /// Loaded, but adjusted in memory — currently only `release` substitutions.
    pub notes: Vec<RuleFileNote>,
    pub directory: PathBuf,
}

impl LoadReport {
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Reads and writes rule YAML files.
#[derive(Debug, Clone)]
pub struct RuleStore {
    directory: PathBuf,
}

impl RuleStore {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    /// `<config>/rules`
    pub fn from_paths(paths: &ConfigPaths) -> Self {
        Self::new(paths.rules_dir())
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// `rules/<id>.yaml`
    pub fn path_for(&self, id: &RuleId) -> PathBuf {
        self.directory.join(format!("{id}.yaml"))
    }

    pub fn ensure_directory(&self) -> Result<()> {
        std::fs::create_dir_all(&self.directory).map_err(|e| OhmError::io(&self.directory, e))
    }

    pub fn exists(&self, id: &RuleId) -> bool {
        self.path_for(id).is_file()
    }

    /// Load every rule, collecting per-file errors instead of failing.
    pub fn load_report(&self) -> LoadReport {
        let mut report = LoadReport {
            directory: self.directory.clone(),
            ..LoadReport::default()
        };
        if !self.directory.exists() {
            return report;
        }
        let entries = match std::fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(err) => {
                report.errors.push(RuleFileError {
                    path: self.directory.clone(),
                    message: err.to_string(),
                });
                return report;
            }
        };

        let mut paths: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .is_some_and(|ext| ext == "yaml" || ext == "yml")
            })
            .collect();
        paths.sort();

        for path in paths {
            match self.load_file(&path) {
                Ok((rule, notes)) => {
                    report.rules.push(rule);
                    report.notes.extend(notes);
                }
                Err(err) => {
                    tracing::warn!(path = %path.display(), error = %err, "skipping unreadable rule file");
                    report.errors.push(RuleFileError {
                        path,
                        message: err.to_string(),
                    });
                }
            }
        }

        report.rules.sort_by(|a, b| a.id.cmp(&b.id));
        report
    }

    /// Load every valid rule. Unreadable files are logged and skipped.
    pub fn load_all(&self) -> Result<Vec<Rule>> {
        Ok(self.load_report().rules)
    }

    fn load_file(&self, path: &Path) -> Result<(Rule, Vec<RuleFileNote>)> {
        let raw = std::fs::read_to_string(path).map_err(|e| OhmError::io(path, e))?;
        let mut rule: Rule = serde_yaml_ng::from_str(&raw)
            .map_err(|e| OhmError::Config(format!("{}: {e}", path.display())))?;
        // The file name is authoritative for the id, so renaming a file does not
        // silently change which rule the UI edits.
        if let Some(file_id) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| RuleId::new(stem).ok())
            .filter(|file_id| rule.id != *file_id)
        {
            {
                tracing::debug!(
                    path = %path.display(),
                    declared = rule.id.as_str(),
                    file = file_id.as_str(),
                    "rule id taken from the file name"
                );
                rule.id = file_id;
            }
        }
        // An action this build cannot perform is substituted here, in memory, and
        // reported: refusing to load the file would leave a machine unmanaged, and
        // rewriting it would make the change invisible.
        let mut notes = Vec::new();
        for (field, action) in [
            ("on_sensor_missing", rule.fallback.on_sensor_missing),
            ("on_write_failure", rule.fallback.on_write_failure),
        ] {
            if action.is_unsupported() {
                // The duty actually applied comes from the safety settings, so the
                // note names the policy rather than inventing a number.
                let effective =
                    "safe_default (the fail-safe duty from the safety settings)".to_string();
                let message = format!(
                    "fallback.{field}: `release` is not supported — no adapter in this build can \
                     hand a channel back while the app runs. Running with {effective} instead. \
                     The file was not modified."
                );
                let hint = format!(
                    "Edit {} and set fallback.{field} to `safe_default` to make the file match what \
                     is running, or to a fixed duty you prefer.",
                    path.display()
                );
                tracing::warn!(path = %path.display(), rule = rule.id.as_str(), "{message}");
                match field {
                    "on_sensor_missing" => {
                        rule.fallback.on_sensor_missing = crate::rule::FallbackAction::SafeDefault;
                    }
                    _ => {
                        rule.fallback.on_write_failure = crate::rule::FallbackAction::SafeDefault;
                    }
                }
                notes.push(RuleFileNote {
                    path: path.to_path_buf(),
                    rule_id: rule.id.clone(),
                    field: format!("fallback.{field}"),
                    original: "release".to_string(),
                    effective,
                    message,
                    hint,
                });
            }
        }

        rule.validate()?;
        Ok((rule, notes))
    }

    /// Write a rule to disk atomically.
    pub fn save(&self, rule: &Rule) -> Result<PathBuf> {
        self.ensure_directory()?;
        let path = self.path_for(&rule.id);
        let yaml = serde_yaml_ng::to_string(rule)
            .map_err(|e| OhmError::Config(format!("could not serialise rule: {e}")))?;
        let header = format!(
            "# OpenHardwareOS automation rule\n# Managed by the app; hand edits are welcome.\n# id: {}\n",
            rule.id
        );
        let tmp = path.with_extension("yaml.tmp");
        std::fs::write(&tmp, format!("{header}{yaml}")).map_err(|e| OhmError::io(&tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| OhmError::io(&path, e))?;
        Ok(path)
    }

    /// Delete a rule file. Returns `false` when it did not exist.
    pub fn delete(&self, id: &RuleId) -> Result<bool> {
        let path = self.path_for(id);
        if !path.exists() {
            return Ok(false);
        }
        std::fs::remove_file(&path).map_err(|e| OhmError::io(&path, e))?;
        Ok(true)
    }

    /// A starter rule written on first run so the Automation page is not empty.
    pub fn ensure_example(&self) -> Result<bool> {
        self.ensure_directory()?;
        let id = RuleId::new(crate::examples::GPU_COOLING_ID)?;
        if self.exists(&id) {
            return Ok(false);
        }
        self.save(&crate::examples::gpu_cooling_example())?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::{Fallback, FallbackAction};

    fn store() -> (tempfile::TempDir, RuleStore) {
        let tmp = tempfile::tempdir().unwrap();
        let store = RuleStore::new(tmp.path().join("rules"));
        (tmp, store)
    }

    #[test]
    fn save_then_load_roundtrip() {
        let (_tmp, store) = store();
        let rule = crate::examples::gpu_cooling_example();
        let path = store.save(&rule).unwrap();
        assert!(path.ends_with("gpu-cooling.yaml"));
        assert!(store.exists(&rule.id));

        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], rule);

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.starts_with("# OpenHardwareOS automation rule"));
        assert!(raw.contains("name: GPU Cooling"));
    }

    #[test]
    fn listing_is_sorted_and_ignores_other_files() {
        let (_tmp, store) = store();
        store.ensure_directory().unwrap();
        let mut second = crate::examples::gpu_cooling_example();
        second.id = RuleId::new("aaa-first").unwrap();
        let mut third = crate::examples::gpu_cooling_example();
        third.id = RuleId::new("zzz-last").unwrap();
        store.save(&second).unwrap();
        store.save(&third).unwrap();
        std::fs::write(store.directory().join("notes.txt"), "ignore me").unwrap();

        let rules = store.load_all().unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].id.as_str(), "aaa-first");
        assert_eq!(rules[1].id.as_str(), "zzz-last");
    }

    #[test]
    fn broken_files_are_reported_not_fatal() {
        let (_tmp, store) = store();
        store.ensure_directory().unwrap();
        std::fs::write(store.directory().join("broken.yaml"), "name: [not: valid").unwrap();
        std::fs::write(
            store.directory().join("incomplete.yaml"),
            "name: No Source\n",
        )
        .unwrap();
        store.save(&crate::examples::gpu_cooling_example()).unwrap();

        let report = store.load_report();
        assert_eq!(report.rules.len(), 1, "the good rule still loads");
        assert_eq!(report.errors.len(), 2);
        assert!(!report.is_clean());
        assert!(report.errors.iter().all(|e| !e.message.is_empty()));
    }

    #[test]
    fn file_name_wins_over_declared_id() {
        let (_tmp, store) = store();
        store.ensure_directory().unwrap();
        let rule = crate::examples::gpu_cooling_example();
        let yaml = serde_yaml_ng::to_string(&rule).unwrap();
        std::fs::write(store.directory().join("renamed.yaml"), yaml).unwrap();
        let loaded = store.load_all().unwrap();
        assert_eq!(loaded[0].id.as_str(), "renamed");
        assert_eq!(loaded[0].name, "GPU Cooling");
    }

    #[test]
    fn delete_is_idempotent() {
        let (_tmp, store) = store();
        let rule = crate::examples::gpu_cooling_example();
        store.save(&rule).unwrap();
        assert!(store.delete(&rule.id).unwrap());
        assert!(!store.delete(&rule.id).unwrap());
        assert!(store.load_all().unwrap().is_empty());
    }

    #[test]
    fn example_is_written_only_once() {
        let (_tmp, store) = store();
        assert!(store.ensure_example().unwrap());
        assert!(!store.ensure_example().unwrap());
        assert_eq!(store.load_all().unwrap().len(), 1);
    }

    #[test]
    fn missing_directory_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let store = RuleStore::new(tmp.path().join("does-not-exist"));
        let report = store.load_report();
        assert!(report.rules.is_empty());
        assert!(report.errors.is_empty());
    }

    #[test]
    fn fallbacks_survive_a_roundtrip() {
        let (_tmp, store) = store();
        let rule = crate::examples::gpu_cooling_example().with_fallback(Fallback {
            on_sensor_missing: FallbackAction::Fixed { percent: 90.0 },
            on_write_failure: FallbackAction::Hold,
            sensor_timeout_s: 3,
        });
        store.save(&rule).unwrap();
        let loaded = store.load_all().unwrap().remove(0);
        assert_eq!(loaded.fallback.sensor_timeout_s, 3);
        assert_eq!(
            loaded.fallback.on_sensor_missing,
            FallbackAction::Fixed { percent: 90.0 }
        );
        assert_eq!(loaded.fallback.on_write_failure, FallbackAction::Hold);
    }
}
