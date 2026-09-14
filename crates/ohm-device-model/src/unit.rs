//! Physical units understood by the device model.

use std::fmt;
use std::str::FromStr;

use ohm_core::{OhmError, Result};
use serde::{Deserialize, Serialize};

/// Unit of a [`Capability`](crate::Capability) or [`Value`](crate::Value).
///
/// The set is deliberately small: it only needs to cover what the automation
/// engine and the UI actually render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Celsius,
    Fahrenheit,
    Rpm,
    Percent,
    Pwm,
    Watt,
    Milliwatt,
    Volt,
    Ampere,
    Hertz,
    Megahertz,
    Byte,
    Second,
    Millisecond,
    Count,
    Boolean,
    Text,
    /// Dimensionless / not applicable.
    None,
}

impl Unit {
    /// Suffix used when rendering a value, including a leading space when one
    /// is needed.
    pub fn suffix(&self) -> &'static str {
        match self {
            Self::Celsius => " °C",
            Self::Fahrenheit => " °F",
            Self::Rpm => " rpm",
            Self::Percent => " %",
            Self::Pwm => " pwm",
            Self::Watt => " W",
            Self::Milliwatt => " mW",
            Self::Volt => " V",
            Self::Ampere => " A",
            Self::Hertz => " Hz",
            Self::Megahertz => " MHz",
            Self::Byte => " B",
            Self::Second => " s",
            Self::Millisecond => " ms",
            Self::Count | Self::None => "",
            Self::Boolean => "",
            Self::Text => "",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Celsius => "celsius",
            Self::Fahrenheit => "fahrenheit",
            Self::Rpm => "rpm",
            Self::Percent => "percent",
            Self::Pwm => "pwm",
            Self::Watt => "watt",
            Self::Milliwatt => "milliwatt",
            Self::Volt => "volt",
            Self::Ampere => "ampere",
            Self::Hertz => "hertz",
            Self::Megahertz => "megahertz",
            Self::Byte => "byte",
            Self::Second => "second",
            Self::Millisecond => "millisecond",
            Self::Count => "count",
            Self::Boolean => "boolean",
            Self::Text => "text",
            Self::None => "none",
        }
    }

    /// `true` for temperature units.
    pub fn is_temperature(&self) -> bool {
        matches!(self, Self::Celsius | Self::Fahrenheit)
    }

    /// `true` for rotational-speed units.
    pub fn is_rotational_speed(&self) -> bool {
        matches!(self, Self::Rpm | Self::Hertz)
    }

    /// `true` when the unit expresses a control duty rather than a measurement.
    pub fn is_duty(&self) -> bool {
        matches!(self, Self::Percent | Self::Pwm)
    }

    /// Convert a value expressed in this unit into Celsius, when meaningful.
    pub fn to_celsius(&self, value: f64) -> Option<f64> {
        match self {
            Self::Celsius => Some(value),
            Self::Fahrenheit => Some((value - 32.0) * 5.0 / 9.0),
            _ => None,
        }
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Unit {
    type Err = OhmError;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "c" | "celsius" | "°c" | "degc" => Ok(Self::Celsius),
            "f" | "fahrenheit" | "°f" | "degf" => Ok(Self::Fahrenheit),
            "rpm" => Ok(Self::Rpm),
            "percent" | "%" | "pct" => Ok(Self::Percent),
            "pwm" | "duty" => Ok(Self::Pwm),
            "w" | "watt" | "watts" => Ok(Self::Watt),
            "mw" | "milliwatt" => Ok(Self::Milliwatt),
            "v" | "volt" | "volts" => Ok(Self::Volt),
            "a" | "ampere" | "amp" | "amps" => Ok(Self::Ampere),
            "hz" | "hertz" => Ok(Self::Hertz),
            "mhz" | "megahertz" => Ok(Self::Megahertz),
            "b" | "byte" | "bytes" => Ok(Self::Byte),
            "s" | "second" | "seconds" => Ok(Self::Second),
            "ms" | "millisecond" => Ok(Self::Millisecond),
            "count" => Ok(Self::Count),
            "bool" | "boolean" => Ok(Self::Boolean),
            "text" | "string" => Ok(Self::Text),
            "none" | "" => Ok(Self::None),
            other => Err(OhmError::Config(format!("unknown unit `{other}`"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffixes() {
        assert_eq!(Unit::Celsius.suffix(), " °C");
        assert_eq!(Unit::Percent.suffix(), " %");
        assert_eq!(Unit::None.suffix(), "");
    }

    #[test]
    fn classification() {
        assert!(Unit::Celsius.is_temperature());
        assert!(Unit::Rpm.is_rotational_speed());
        assert!(Unit::Percent.is_duty());
        assert!(Unit::Pwm.is_duty());
        assert!(!Unit::Watt.is_temperature());
    }

    #[test]
    fn fahrenheit_conversion() {
        assert_eq!(Unit::Celsius.to_celsius(70.0), Some(70.0));
        let c = Unit::Fahrenheit.to_celsius(158.0).unwrap();
        assert!((c - 70.0).abs() < 1e-9);
        assert_eq!(Unit::Watt.to_celsius(1.0), None);
    }

    #[test]
    fn parse_and_serde() {
        assert_eq!("°C".parse::<Unit>().unwrap(), Unit::Celsius);
        assert_eq!("RPM".parse::<Unit>().unwrap(), Unit::Rpm);
        assert!("furlongs".parse::<Unit>().is_err());
        assert_eq!(
            serde_json::to_string(&Unit::Celsius).unwrap(),
            "\"celsius\""
        );
    }
}
