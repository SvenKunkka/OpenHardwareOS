//! The OpenHardwareOS hardware runtime.
//!
//! Responsibilities, in the order they matter:
//!
//! 1. **Discover** hardware through adapters and detect hotplug.
//! 2. **Read** state on a cadence and publish changes on the event bus.
//! 3. **Write** actuator values — but only through range validation, the safety
//!    policy and the audit log.
//! 4. **Persist** user settings and the audit trail, locally only.
//!
//! Everything is decoupled: the UI and the automation engine both see
//! [`Runtime`], never an adapter, and both consume [`RuntimeEvent`]s.
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::Arc;
//! use ohm_adapter_api::HardwareAdapter;
//! use ohm_runtime::{ConfigPaths, Runtime, Settings};
//!
//! let adapters: Vec<Arc<dyn HardwareAdapter>> = vec![];
//! let runtime = Runtime::new(ConfigPaths::discover()?, Settings::default(), adapters)?;
//! let snapshot = runtime.start().await?;
//! println!("{} devices", snapshot.devices.len());
//! # Ok(())
//! # }
//! ```

pub mod audit;
pub mod bus;
pub mod config;
pub mod device_table;
pub mod discovery;
pub mod registry;
pub mod release;
pub mod runtime;
pub mod safety;
pub mod service;
pub mod snapshot;
pub mod store;

pub use audit::{AuditLog, WriteOrigin, WriteReport};
pub use bus::{EventBus, RuntimeEvent};
pub use config::{Settings, SettingsStore, Theme};
pub use device_table::{DeviceRecord, DeviceTable, ReconcileDiff, Upsert};
pub use discovery::{AdapterResult, AdapterShutdown, DiscoveryManager, DiscoveryOutcome};
pub use ohm_core::ConfigPaths;
pub use registry::{
    BindingRole, CapabilityIndex, CapabilityRef, CapabilityRegistry, ResolvedTarget,
};
pub use release::{ControlRelease, ReleasedChannel, ShutdownFailure};
pub use runtime::{Runtime, group_by_type, online_devices};
pub use safety::{SafetyDecision, SafetyPolicy};
pub use service::{ServiceError, ServiceGuard, ServiceState};
pub use snapshot::{AdapterView, DeviceStatus, DeviceView, RuntimeSnapshot, RuntimeStats};
pub use store::{ReadingChange, Sample, StateStore, reading_epsilon, value_changed};
