//! Numeric and enumerable values that flow through the runtime.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::unit::Unit;

/// A single hardware value.
///
/// Serialised untagged so the JSON on the wire (and in the UI) looks natural:
/// `76.0`, `1120`, `true`, `"auto"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    /// Whole numbers first so `42` round-trips as an integer.
    Integer(i64),
    Number(f64),
    Bool(bool),
    Text(String),
}

impl Value {
    /// Numeric view of the value, used by the automation engine.
    ///
    /// Booleans map to `1.0` / `0.0`, text has no numeric view.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Integer(v) => Some(*v as f64),
            Self::Number(v) => Some(*v),
            Self::Bool(v) => Some(if *v { 1.0 } else { 0.0 }),
            Self::Text(_) => None,
        }
    }

    pub fn is_numeric(&self) -> bool {
        !matches!(self, Self::Text(_))
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(v) => Some(*v),
            Self::Integer(v) => Some(*v != 0),
            Self::Number(v) => Some(*v != 0.0),
            Self::Text(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(v) => Some(v),
            _ => None,
        }
    }

    /// Human readable rendering including the unit symbol.
    pub fn display(&self, unit: Unit) -> String {
        match self {
            Self::Integer(v) => format!("{v}{}", unit.suffix()),
            Self::Number(v) => format!("{}{}", trim_float(*v), unit.suffix()),
            Self::Bool(v) => v.to_string(),
            Self::Text(v) => v.clone(),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(v) => write!(f, "{v}"),
            Self::Number(v) => f.write_str(&trim_float(*v)),
            Self::Bool(v) => write!(f, "{v}"),
            Self::Text(v) => f.write_str(v),
        }
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

impl From<u32> for Value {
    fn from(value: u32) -> Self {
        Self::Integer(i64::from(value))
    }
}

macro_rules! value_from_int {
    ($($t:ty),*) => {
        $(
            impl From<$t> for Value {
                fn from(value: $t) -> Self {
                    Self::Integer(i64::from(value))
                }
            }
        )*
    };
}

value_from_int!(i8, i16, i32, u8, u16);

impl From<u64> for Value {
    fn from(value: u64) -> Self {
        match i64::try_from(value) {
            Ok(v) => Self::Integer(v),
            Err(_) => Self::Number(value as f64),
        }
    }
}

impl From<usize> for Value {
    fn from(value: usize) -> Self {
        Self::from(value as u64)
    }
}

impl From<f32> for Value {
    fn from(value: f32) -> Self {
        Self::Number(f64::from(value))
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

/// Render an `f64` without a trailing `.0` or float noise.
pub fn trim_float(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    if (value - value.round()).abs() < f64::EPSILON && value.abs() < 1e15 {
        format!("{}", value.round() as i64)
    } else {
        let s = format!("{value:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_stay_integers() {
        let v: Value = serde_json::from_str("1120").unwrap();
        assert_eq!(v, Value::Integer(1120));
        assert_eq!(v.as_f64(), Some(1120.0));
    }

    #[test]
    fn floats_stay_floats() {
        let v: Value = serde_json::from_str("76.5").unwrap();
        assert_eq!(v, Value::Number(76.5));
        assert_eq!(serde_json::to_string(&v).unwrap(), "76.5");
    }

    #[test]
    fn bool_and_text_roundtrip() {
        let b: Value = serde_json::from_str("true").unwrap();
        assert_eq!(b.as_bool(), Some(true));
        let t: Value = serde_json::from_str("\"auto\"").unwrap();
        assert_eq!(t.as_str(), Some("auto"));
        assert_eq!(t.as_f64(), None);
        assert!(!t.is_numeric());
    }

    #[test]
    fn display_uses_unit_suffix() {
        assert_eq!(Value::Integer(1120).display(Unit::Rpm), "1120 rpm");
        assert_eq!(Value::Number(68.0).display(Unit::Celsius), "68 °C");
        assert_eq!(Value::Number(42.25).display(Unit::Percent), "42.25 %");
    }

    #[test]
    fn float_trimming() {
        assert_eq!(trim_float(68.0), "68");
        assert_eq!(trim_float(68.5), "68.5");
        assert_eq!(trim_float(68.123456), "68.1235");
        assert_eq!(trim_float(f64::NAN), "NaN");
    }
}
