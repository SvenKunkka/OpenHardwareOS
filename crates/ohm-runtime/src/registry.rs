//! The capability registry.
//!
//! Adapters describe capabilities; this module turns that description into the
//! two things everything else needs:
//!
//! * **discovery for humans** — "which capabilities could drive a fan?", used
//!   to populate the Automation form and the device detail page.
//! * **resolution and validation** — "is `gpu.nvidia.0/temperature.core` a
//!   legal *source* for a curve?", used to reject a bad rule *before* it can
//!   touch hardware.
//!
//! This is the reason no business logic needs to know a vendor or a model.

use ohm_core::{AdapterId, CapabilityId, DeviceId, OhmError, Result};
use ohm_device_model::{Capability, CapabilityKind, Device, DeviceType};
use serde::{Deserialize, Serialize};

use crate::device_table::{DeviceRecord, DeviceTable};

/// A capability of a concrete device, flattened for the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityRef {
    pub device_id: DeviceId,
    pub device_name: String,
    pub device_type: DeviceType,
    pub adapter: AdapterId,
    pub capability: Capability,
    /// Last known value, when there is one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub current: Option<serde_json::Value>,
}

impl CapabilityRef {
    /// `gpu.nvidia.0/temperature.core` — the id a rule stores.
    pub fn qualified_id(&self) -> String {
        format!("{}/{}", self.device_id, self.capability.id)
    }

    /// Label for a dropdown, e.g. `NVIDIA GeForce RTX 5090 — Core Temperature`.
    pub fn label(&self) -> String {
        format!("{} — {}", self.device_name, self.capability.name)
    }
}

/// Source and target candidates for the automation editor.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CapabilityIndex {
    /// Every read-only measurement that could feed a rule.
    pub sources: Vec<CapabilityRef>,
    /// Every writable actuator a rule could drive.
    pub targets: Vec<CapabilityRef>,
}

impl CapabilityIndex {
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty() && self.targets.is_empty()
    }
}

/// A resolved device + capability pair.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTarget<'a> {
    pub record: &'a DeviceRecord,
    pub capability: &'a Capability,
}

impl ResolvedTarget<'_> {
    pub fn device(&self) -> &Device {
        &self.record.device
    }

    pub fn device_id(&self) -> &DeviceId {
        &self.record.device.id
    }

    pub fn capability_id(&self) -> &CapabilityId {
        &self.capability.id
    }
}

/// Read-only views over the device table.
#[derive(Debug, Clone, Copy)]
pub struct CapabilityRegistry;

impl CapabilityRegistry {
    /// Is this capability usable as an automation *source* (a numeric sensor)?
    pub fn is_source(capability: &Capability) -> bool {
        capability.kind.is_sensor() && capability.readable
    }

    /// Is this capability usable as an automation *target*?
    pub fn is_target(capability: &Capability) -> bool {
        capability.kind.is_actuator() && capability.writable
    }

    /// A temperature sensor, i.e. something a cooling curve can read.
    pub fn is_temperature_source(capability: &Capability) -> bool {
        Self::is_source(capability) && capability.unit.is_temperature()
    }

    /// A fan/pump duty control.
    pub fn is_cooling_target(capability: &Capability) -> bool {
        Self::is_target(capability) && capability.unit.is_duty()
    }

    /// Everything that could feed a rule: numeric, readable sensors.
    pub fn sources(table: &DeviceTable) -> Vec<CapabilityRef> {
        Self::collect(table, |r, c| {
            r.enabled && Self::is_source(c) && c.unit != ohm_device_model::Unit::Text
        })
    }

    /// Everything a rule could drive.
    pub fn targets(table: &DeviceTable) -> Vec<CapabilityRef> {
        Self::collect(table, |r, c| r.enabled && Self::is_target(c))
    }

    /// Temperature sensors only, ordered hottest-first is *not* guaranteed.
    pub fn temperature_sources(table: &DeviceTable) -> Vec<CapabilityRef> {
        Self::collect(table, |r, c| r.enabled && Self::is_temperature_source(c))
    }

    /// Duty controls only (fans, pumps).
    pub fn cooling_targets(table: &DeviceTable) -> Vec<CapabilityRef> {
        Self::collect(table, |r, c| r.enabled && Self::is_cooling_target(c))
    }

    fn collect(
        table: &DeviceTable,
        predicate: impl Fn(&DeviceRecord, &Capability) -> bool,
    ) -> Vec<CapabilityRef> {
        let mut out = Vec::new();
        for record in table.iter() {
            for capability in &record.device.capabilities {
                if predicate(record, capability) {
                    out.push(CapabilityRef {
                        device_id: record.device.id.clone(),
                        device_name: record.device.name.clone(),
                        device_type: record.device.device_type,
                        adapter: record.adapter.clone(),
                        capability: capability.clone(),
                        current: None,
                    });
                }
            }
        }
        out.sort_by_key(|entry| entry.qualified_id());
        out
    }

    /// The full index used by the Automation page.
    pub fn index(table: &DeviceTable) -> CapabilityIndex {
        CapabilityIndex {
            sources: Self::sources(table),
            targets: Self::targets(table),
        }
    }

    /// Resolve a `(device, capability)` pair, with a precise error when it does
    /// not exist.
    pub fn resolve<'a>(
        table: &'a DeviceTable,
        device: &str,
        capability: &str,
    ) -> Result<ResolvedTarget<'a>> {
        let record = table
            .get_str(device)
            .ok_or_else(|| OhmError::DeviceNotFound(device.to_string()))?;
        let capability = record.device.capability_str(capability).ok_or_else(|| {
            OhmError::CapabilityNotFound {
                device: device.to_string(),
                capability: capability.to_string(),
            }
        })?;
        Ok(ResolvedTarget { record, capability })
    }

    /// Resolve a pair that a rule wants to *read*.
    pub fn resolve_source<'a>(
        table: &'a DeviceTable,
        device: &str,
        capability: &str,
    ) -> Result<ResolvedTarget<'a>> {
        let target = Self::resolve(table, device, capability)?;
        if !Self::is_source(target.capability) {
            return Err(OhmError::Automation(format!(
                "`{device}/{capability}` is a {} capability and cannot be used as a source",
                target.capability.kind
            )));
        }
        Ok(target)
    }

    /// Resolve a pair that a rule wants to *write*.
    pub fn resolve_target<'a>(
        table: &'a DeviceTable,
        device: &str,
        capability: &str,
    ) -> Result<ResolvedTarget<'a>> {
        let target = Self::resolve(table, device, capability)?;
        if !target.capability.writable {
            return Err(OhmError::CapabilityNotWritable {
                device: device.to_string(),
                capability: capability.to_string(),
            });
        }
        if target.record.device.device_type.requires_safe_floor()
            && !target.capability.unit.is_duty()
        {
            tracing::debug!(
                device,
                capability,
                "controlling a fan/pump through a non duty capability"
            );
        }
        Ok(target)
    }

    /// Validate a rule binding with role specific, actionable messages.
    pub fn validate_binding(
        table: &DeviceTable,
        device: &str,
        capability: &str,
        role: BindingRole,
    ) -> Result<()> {
        match role {
            BindingRole::TemperatureSource => {
                let target = Self::resolve_source(table, device, capability)?;
                if !target.capability.unit.is_temperature() {
                    return Err(OhmError::Automation(format!(
                        "`{device}/{capability}` is a {} sensor; a cooling curve needs a temperature",
                        target.capability.unit
                    )));
                }
                Ok(())
            }
            BindingRole::Sensor => Self::resolve_source(table, device, capability).map(|_| ()),
            BindingRole::Actuator => Self::resolve_target(table, device, capability).map(|_| ()),
            BindingRole::CoolingActuator => {
                let target = Self::resolve_target(table, device, capability)?;
                if !target.capability.unit.is_duty() {
                    return Err(OhmError::Automation(format!(
                        "`{device}/{capability}` is not a duty control ({})",
                        target.capability.unit
                    )));
                }
                Ok(())
            }
        }
    }

    /// How many writable cooling actuators exist, i.e. can this machine be
    /// automated at all?
    pub fn controllable_cooling_count(table: &DeviceTable) -> usize {
        Self::cooling_targets(table).len()
    }

    /// Human summary used by the Overview page banner.
    pub fn describe(table: &DeviceTable) -> String {
        let sources = Self::temperature_sources(table).len();
        let targets = Self::cooling_targets(table).len();
        format!("{sources} temperature sources, {targets} controllable outputs")
    }
}

/// What a `(device, capability)` pair is being used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingRole {
    /// Must be a temperature sensor.
    TemperatureSource,
    /// Any readable capability.
    Sensor,
    /// Any writable capability.
    Actuator,
    /// Must be a duty control.
    CoolingActuator,
}

impl BindingRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TemperatureSource => "temperature_source",
            Self::Sensor => "sensor",
            Self::Actuator => "actuator",
            Self::CoolingActuator => "cooling_actuator",
        }
    }
}

/// Helper: is this kind/unit combination a fan or pump reading?
pub fn is_fan_reading(capability: &Capability) -> bool {
    capability.kind == CapabilityKind::Sensor && capability.unit.is_rotational_speed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use ohm_device_model::{Transport, Unit};

    fn gpu() -> Device {
        Device::new(
            DeviceId::new("gpu.mock.0").unwrap(),
            "Mock GPU",
            DeviceType::Gpu,
            Transport::Mock,
            AdapterId::new("mock").unwrap(),
        )
        .with_capability(Capability::sensor(
            "temperature.core",
            "Core Temperature",
            Unit::Celsius,
        ))
        .with_capability(Capability::sensor("load.gpu", "GPU Load", Unit::Percent))
    }

    fn fan() -> Device {
        Device::new(
            DeviceId::new("fan.mock.0").unwrap(),
            "Mock Fan",
            DeviceType::Fan,
            Transport::Mock,
            AdapterId::new("mock").unwrap(),
        )
        .with_capability(Capability::sensor("fan.rpm", "Fan RPM", Unit::Rpm))
        .with_capability(Capability::actuator(
            "fan.speed_percent",
            "Fan Speed",
            Unit::Percent,
            0.0,
            100.0,
        ))
    }

    fn table() -> DeviceTable {
        let mut table = DeviceTable::new();
        table.reconcile(
            &AdapterId::new("mock").unwrap(),
            vec![gpu(), fan()],
            &Settings::default(),
            1,
        );
        table
    }

    #[test]
    fn index_separates_sources_and_targets() {
        let index = CapabilityRegistry::index(&table());
        let sources: Vec<String> = index
            .sources
            .iter()
            .map(CapabilityRef::qualified_id)
            .collect();
        assert!(sources.iter().any(|s| s == "gpu.mock.0/temperature.core"));
        assert!(sources.iter().any(|s| s == "fan.mock.0/fan.rpm"));
        assert!(!sources.iter().any(|s| s.contains("fan.speed_percent")));
        assert_eq!(index.targets.len(), 1);
        assert_eq!(
            index.targets[0].qualified_id(),
            "fan.mock.0/fan.speed_percent"
        );
        assert_eq!(index.targets[0].label(), "Mock Fan — Fan Speed");
    }

    #[test]
    fn temperature_and_cooling_views() {
        let table = table();
        assert_eq!(CapabilityRegistry::temperature_sources(&table).len(), 1);
        assert_eq!(CapabilityRegistry::cooling_targets(&table).len(), 1);
        assert_eq!(CapabilityRegistry::controllable_cooling_count(&table), 1);
        assert_eq!(
            CapabilityRegistry::describe(&table),
            "1 temperature sources, 1 controllable outputs"
        );
    }

    #[test]
    fn resolution_reports_missing_pieces() {
        let table = table();
        let ok = CapabilityRegistry::resolve(&table, "gpu.mock.0", "temperature.core").unwrap();
        assert_eq!(ok.device_id().as_str(), "gpu.mock.0");
        assert_eq!(ok.capability_id().as_str(), "temperature.core");

        let err = CapabilityRegistry::resolve(&table, "nope.0", "temperature.core").unwrap_err();
        assert_eq!(err.code(), "device_not_found");
        let err = CapabilityRegistry::resolve(&table, "gpu.mock.0", "fan.rpm").unwrap_err();
        assert_eq!(err.code(), "capability_not_found");
    }

    #[test]
    fn role_validation_gives_actionable_errors() {
        let table = table();
        assert!(
            CapabilityRegistry::validate_binding(
                &table,
                "gpu.mock.0",
                "temperature.core",
                BindingRole::TemperatureSource
            )
            .is_ok()
        );
        // A tachometer reading is a legal source but not a temperature.
        let err = CapabilityRegistry::validate_binding(
            &table,
            "fan.mock.0",
            "fan.rpm",
            BindingRole::TemperatureSource,
        )
        .unwrap_err();
        assert!(err.to_string().contains("needs a temperature"), "{err}");
        // A capability that does not exist at all.
        assert_eq!(
            CapabilityRegistry::validate_binding(
                &table,
                "fan.mock.0",
                "fan.speed_percent_x",
                BindingRole::TemperatureSource,
            )
            .unwrap_err()
            .code(),
            "capability_not_found"
        );

        // A load sensor is a legal source but not a temperature.
        let err = CapabilityRegistry::validate_binding(
            &table,
            "gpu.mock.0",
            "load.gpu",
            BindingRole::TemperatureSource,
        )
        .unwrap_err();
        assert_eq!(err.code(), "automation_error");
        assert!(
            CapabilityRegistry::validate_binding(
                &table,
                "gpu.mock.0",
                "load.gpu",
                BindingRole::Sensor
            )
            .is_ok()
        );

        // Read-only capability cannot be a target.
        let err = CapabilityRegistry::validate_binding(
            &table,
            "fan.mock.0",
            "fan.rpm",
            BindingRole::Actuator,
        )
        .unwrap_err();
        assert_eq!(err.code(), "capability_read_only");

        assert!(
            CapabilityRegistry::validate_binding(
                &table,
                "fan.mock.0",
                "fan.speed_percent",
                BindingRole::CoolingActuator
            )
            .is_ok()
        );
        assert_eq!(BindingRole::CoolingActuator.as_str(), "cooling_actuator");
    }

    #[test]
    fn disabled_devices_drop_out_of_the_index() {
        let mut table = table();
        table
            .set_enabled(&DeviceId::new("fan.mock.0").unwrap(), false)
            .unwrap();
        let index = CapabilityRegistry::index(&table);
        assert!(index.targets.is_empty());
        assert_eq!(index.sources.len(), 2, "sources of the GPU remain");
    }

    #[test]
    fn classification_helpers() {
        let fan_cap = Capability::sensor("fan.rpm", "Fan RPM", Unit::Rpm);
        assert!(is_fan_reading(&fan_cap));
        assert!(CapabilityRegistry::is_source(&fan_cap));
        assert!(!CapabilityRegistry::is_target(&fan_cap));
        let duty =
            Capability::actuator("fan.speed_percent", "Fan Speed", Unit::Percent, 0.0, 100.0);
        assert!(CapabilityRegistry::is_cooling_target(&duty));
        assert!(!CapabilityRegistry::is_temperature_source(&duty));
    }
}
