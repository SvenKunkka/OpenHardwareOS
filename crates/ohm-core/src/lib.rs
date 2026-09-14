//! Shared primitives for every OpenHardwareOS crate.
//!
//! `ohm-core` deliberately has no dependency on hardware, runtime or UI code. It
//! only provides the vocabulary that the rest of the workspace agrees on:
//! identifiers, errors, configuration paths, time helpers and logging setup.

pub mod error;
pub mod ids;
pub mod logging;
pub mod paths;
pub mod time;

pub use error::{OhmError, Result};
pub use ids::{AdapterId, CapabilityId, DeviceId, RuleId};
pub use paths::ConfigPaths;
pub use time::{now_ms, now_rfc3339, unix_ms_to_rfc3339};

/// Product identity, used by logs, the CLI banner and the UI about-box.
pub const PRODUCT_NAME: &str = "OpenHardwareOS";
/// Short product tag, used for the config directory name.
pub const PRODUCT_SLUG: &str = "OpenHardwareOS";
/// Version of the workspace as a whole.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Version of the Open Device Protocol implemented by the `ohm-protocol` crate.
///
/// Kept in `ohm-core` so that adapters can negotiate without depending on the
/// protocol crate itself.
pub const OPEN_DEVICE_PROTOCOL_VERSION: (u16, u16) = (0, 1);
