//! Linux `hwmon` fan channels, read from sysfs.
//!
//! The kernel's hwmon subsystem is the one place a Linux machine exposes chassis
//! fan tachometers (`fan<N>_input`) and the current PWM duty (`pwm<N>`). Whether
//! any of it exists depends on the board and the driver — a SuperIO driver such
//! as `nct6798d`, a GPU driver, an AIO — and every file can be absent or
//! unreadable on its own, so each field is probed and every absence carries a
//! reason instead of a zero.
//!
//! This module is deliberately platform-independent: it takes the directory to
//! read (`/sys/class/hwmon` in production, a fixture tree in the tests) and does
//! pure path and parse work. The Linux-only part is choosing the directory and
//! calling into here.
//!
//! **Read-only, deliberately.** Writing `pwm<N>` needs root, and a wrong value
//! on a board whose channel mapping nobody verified is the classic "fans stop"
//! failure. Monitoring is what this release promises; control over an confirmed
//! channel is separate work.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use ohm_device_model::UnavailableReason;

/// Where `/sys/class/hwmon` lives on a normal Linux system.
pub const DEFAULT_ROOT: &str = "/sys/class/hwmon";

/// The kernel's PWM interface is a byte.
const PWM_FULL_SCALE: f64 = 255.0;

/// Who currently owns a channel's duty, as `pwm<N>_enable` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlMode {
    /// The driver is driving the channel (a curve, a table, firmware).
    Automatic,
    /// Software is expected to set the duty — but not this program, yet.
    Manual,
    /// A value this build does not know how to read.
    Other(i64),
    /// The file could not be read, with the reason and the platform's message.
    Unknown(UnavailableReason, String),
}

impl ControlMode {
    /// Parse the kernel's `pwm<N>_enable` value.
    ///
    /// 0 = no control (full speed), 1 = manual, 2 = automatic, 3 = automatic
    /// with a driver curve. Anything else is reported as-is rather than guessed.
    pub fn from_sysfs(raw: &str) -> Self {
        match raw.trim().parse::<i64>() {
            Ok(1) => Self::Manual,
            Ok(2 | 3) => Self::Automatic,
            Ok(0) => Self::Other(0),
            Ok(other) => Self::Other(other),
            Err(_) => Self::Other(-1),
        }
    }

    /// One line for the UI and the audit trail.
    pub fn describe(&self) -> String {
        match self {
            Self::Automatic => "the driver controls this channel".to_string(),
            Self::Manual => "software is expected to set this channel".to_string(),
            Self::Other(0) => "no control; the channel runs at full speed".to_string(),
            Self::Other(value) => format!("unrecognised control mode {value}"),
            Self::Unknown(reason, detail) => {
                format!("control mode unreadable ({}): {detail}", reason.as_str())
            }
        }
    }
}

/// One fan channel of one hwmon chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FanChannel {
    /// Chip name from `name`, e.g. `nct6798d`. Part of the device id.
    pub chip: String,
    /// Channel number from the file names (`fan1_input` -> 1).
    pub channel: u32,
    /// `fan<N>_label`, when the driver provides one.
    pub label: Option<String>,
    /// `fan<N>_input` — the tachometer.
    pub rpm: Option<PathBuf>,
    /// `pwm<N>` — the duty, if this channel has one.
    pub pwm: Option<PathBuf>,
    /// `pwm<N>_enable` — who owns the duty.
    pub mode: Option<(PathBuf, ControlMode)>,
}

impl FanChannel {
    /// Stable device id: chip name and channel number, never enumeration order.
    ///
    /// A positional id (`fan.system.0`) would point at a different header if the
    /// kernel enumerated the chips in another order, and saved rules address
    /// devices by id.
    pub fn device_id(&self) -> String {
        format!("fan.system.{}_fan{}", slug(&self.chip), self.channel)
    }

    /// Human name: the driver's label when there is one.
    pub fn display_name(&self) -> String {
        match &self.label {
            Some(label) if !label.trim().is_empty() => {
                format!("{} — {}", label.trim(), self.chip)
            }
            _ => format!("{} fan {}", self.chip, self.channel),
        }
    }

    /// The channel's duty as a percentage, and the raw byte beside it.
    pub fn read_pwm(&self) -> Option<Result<(f64, i64), (UnavailableReason, String)>> {
        let path = self.pwm.as_ref()?;
        Some(match read_number(path) {
            Ok(raw) => Ok(((raw / PWM_FULL_SCALE * 100.0).clamp(0.0, 100.0), raw as i64)),
            Err(failure) => Err(failure),
        })
    }

    /// The tachometer, in RPM.
    pub fn read_rpm(&self) -> Option<Result<f64, (UnavailableReason, String)>> {
        let path = self.rpm.as_ref()?;
        Some(read_number(path))
    }

    /// Control mode, read from the filesystem now (not from discovery).
    pub fn read_mode(&self) -> Option<(UnavailableReason, String)> {
        let (path, _) = self.mode.as_ref()?;
        match fs::read_to_string(path) {
            Ok(raw) => match ControlMode::from_sysfs(&raw) {
                ControlMode::Unknown(reason, detail) => Some((reason, detail)),
                _ => None,
            },
            Err(error) => Some(classify(&error, path)),
        }
    }

    /// The control mode as it was discovered, refreshed when readable.
    pub fn current_mode(&self) -> Option<ControlMode> {
        let (path, fallback) = self.mode.as_ref()?;
        match fs::read_to_string(path) {
            Ok(raw) => Some(ControlMode::from_sysfs(&raw)),
            Err(error) => {
                let (reason, detail) = classify(&error, path);
                let _ = fallback;
                Some(ControlMode::Unknown(reason, detail))
            }
        }
    }
}

/// What a chip turned out to expose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HwmonChip {
    /// Directory name, e.g. `hwmon3`.
    pub directory: String,
    /// Contents of `name`.
    pub name: String,
    pub fans: Vec<FanChannel>,
}

/// The result of one scan, plus what could not be read and why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HwmonTree {
    pub chips: Vec<HwmonChip>,
    /// Notes about skipped or unreadable chips, for the log and the UI.
    pub notes: Vec<String>,
}

impl HwmonTree {
    /// Every fan channel, in a stable order.
    pub fn channels(&self) -> impl Iterator<Item = (&HwmonChip, &FanChannel)> {
        self.chips
            .iter()
            .flat_map(|chip| chip.fans.iter().map(move |fan| (chip, fan)))
    }

    pub fn is_empty(&self) -> bool {
        self.chips.iter().all(|chip| chip.fans.is_empty())
    }
}

/// Scan `root` (`/sys/class/hwmon`) for chips and their fan channels.
///
/// A missing root is not an error: plenty of machines — containers, VMs, ARM
/// boards — have no hwmon at all, and an empty result is the honest answer.
pub fn discover(root: &Path) -> HwmonTree {
    let mut tree = HwmonTree::default();
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return tree,
        Err(error) => {
            tree.notes
                .push(format!("cannot read {}: {error}", root.display()));
            return tree;
        }
    };

    // `hwmon10` sorts after `hwmon9` by number, not by string.
    let mut directories: Vec<(u32, PathBuf, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(number) = name
            .strip_prefix("hwmon")
            .and_then(|rest| rest.parse::<u32>().ok())
        else {
            continue;
        };
        directories.push((number, entry.path(), name));
    }
    directories.sort_by_key(|(number, _, _)| *number);

    for (_, path, directory) in directories {
        let name_path = path.join("name");
        let name = match fs::read_to_string(&name_path) {
            Ok(name) => name.trim().to_string(),
            Err(error) => {
                tree.notes
                    .push(format!("{directory} has no readable `name`: {error}"));
                continue;
            }
        };
        if name.is_empty() {
            tree.notes.push(format!("{directory} has an empty `name`"));
            continue;
        }

        let mut chips_with = HwmonChip {
            directory,
            name,
            fans: Vec::new(),
        };
        for channel in fan_numbers(&path) {
            let rpm = readable_file(&path, &format!("fan{channel}_input"));
            let pwm = readable_file(&path, &format!("pwm{channel}"));
            if rpm.is_none() && pwm.is_none() {
                continue;
            }
            let label = fs::read_to_string(path.join(format!("fan{channel}_label")))
                .ok()
                .map(|label| label.trim().to_string())
                .filter(|label| !label.is_empty());
            let mode = fs::read_to_string(path.join(format!("pwm{channel}_enable")))
                .ok()
                .map(|raw| {
                    (
                        path.join(format!("pwm{channel}_enable")),
                        ControlMode::from_sysfs(&raw),
                    )
                });
            chips_with.fans.push(FanChannel {
                chip: chips_with.name.clone(),
                channel,
                label,
                rpm,
                pwm,
                mode,
            });
        }
        tree.chips.push(chips_with);
    }

    tree
}

/// Channel numbers that appear as `fan<N>_input` or `pwm<N>`, sorted.
fn fan_numbers(path: &Path) -> Vec<u32> {
    let mut numbers: Vec<u32> = Vec::new();
    let Ok(entries) = fs::read_dir(path) else {
        return numbers;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let number = name
            .strip_prefix("fan")
            .and_then(|rest| rest.strip_suffix("_input"))
            .or_else(|| {
                name.strip_prefix("pwm")
                    .filter(|rest| rest.chars().all(|c| c.is_ascii_digit()))
            })
            .and_then(|digits| digits.parse::<u32>().ok());
        if let Some(number) = number {
            numbers.push(number);
        }
    }
    numbers.sort_unstable();
    numbers.dedup();
    numbers
}

/// A path only when it is a readable regular file, so discovery does not
/// advertise a channel whose file cannot be read.
fn readable_file(directory: &Path, name: &str) -> Option<PathBuf> {
    let path = directory.join(name);
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => Some(path),
        _ => None,
    }
}

/// Read an integer from a sysfs file.
fn read_number(path: &Path) -> Result<f64, (UnavailableReason, String)> {
    let raw = fs::read_to_string(path).map_err(|error| classify(&error, path))?;
    let trimmed = raw.trim();
    trimmed.parse::<f64>().map_err(|_| {
        (
            UnavailableReason::ReadError,
            format!("not a number: {trimmed:?}"),
        )
    })
}

/// Turn an I/O failure into the model's vocabulary, keeping the system's text.
fn classify(error: &std::io::Error, path: &Path) -> (UnavailableReason, String) {
    let detail = format!("{}: {error}", path.display());
    match error.kind() {
        ErrorKind::NotFound => (UnavailableReason::NotPresent, detail),
        ErrorKind::PermissionDenied => (UnavailableReason::PermissionDenied, detail),
        _ => (UnavailableReason::ReadError, detail),
    }
}

/// Lowercase, id-safe form of a chip name.
fn slug(value: &str) -> String {
    let slug: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if slug.is_empty() {
        "hwmon".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A sysfs tree the test controls: `hwmon0` with two fans, `hwmon1` with none.
    fn fixture() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        let chip = root.join("hwmon2");
        fs::create_dir_all(&chip).unwrap();
        fs::write(chip.join("name"), "nct6798d\n").unwrap();
        fs::write(chip.join("fan1_input"), "1200\n").unwrap();
        fs::write(chip.join("fan1_label"), "CPU Fan\n").unwrap();
        fs::write(chip.join("pwm1"), "128\n").unwrap();
        fs::write(chip.join("pwm1_enable"), "1\n").unwrap();
        fs::write(chip.join("fan2_input"), "0\n").unwrap();
        fs::write(chip.join("pwm2"), "0\n").unwrap();
        fs::write(chip.join("pwm2_enable"), "2\n").unwrap();

        let other = root.join("hwmon10");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("name"), "k10temp\n").unwrap();
        fs::write(other.join("temp1_input"), "45000\n").unwrap();

        temp
    }

    #[test]
    fn a_chip_is_read_by_name_not_by_directory_order() {
        let temp = fixture();
        let tree = discover(temp.path());
        let names: Vec<&str> = tree.chips.iter().map(|chip| chip.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["nct6798d", "k10temp"],
            "hwmon10 sorts after hwmon2"
        );

        let (_, fan) = tree.channels().next().unwrap();
        assert_eq!(fan.chip, "nct6798d");
        assert_eq!(fan.channel, 1);
        assert_eq!(fan.device_id(), "fan.system.nct6798d_fan1");
        assert_eq!(fan.display_name(), "CPU Fan — nct6798d");
    }

    /// The id must not depend on how many chips exist or in what order.
    #[test]
    fn device_ids_are_stable_when_the_kernel_renumbers_chips() {
        let temp = fixture();
        let before: Vec<String> = discover(temp.path())
            .channels()
            .map(|(_, fan)| fan.device_id())
            .collect();

        // The same chip, now discovered under a different hwmon number, with
        // another chip appearing before it.
        fs::rename(temp.path().join("hwmon2"), temp.path().join("hwmon0")).unwrap();
        let tree = discover(temp.path());
        let after: Vec<String> = tree.channels().map(|(_, fan)| fan.device_id()).collect();

        assert_eq!(
            before, after,
            "renumbering must not change a channel's identity"
        );
    }

    #[test]
    fn readings_come_from_the_files_and_carry_units_of_their_own() {
        let temp = fixture();
        let tree = discover(temp.path());
        let (_, fan) = tree.channels().next().unwrap();

        assert_eq!(fan.read_rpm().unwrap().unwrap(), 1200.0);
        let (percent, raw) = fan.read_pwm().unwrap().unwrap();
        assert_eq!(raw, 128);
        assert!((percent - 50.196).abs() < 0.01, "{percent}");
        assert_eq!(fan.current_mode(), Some(ControlMode::Manual));
        assert_eq!(
            fan.current_mode().unwrap().describe(),
            "software is expected to set this channel"
        );
    }

    /// A fan the driver reports as stopped is a reading, not a missing sensor.
    #[test]
    fn a_zero_rpm_is_a_reading_not_an_absence() {
        let temp = fixture();
        let tree = discover(temp.path());
        let (_, fan) = tree
            .channels()
            .find(|(_, fan)| fan.channel == 2)
            .expect("channel 2 exists");
        assert_eq!(fan.read_rpm().unwrap().unwrap(), 0.0);
        assert_eq!(fan.current_mode(), Some(ControlMode::Automatic));
    }

    #[test]
    fn a_channel_that_disappears_reports_a_reason_not_a_zero() {
        let temp = fixture();
        let tree = discover(temp.path());
        let (_, fan) = tree.channels().next().unwrap();
        fs::remove_file(temp.path().join("hwmon2/fan1_input")).unwrap();

        let (reason, detail) = fan.read_rpm().unwrap().unwrap_err();
        assert_eq!(reason, UnavailableReason::NotPresent);
        assert!(detail.contains("fan1_input"), "{detail}");
    }

    #[test]
    fn content_that_is_not_a_number_is_a_read_error() {
        let temp = fixture();
        fs::write(temp.path().join("hwmon2/fan1_input"), "n/a\n").unwrap();
        let tree = discover(temp.path());
        let (_, fan) = tree.channels().next().unwrap();

        let (reason, detail) = fan.read_rpm().unwrap().unwrap_err();
        assert_eq!(reason, UnavailableReason::ReadError);
        assert!(detail.contains("n/a"), "{detail}");
    }

    #[test]
    fn a_chip_without_a_name_is_skipped_and_said_so() {
        let temp = fixture();
        fs::remove_file(temp.path().join("hwmon2/name")).unwrap();
        let tree = discover(temp.path());

        assert_eq!(tree.chips.len(), 1, "the unnamed chip is not guessed at");
        assert!(
            tree.notes.iter().any(|note| note.contains("hwmon2")),
            "{:?}",
            tree.notes
        );
    }

    #[test]
    fn a_missing_root_is_an_empty_tree_and_not_an_error() {
        let temp = tempfile::tempdir().unwrap();
        let tree = discover(&temp.path().join("nowhere"));
        assert!(tree.is_empty());
        assert!(tree.notes.is_empty(), "{:?}", tree.notes);
    }

    #[test]
    fn a_control_mode_this_build_does_not_know_is_reported_verbatim() {
        assert_eq!(ControlMode::from_sysfs("2\n"), ControlMode::Automatic);
        assert_eq!(ControlMode::from_sysfs("1\n"), ControlMode::Manual);
        assert_eq!(ControlMode::from_sysfs("0\n"), ControlMode::Other(0));
        assert_eq!(ControlMode::from_sysfs("7\n"), ControlMode::Other(7));
        assert!(ControlMode::from_sysfs("7").describe().contains('7'));
        assert_eq!(ControlMode::from_sysfs("bogus\n"), ControlMode::Other(-1));
    }
}
