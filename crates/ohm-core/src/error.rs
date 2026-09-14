//! Error type shared by the whole workspace.

use std::path::{Path, PathBuf};

/// Convenience alias used across the project.
pub type Result<T, E = OhmError> = std::result::Result<T, E>;

/// Every failure mode the runtime knows about.
///
/// The variants are intentionally coarse: the UI groups them into user-facing
/// messages through [`OhmError::code`] and [`OhmError::hint`], while logs keep
/// the structured payload.
#[derive(Debug, thiserror::Error)]
pub enum OhmError {
    #[error("device `{0}` was not found")]
    DeviceNotFound(String),

    #[error("device `{device}` has no capability `{capability}`")]
    CapabilityNotFound { device: String, capability: String },

    #[error("capability `{device}/{capability}` is read-only")]
    CapabilityNotWritable { device: String, capability: String },

    #[error(
        "value {value} is outside the supported range {min}..={max} of `{device}/{capability}`"
    )]
    ValueOutOfRange {
        device: String,
        capability: String,
        value: f64,
        min: f64,
        max: f64,
    },

    #[error("value for `{device}/{capability}` is not a number: {detail}")]
    InvalidValue {
        device: String,
        capability: String,
        detail: String,
    },

    #[error("device `{0}` is disabled by the user")]
    DeviceDisabled(String),

    #[error("adapter `{adapter}` is unavailable: {detail}")]
    AdapterUnavailable { adapter: String, detail: String },

    #[error("adapter `{adapter}` failed: {detail}")]
    Adapter { adapter: String, detail: String },

    /// The hardware (or its driver) refuses the write. This is a normal,
    /// expected outcome on locked-down machines and must never be faked as a
    /// success.
    #[error("hardware refused the write to `{device}/{capability}`: {detail}")]
    WriteRejected {
        device: String,
        capability: String,
        detail: String,
    },

    /// The safety layer refused the write before it reached the hardware.
    #[error("safety policy blocked the write to `{device}/{capability}`: {detail}")]
    SafetyBlocked {
        device: String,
        capability: String,
        detail: String,
    },

    #[error("automation error: {0}")]
    Automation(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("invalid identifier `{value}`: {detail}")]
    InvalidId { value: String, detail: String },

    #[error("i/o error at `{}`: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to (de)serialize data: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

impl OhmError {
    /// Build an [`OhmError::Io`] with the offending path attached.
    pub fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::Other(message.into())
    }

    /// Stable machine readable code. Safe to match on in the UI and in tests.
    pub fn code(&self) -> &'static str {
        match self {
            Self::DeviceNotFound(_) => "device_not_found",
            Self::CapabilityNotFound { .. } => "capability_not_found",
            Self::CapabilityNotWritable { .. } => "capability_read_only",
            Self::ValueOutOfRange { .. } => "value_out_of_range",
            Self::InvalidValue { .. } => "invalid_value",
            Self::DeviceDisabled(_) => "device_disabled",
            Self::AdapterUnavailable { .. } => "adapter_unavailable",
            Self::Adapter { .. } => "adapter_error",
            Self::WriteRejected { .. } => "write_rejected",
            Self::SafetyBlocked { .. } => "safety_blocked",
            Self::Automation(_) => "automation_error",
            Self::Protocol(_) => "protocol_error",
            Self::Config(_) => "config_error",
            Self::InvalidId { .. } => "invalid_id",
            Self::Io { .. } => "io_error",
            Self::Serde(_) => "serde_error",
            Self::Other(_) => "error",
        }
    }

    /// Actionable one-liner for the desktop UI ("why can I not do this?").
    ///
    /// Hardware limitations are a first class citizen here: the MVP must show
    /// `unsupported` / `permission denied` / `hardware limitation` instead of
    /// pretending a fan was set.
    pub fn hint(&self) -> &'static str {
        match self {
            Self::CapabilityNotWritable { .. } => {
                "This capability is read-only on this machine. Fan control requires a \
                 motherboard SuperIO chip exposed through LibreHardwareMonitor."
            }
            Self::ValueOutOfRange { .. } => {
                "The value was clamped or rejected by the device range."
            }
            Self::DeviceDisabled(_) => "Enable the device in Devices or Settings to use it.",
            Self::AdapterUnavailable { .. } => {
                "The provider is not present on this system. Install or start the required \
                 component (for example LibreHardwareMonitor) and rescan."
            }
            Self::WriteRejected { .. } => {
                "The driver or firmware refused the write. Run as Administrator and make sure \
                 no vendor tool is currently owning the fan controller."
            }
            Self::SafetyBlocked { .. } => {
                "The safety policy blocked this write to protect the hardware."
            }
            Self::DeviceNotFound(_) => "Rescan devices; the device may have been unplugged.",
            _ => "See the log for details.",
        }
    }

    /// `true` when the failure means "the hardware cannot do this here", which
    /// the UI surfaces as an *unsupported* badge rather than a red error.
    pub fn is_unsupported(&self) -> bool {
        matches!(
            self,
            Self::CapabilityNotWritable { .. }
                | Self::AdapterUnavailable { .. }
                | Self::WriteRejected { .. }
        )
    }
}

impl From<std::io::Error> for OhmError {
    fn from(source: std::io::Error) -> Self {
        Self::Io {
            path: PathBuf::from("<unknown>"),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_stable() {
        let err = OhmError::DeviceNotFound("fan.system.0".into());
        assert_eq!(err.code(), "device_not_found");
        assert_eq!(err.to_string(), "device `fan.system.0` was not found");
    }

    #[test]
    fn unsupported_classification() {
        let err = OhmError::CapabilityNotWritable {
            device: "fan.0".into(),
            capability: "fan.speed_percent".into(),
        };
        assert!(err.is_unsupported());
        assert!(!err.hint().is_empty());

        let err = OhmError::Config("bad".into());
        assert!(!err.is_unsupported());
    }

    #[test]
    fn io_error_keeps_path() {
        let err = OhmError::io("/tmp/does-not-exist", std::io::Error::other("boom"));
        assert!(err.to_string().contains("/tmp/does-not-exist"));
        assert_eq!(err.code(), "io_error");
    }
}
