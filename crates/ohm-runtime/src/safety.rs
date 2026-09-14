//! Hardware protection.
//!
//! This is a Hardware OS, so the safety layer is not optional decoration: it is
//! a mandatory gate on the write path. Everything an adapter is asked to do
//! passes through [`SafetyPolicy::check_duty`] first.
//!
//! What it guarantees for the MVP:
//!
//! | Risk | Mitigation |
//! |------|------------|
//! | Fan driven to a dangerous low duty | `min_duty_percent` floor |
//! | Pump stopped | `pump_min_duty_percent`, never relaxed |
//! | Sensor lost while controlling | fail-safe duty (`fail_safe_duty_percent`) |
//! | Thermal runaway | `emergency_temp_c` overrides every rule at 100 % |
//! | Adapter write failure | fallback duty + error surfaced, never faked success |
//! | App exit while owning a fan | control released to firmware on shutdown |
//! | Silent hardware changes | every write is appended to the audit log |
//!
//! Safety is enforced *before* the adapter sees a value, and the automation
//! engine additionally consults [`SafetyPolicy::emergency_override`] on every
//! evaluation, so no rule can out-vote the emergency threshold.

use ohm_device_model::DeviceType;
use serde::{Deserialize, Serialize};

/// User configurable (but bounded) safety limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SafetyPolicy {
    /// Master switch. Turning this off still keeps the pump floor and the
    /// emergency ceiling active, because those protect hardware.
    pub enabled: bool,
    /// Enforce a minimum duty on fans.
    pub require_min_duty: bool,
    /// Lowest duty a fan may be driven to, in percent.
    pub min_duty_percent: f64,
    /// Lowest duty a pump may be driven to, in percent. Never zero.
    pub pump_min_duty_percent: f64,
    /// Temperature at which every controlled actuator is forced to
    /// `emergency_duty_percent`.
    pub emergency_temp_c: f64,
    /// Duty used during a thermal emergency, in percent.
    pub emergency_duty_percent: f64,
    /// Duty used when a rule loses its sensor or a write fails.
    pub fail_safe_duty_percent: f64,
    /// Let the emergency ceiling override automation. Default on.
    pub emergency_override_enabled: bool,
    /// Hand fan control back to the BIOS/firmware when the runtime stops.
    pub relinquish_on_exit: bool,
    /// Largest duty change allowed per write, in percent. `0` disables ramp
    /// limiting (default), because some fans stall when ramped too slowly.
    pub max_write_delta_percent: f64,
    /// Seconds a sensor may be stale before rules fall back. `0` disables the
    /// staleness check (rules then fall back on the first missing reading).
    pub sensor_stale_after_s: u64,
}

impl Default for SafetyPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            require_min_duty: true,
            min_duty_percent: 25.0,
            pump_min_duty_percent: 60.0,
            emergency_temp_c: 90.0,
            emergency_duty_percent: 100.0,
            fail_safe_duty_percent: 70.0,
            emergency_override_enabled: true,
            relinquish_on_exit: true,
            max_write_delta_percent: 0.0,
            sensor_stale_after_s: 5,
        }
    }
}

impl SafetyPolicy {
    /// Clamp every limit into a sane, non self-defeating range.
    pub fn sanitise(&mut self) {
        self.min_duty_percent = self.min_duty_percent.clamp(0.0, 100.0);
        self.pump_min_duty_percent = self.pump_min_duty_percent.clamp(10.0, 100.0);
        // A pump floor below the fan floor is a configuration mistake.
        if self.pump_min_duty_percent < self.min_duty_percent {
            self.pump_min_duty_percent = self.min_duty_percent.max(10.0);
        }
        self.emergency_duty_percent = self.emergency_duty_percent.clamp(50.0, 100.0);
        self.fail_safe_duty_percent = self.fail_safe_duty_percent.clamp(0.0, 100.0);
        if self.fail_safe_duty_percent < self.min_duty_percent {
            self.fail_safe_duty_percent = self.min_duty_percent;
        }
        self.emergency_temp_c = self.emergency_temp_c.clamp(50.0, 130.0);
        self.max_write_delta_percent = self.max_write_delta_percent.clamp(0.0, 100.0);
        self.sensor_stale_after_s = self.sensor_stale_after_s.min(600);
    }

    /// The duty floor for a device type.
    pub fn duty_floor(&self, device_type: DeviceType) -> f64 {
        match device_type {
            // A stopped pump is a dead CPU. The floor applies even when the
            // user disabled the generic minimum.
            DeviceType::Pump => self.pump_min_duty_percent.max(self.min_duty_percent),
            DeviceType::Fan if self.require_min_duty && self.enabled => self.min_duty_percent,
            _ => 0.0,
        }
    }

    /// Duty to use when a rule cannot compute a value any more.
    pub fn fail_safe_duty(&self, device_type: DeviceType) -> f64 {
        self.fail_safe_duty_percent
            .max(self.duty_floor(device_type))
    }

    /// Emergency duty for a device type.
    pub fn emergency_duty(&self, device_type: DeviceType) -> f64 {
        self.emergency_duty_percent
            .max(self.duty_floor(device_type))
    }

    /// Is this temperature over the emergency ceiling?
    pub fn is_emergency(&self, celsius: f64) -> bool {
        self.emergency_temp_c > 0.0 && celsius >= self.emergency_temp_c
    }

    /// If the hottest temperature is at/over the ceiling, return the duty that
    /// must be applied to every controlled actuator.
    pub fn emergency_override(&self, hottest_celsius: Option<f64>) -> Option<f64> {
        if !self.emergency_override_enabled {
            return None;
        }
        let hottest = hottest_celsius?;
        self.is_emergency(hottest)
            .then_some(self.emergency_duty_percent)
    }

    /// Decide what to do with a duty write. This is the single gate on the
    /// write path; the result is either an allowed (possibly clamped) value or
    /// an explicit block.
    pub fn check_duty(
        &self,
        device_type: DeviceType,
        requested: f64,
        current: Option<f64>,
        hottest_celsius: Option<f64>,
    ) -> SafetyDecision {
        if !requested.is_finite() {
            return SafetyDecision::Block {
                reason: format!("refusing to write a non finite duty ({requested})"),
            };
        }

        // 1. Thermal emergency wins over everything.
        if let Some(duty) = self.emergency_override(hottest_celsius) {
            let duty = self.emergency_duty(device_type).max(duty);
            if requested < duty {
                return SafetyDecision::Emergency {
                    value: duty,
                    reason: format!(
                        "{} °C is at or above the emergency threshold of {} °C",
                        hottest_celsius.unwrap_or_default(),
                        self.emergency_temp_c
                    ),
                };
            }
        }

        // 2. Duty floor (pump floor is unconditional).
        let floor = self.duty_floor(device_type);
        if requested < floor {
            return SafetyDecision::Allow {
                value: floor,
                clamped: Some(format!(
                    "raised to the {} % {} floor",
                    floor,
                    if device_type == DeviceType::Pump {
                        "pump"
                    } else {
                        "minimum fan"
                    }
                )),
            };
        }

        // 3. Optional ramp limiting.
        if let Some(current) = current.filter(|_| self.max_write_delta_percent > 0.0) {
            let delta = requested - current;
            if delta.abs() > self.max_write_delta_percent {
                let stepped = current + self.max_write_delta_percent * delta.signum();
                return SafetyDecision::Allow {
                    value: stepped.clamp(0.0, 100.0),
                    clamped: Some(format!(
                        "ramp limited to {} % per write",
                        self.max_write_delta_percent
                    )),
                };
            }
        }

        SafetyDecision::Allow {
            value: requested,
            clamped: None,
        }
    }
}

/// What the safety layer decided about a write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum SafetyDecision {
    /// Write may proceed, with this value.
    Allow {
        value: f64,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        clamped: Option<String>,
    },
    /// Thermal emergency: the value was raised to the emergency duty.
    Emergency { value: f64, reason: String },
    /// The write must not happen.
    Block { reason: String },
}

impl SafetyDecision {
    pub fn value(&self) -> Option<f64> {
        match self {
            Self::Allow { value, .. } | Self::Emergency { value, .. } => Some(*value),
            Self::Block { .. } => None,
        }
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Block { .. })
    }

    pub fn note(&self) -> Option<&str> {
        match self {
            Self::Allow { clamped, .. } => clamped.as_deref(),
            Self::Emergency { reason, .. } => Some(reason.as_str()),
            Self::Block { reason } => Some(reason.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohm_device_model::DeviceType::{Fan, Gpu, Pump};

    #[test]
    fn defaults_are_protective() {
        let policy = SafetyPolicy::default();
        assert!(policy.enabled);
        assert_eq!(policy.min_duty_percent, 25.0);
        assert!(policy.pump_min_duty_percent > policy.min_duty_percent);
        assert!(policy.fail_safe_duty_percent >= policy.min_duty_percent);
    }

    #[test]
    fn fan_floor_is_enforced_and_reported() {
        let policy = SafetyPolicy::default();
        let decision = policy.check_duty(Fan, 5.0, Some(40.0), Some(50.0));
        assert_eq!(decision.value(), Some(25.0));
        assert!(decision.note().unwrap().contains("floor"));
    }

    #[test]
    fn pump_can_never_be_stopped() {
        let policy = SafetyPolicy::default();
        assert_eq!(policy.duty_floor(Pump), 60.0);
        let decision = policy.check_duty(Pump, 0.0, Some(70.0), Some(40.0));
        assert_eq!(decision.value(), Some(60.0));

        // Even with the generic minimum disabled the pump floor applies.
        let relaxed = SafetyPolicy {
            enabled: false,
            require_min_duty: false,
            min_duty_percent: 0.0,
            pump_min_duty_percent: 60.0,
            ..Default::default()
        };
        assert_eq!(relaxed.duty_floor(Pump), 60.0);
        assert_eq!(relaxed.duty_floor(Fan), 0.0);
    }

    #[test]
    fn emergency_overrides_a_low_request() {
        let policy = SafetyPolicy::default();
        let decision = policy.check_duty(Fan, 30.0, Some(30.0), Some(95.0));
        match decision {
            SafetyDecision::Emergency { value, reason } => {
                assert_eq!(value, 100.0);
                assert!(reason.contains("95"));
            }
            other => panic!("expected emergency, got {other:?}"),
        }
    }

    #[test]
    fn emergency_does_not_lower_a_higher_request() {
        let policy = SafetyPolicy::default();
        let decision = policy.check_duty(Fan, 100.0, Some(80.0), Some(95.0));
        assert!(matches!(decision, SafetyDecision::Allow { value, .. } if value == 100.0));
    }

    #[test]
    fn normal_writes_pass_through() {
        let policy = SafetyPolicy::default();
        let decision = policy.check_duty(Fan, 55.0, Some(50.0), Some(60.0));
        assert_eq!(decision.value(), Some(55.0));
        assert!(decision.note().is_none());
        assert!(!decision.is_blocked());
    }

    #[test]
    fn nan_is_blocked() {
        let policy = SafetyPolicy::default();
        assert!(policy.check_duty(Fan, f64::NAN, None, None).is_blocked());
    }

    #[test]
    fn ramp_limiting_when_enabled() {
        let policy = SafetyPolicy {
            max_write_delta_percent: 10.0,
            ..Default::default()
        };
        let decision = policy.check_duty(Fan, 90.0, Some(50.0), Some(50.0));
        assert_eq!(decision.value(), Some(60.0));
        assert!(decision.note().unwrap().contains("ramp"));
        // Downwards too, and never below the floor.
        let decision = policy.check_duty(Fan, 25.0, Some(90.0), Some(50.0));
        assert_eq!(decision.value(), Some(80.0));
    }

    #[test]
    fn emergency_can_be_disabled() {
        let policy = SafetyPolicy {
            emergency_override_enabled: false,
            ..Default::default()
        };
        assert_eq!(policy.emergency_override(Some(120.0)), None);
        assert_eq!(policy.emergency_override(None), None);
    }

    #[test]
    fn sanitise_repairs_contradictory_limits() {
        let mut policy = SafetyPolicy {
            min_duty_percent: 80.0,
            pump_min_duty_percent: 20.0,
            fail_safe_duty_percent: 5.0,
            emergency_duty_percent: 10.0,
            emergency_temp_c: 500.0,
            ..Default::default()
        };
        policy.sanitise();
        assert_eq!(policy.pump_min_duty_percent, 80.0);
        assert_eq!(policy.fail_safe_duty_percent, 80.0);
        assert_eq!(policy.emergency_duty_percent, 50.0);
        assert_eq!(policy.emergency_temp_c, 130.0);
    }

    #[test]
    fn fan_floor_can_be_switched_off_but_pump_cannot() {
        let policy = SafetyPolicy {
            require_min_duty: false,
            ..Default::default()
        };
        assert_eq!(policy.duty_floor(Fan), 0.0);
        assert_eq!(policy.duty_floor(Gpu), 0.0);
        assert_eq!(policy.duty_floor(Pump), 60.0);
    }

    #[test]
    fn fail_safe_is_never_below_the_floor() {
        let policy = SafetyPolicy {
            fail_safe_duty_percent: 10.0,
            min_duty_percent: 30.0,
            ..Default::default()
        };
        assert_eq!(policy.fail_safe_duty(Fan), 30.0);
        assert_eq!(policy.fail_safe_duty(Pump), 60.0);
    }
}
