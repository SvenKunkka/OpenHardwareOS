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
//! **Reading is unconditional; writing is per channel and opt-in.**
//!
//! Writing `pwm<N>` needs root, and a wrong value on a board whose channel mapping
//! nobody verified is the classic "fans stop" failure. `pwm<N>` and `fan<N>_input`
//! share a chip and a channel number, which is *not* a promise that `pwm1` drives
//! the header `fan1_input` measures — that pairing is a property of the board, and
//! no kernel interface states it. So this module can write, and the adapter only
//! exposes a writable duty for a channel somebody listed in
//! `adapter_settings.system.pwm_write_allow` after checking it on their machine.
//!
//! Taking control is explicit and reversible. If the driver owns the channel
//! (`pwm<N>_enable` is automatic) the adapter switches it to manual and remembers
//! what it was, so it can put it back on release; if the channel's ownership cannot
//! be read at all, nothing is written — a channel whose current owner is unknown is
//! not one to take over.

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
            // Not a number at all: that is not "a mode this build does not know",
            // it is ownership that could not be read. The difference decides
            // whether a write may take the channel over, so it is kept.
            Err(_) => Self::Unknown(
                UnavailableReason::ReadError,
                format!("pwm_enable is not a number ({:?})", raw.trim()),
            ),
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

    /// Set this channel's duty, as a percentage.
    ///
    /// Returns the raw byte the kernel was given, so the caller can report what it
    /// actually asked for rather than what it meant to ask for. The percentage is
    /// clamped to the interface's range here as well as by the safety layer: a
    /// negative or absurd value must never reach a fan because something upstream
    /// mis-computed it.
    pub fn set_pwm(&self, percent: f64) -> Result<i64, (UnavailableReason, String)> {
        let Some(path) = self.pwm.as_ref() else {
            return Err((
                UnavailableReason::Unsupported,
                format!(
                    "{} channel {} exposes no pwm file, so this build cannot set its duty",
                    self.chip, self.channel
                ),
            ));
        };
        if !percent.is_finite() {
            return Err((
                UnavailableReason::ReadError,
                format!("refusing to write a non-finite duty ({percent})"),
            ));
        }
        let clamped = percent.clamp(0.0, 100.0);
        let raw = (clamped / 100.0 * PWM_FULL_SCALE).round() as i64;
        fs::write(path, format!("{raw}\n")).map_err(|error| classify(&error, path))?;
        Ok(raw)
    }

    /// Take control of this channel, returning what it was so it can be put back.
    ///
    /// `Ok(None)` means the channel is already ours to drive (or has no ownership
    /// file at all); `Err` means it must not be written, with the reason.
    pub fn take_control(&self) -> Result<Option<ControlMode>, (UnavailableReason, String)> {
        let Some((path, _)) = self.mode.as_ref() else {
            // No `pwm<N>_enable`: the driver left the channel to software, or the
            // kernel does not model ownership for this chip.
            return Ok(None);
        };
        let mode = match self.current_mode() {
            Some(mode) => mode,
            None => return Ok(None),
        };
        match &mode {
            ControlMode::Manual => Ok(None),
            ControlMode::Automatic | ControlMode::Other(_) => {
                if mode.restorable_value().is_none() {
                    return Err((
                        UnavailableReason::Unsupported,
                        format!(
                            "refusing to take control of {} channel {}: {}",
                            self.chip,
                            self.channel,
                            mode.describe()
                        ),
                    ));
                }
                fs::write(path, "1\n").map_err(|error| classify(&error, path))?;
                Ok(Some(mode))
            }
            ControlMode::Unknown(reason, detail) => Err((
                *reason,
                format!(
                    "refusing to write {} channel {}: its ownership could not be read ({detail})",
                    self.chip, self.channel
                ),
            )),
        }
    }

    /// Put the channel back under whatever drove it before.
    ///
    /// Never writes a value it did not read: a mode that could not be read is
    /// reported instead, because guessing here decides who owns a fan.
    pub fn restore_control(&self, original: Option<ControlMode>) -> Result<(), String> {
        let Some(original) = original else {
            return Ok(());
        };
        let Some((path, _)) = self.mode.as_ref() else {
            return Ok(());
        };
        match original.restorable_value() {
            Some(value) => fs::write(path, format!("{value}\n"))
                .map_err(|error| format!("could not restore {}: {error}", path.display())),
            None => Err(format!(
                "not restoring {} channel {}: the mode it had could not be read, and writing \
                 a guess would decide who owns this fan",
                self.chip, self.channel
            )),
        }
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

/// The raw `pwm<N>_enable` value to write for a mode we read earlier.
impl ControlMode {
    /// The integer to write back to restore this mode, when there is one.
    ///
    /// `None` for a mode we could not read: restoring an unknown value means
    /// guessing, and guessing here changes who drives a fan.
    pub fn restorable_value(&self) -> Option<i64> {
        match self {
            Self::Manual => Some(1),
            Self::Automatic => Some(2),
            Self::Other(value) => Some(*value),
            Self::Unknown(..) => None,
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

    /// The channel with this device id, when the tree has it.
    pub fn channel_by_device_id(&self, device_id: &str) -> Option<&FanChannel> {
        self.channels()
            .map(|(_, fan)| fan)
            .find(|fan| fan.device_id() == device_id)
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
    /// A tree with one chip, one tachometer and one PWM channel, plus the file
    /// contents, so a write can be checked by reading the file back.
    fn writable_fixture(mode: Option<&str>) -> (tempfile::TempDir, FanChannel) {
        let dir = tempfile::tempdir().unwrap();
        let chip = dir.path().join("hwmon0");
        fs::create_dir_all(&chip).unwrap();
        fs::write(chip.join("name"), "nct6798d\n").unwrap();
        fs::write(chip.join("fan1_input"), "1200\n").unwrap();
        fs::write(chip.join("pwm1"), "128\n").unwrap();
        if let Some(mode) = mode {
            fs::write(chip.join("pwm1_enable"), format!("{mode}\n")).unwrap();
        }
        let tree = discover(dir.path());
        let (_, fan) = tree.channels().next().expect("one channel");
        (dir, fan.clone())
    }

    #[test]
    fn a_percentage_becomes_the_byte_the_kernel_expects() {
        let (dir, fan) = writable_fixture(Some("1"));
        assert_eq!(fan.set_pwm(50.0).unwrap(), 128, "50 % of 255, rounded");
        assert_eq!(
            fs::read_to_string(dir.path().join("hwmon0/pwm1")).unwrap(),
            "128\n"
        );
        assert_eq!(fan.set_pwm(100.0).unwrap(), 255);
        assert_eq!(fan.set_pwm(0.0).unwrap(), 0);
        // Out-of-range values are clamped here too, not only by the safety layer:
        // a negative duty reaching a fan is the failure this exists to prevent.
        assert_eq!(fan.set_pwm(-10.0).unwrap(), 0);
        assert_eq!(fan.set_pwm(1000.0).unwrap(), 255);
    }

    #[test]
    fn a_non_finite_duty_is_refused() {
        let (_dir, fan) = writable_fixture(Some("1"));
        let error = fan.set_pwm(f64::NAN).unwrap_err();
        assert!(error.1.contains("non-finite"), "{}", error.1);
    }

    #[test]
    fn taking_control_switches_the_driver_off_and_remembers_what_it_was() {
        let (dir, fan) = writable_fixture(Some("2"));
        let original = fan.take_control().unwrap().expect("it was the driver's");
        assert_eq!(original, ControlMode::Automatic);
        assert_eq!(
            fs::read_to_string(dir.path().join("hwmon0/pwm1_enable")).unwrap(),
            "1\n",
            "manual, so the write is not ignored by a driver curve"
        );

        // And putting it back is exact.
        fan.restore_control(Some(original)).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("hwmon0/pwm1_enable")).unwrap(),
            "2\n"
        );
    }

    #[test]
    fn a_channel_already_in_manual_needs_no_switch_and_nothing_to_restore() {
        let (dir, fan) = writable_fixture(Some("1"));
        assert_eq!(fan.take_control().unwrap(), None);
        assert_eq!(
            fs::read_to_string(dir.path().join("hwmon0/pwm1_enable")).unwrap(),
            "1\n"
        );
        fan.restore_control(None).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("hwmon0/pwm1_enable")).unwrap(),
            "1\n"
        );
    }

    #[test]
    fn an_ownership_that_cannot_be_read_is_not_taken_over() {
        // No `pwm1_enable` at all is fine — that is a chip without ownership
        // modelling. A file that exists but cannot be parsed is different: writing
        // over it would decide who owns a fan without knowing who did.
        let (dir, fan) = writable_fixture(None);
        assert_eq!(fan.take_control().unwrap(), None, "no ownership file");

        fs::write(dir.path().join("hwmon0/pwm1_enable"), "automatic\n").unwrap();
        let tree = discover(dir.path());
        let (_, fan) = tree.channels().next().unwrap();
        let mode = fan.current_mode().expect("a mode");
        assert!(matches!(mode, ControlMode::Unknown(..)), "{mode:?}");
        assert_eq!(
            mode.restorable_value(),
            None,
            "an unknown *number* can be restored verbatim; a value that is not a              number cannot, because writing it back would be a guess"
        );
        let error = fan.take_control().unwrap_err();
        assert!(
            error.1.contains("ownership could not be read"),
            "{}",
            error.1
        );
    }

    #[test]
    fn a_channel_without_a_pwm_file_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        let chip = dir.path().join("hwmon0");
        fs::create_dir_all(&chip).unwrap();
        fs::write(chip.join("name"), "nct6798d\n").unwrap();
        fs::write(chip.join("fan1_input"), "1200\n").unwrap();
        let tree = discover(dir.path());
        let (_, fan) = tree.channels().next().expect("a tachometer channel");
        let error = fan.set_pwm(50.0).unwrap_err();
        assert!(error.1.contains("no pwm file"), "{}", error.1);
    }

    #[test]
    fn ownership_that_is_not_a_number_is_neither_taken_nor_restored() {
        // `pwm1_enable` exists and says something that is not a mode. Taking the
        // channel over would decide who owns a fan without knowing who did; putting
        // it back would write a value that was never there.
        let (dir, fan) = writable_fixture(Some("automatic"));
        let mode = fan.current_mode().expect("a mode");
        assert!(matches!(mode, ControlMode::Unknown(..)), "{mode:?}");

        let error = fan.take_control().unwrap_err();
        assert!(
            error.1.contains("ownership could not be read"),
            "{}",
            error.1
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("hwmon0/pwm1_enable")).unwrap(),
            "automatic\n",
            "nothing was written over it"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("hwmon0/pwm1")).unwrap(),
            "128\n",
            "and the duty was left alone"
        );

        let refused = fan.restore_control(Some(mode)).unwrap_err();
        assert!(refused.contains("not restoring"), "{refused}");
    }

    #[test]
    fn a_chip_without_an_ownership_file_is_left_to_software_without_a_switch() {
        // No `pwm<N>_enable` at all: the kernel does not model ownership for this
        // chip, so there is nothing to switch and nothing to restore.
        let (_dir, fan) = writable_fixture(None);
        assert_eq!(fan.take_control().unwrap(), None);
        assert!(fan.restore_control(None).is_ok());
    }

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
        // A value that is not a number is ownership we could not read, and it is
        // deliberately *not* restorable: writing back a guess would decide who owns
        // a fan.
        let bogus = ControlMode::from_sysfs("bogus\n");
        assert!(matches!(bogus, ControlMode::Unknown(..)), "{bogus:?}");
        assert_eq!(bogus.restorable_value(), None);
        assert!(
            bogus.describe().contains("unreadable"),
            "{}",
            bogus.describe()
        );
    }
}
