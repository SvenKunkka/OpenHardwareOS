//! Fan curves: piecewise linear mapping from a sensor value to an output.
//!
//! The curve is the *only* place where a temperature turns into a fan speed.
//! It is pure maths, which is why it is the most heavily unit-tested part of
//! the automation engine.

use ohm_core::{OhmError, Result};
use serde::{Deserialize, Serialize};

/// One point of a curve: `input` (e.g. °C) maps to `output` (e.g. %).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ControlPoint {
    pub input: f64,
    pub output: f64,
}

impl ControlPoint {
    pub fn new(input: f64, output: f64) -> Self {
        Self { input, output }
    }
}

/// A monotonic piecewise linear curve.
///
/// ```text
/// output
///  100 |                         *
///   80 |                    *
///   50 |              *
///   35 |         *
///   20 |    *
///      +----+----+----+----+---- input
///       40   60   70   80   85
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Vec<(f64, f64)>", into = "Vec<(f64, f64)>")]
pub struct Curve {
    points: Vec<ControlPoint>,
}

impl Curve {
    /// Build a curve, sorting points by input and rejecting duplicates.
    ///
    /// Returns an error when there are fewer than two points or when two points
    /// share the same input, because such a curve is ambiguous.
    pub fn new(points: impl IntoIterator<Item = (f64, f64)>) -> Result<Self> {
        let mut points: Vec<ControlPoint> = points
            .into_iter()
            .map(|(input, output)| ControlPoint::new(input, output))
            .collect();
        points.sort_by(|a, b| {
            a.input
                .partial_cmp(&b.input)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let curve = Self { points };
        curve.validate()?;
        Ok(curve)
    }

    /// Build from already ordered control points.
    pub fn from_points(points: Vec<ControlPoint>) -> Result<Self> {
        Self::new(points.into_iter().map(|p| (p.input, p.output)))
    }

    /// Build without validation, for tests and constants.
    ///
    /// # Panics
    /// Panics when the curve is invalid; use [`Curve::new`] when the data comes
    /// from a user.
    pub fn expect(points: impl IntoIterator<Item = (f64, f64)>) -> Self {
        Self::new(points).expect("valid curve")
    }

    /// A flat curve, handy as a fallback.
    pub fn flat(output: f64) -> Self {
        Self::expect([(0.0, output), (100.0, output)])
    }

    /// Validation rules shared by construction and by rule import.
    pub fn validate(&self) -> Result<()> {
        if self.points.len() < 2 {
            return Err(OhmError::Automation(
                "a curve needs at least two points".into(),
            ));
        }
        for point in &self.points {
            if !point.input.is_finite() || !point.output.is_finite() {
                return Err(OhmError::Automation(
                    "curve points must be finite numbers".into(),
                ));
            }
        }
        for pair in self.points.windows(2) {
            if pair[1].input <= pair[0].input {
                return Err(OhmError::Automation(format!(
                    "curve inputs must strictly increase ({} then {})",
                    pair[0].input, pair[1].input
                )));
            }
        }
        Ok(())
    }

    pub fn points(&self) -> &[ControlPoint] {
        &self.points
    }

    pub fn as_tuples(&self) -> Vec<(f64, f64)> {
        self.points.iter().map(|p| (p.input, p.output)).collect()
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn min_input(&self) -> f64 {
        self.points.first().map(|p| p.input).unwrap_or(0.0)
    }

    pub fn max_input(&self) -> f64 {
        self.points.last().map(|p| p.input).unwrap_or(0.0)
    }

    pub fn min_output(&self) -> f64 {
        self.points
            .iter()
            .map(|p| p.output)
            .fold(f64::INFINITY, f64::min)
    }

    pub fn max_output(&self) -> f64 {
        self.points
            .iter()
            .map(|p| p.output)
            .fold(f64::NEG_INFINITY, f64::max)
    }

    /// Evaluate the curve.
    ///
    /// * below the first point: the first output (never extrapolate downwards)
    /// * above the last point: the last output
    /// * strictly between two points: linear interpolation
    ///
    /// Non finite inputs return the minimum output, so a broken sensor can never
    /// command a fan to full speed by accident.
    pub fn eval(&self, input: f64) -> f64 {
        if !input.is_finite() {
            return self.min_output();
        }
        let first = self.points[0];
        if input <= first.input {
            return first.output;
        }
        let last = self.points[self.points.len() - 1];
        if input >= last.input {
            return last.output;
        }
        for pair in self.points.windows(2) {
            let (low, high) = (pair[0], pair[1]);
            if input >= low.input && input <= high.input {
                let span = high.input - low.input;
                if span <= 0.0 {
                    return high.output;
                }
                let t = (input - low.input) / span;
                return low.output + (high.output - low.output) * t;
            }
        }
        last.output
    }

    /// The input at which the curve reaches `output` (first crossing), used by
    /// the UI to draw the rule's trigger points.
    pub fn input_for_output(&self, output: f64) -> Option<f64> {
        for pair in self.points.windows(2) {
            let (low, high) = (pair[0], pair[1]);
            let (lo_out, hi_out) = (low.output, high.output);
            let within = (output >= lo_out.min(hi_out)) && (output <= lo_out.max(hi_out));
            if within && (hi_out - lo_out).abs() > f64::EPSILON {
                let t = (output - lo_out) / (hi_out - lo_out);
                return Some(low.input + (high.input - low.input) * t);
            }
        }
        None
    }
}

impl TryFrom<Vec<(f64, f64)>> for Curve {
    type Error = OhmError;

    fn try_from(points: Vec<(f64, f64)>) -> Result<Self> {
        Self::new(points)
    }
}

impl From<Curve> for Vec<(f64, f64)> {
    fn from(curve: Curve) -> Self {
        curve.as_tuples()
    }
}

/// The curve used by the documentation and by the default "GPU Cooling" rule.
pub fn gpu_cooling_curve() -> Curve {
    Curve::expect([
        (40.0, 20.0),
        (60.0, 35.0),
        (70.0, 50.0),
        (80.0, 80.0),
        (85.0, 100.0),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_is_linear() {
        let curve = gpu_cooling_curve();
        assert_eq!(curve.eval(40.0), 20.0);
        assert_eq!(curve.eval(60.0), 35.0);
        assert_eq!(curve.eval(70.0), 50.0);
        assert_eq!(curve.eval(80.0), 80.0);
        assert_eq!(curve.eval(85.0), 100.0);
        // Halfway between 60 and 70.
        assert!((curve.eval(65.0) - 42.5).abs() < 1e-9);
        // Halfway between 40 and 60.
        assert!((curve.eval(50.0) - 27.5).abs() < 1e-9);
    }

    #[test]
    fn out_of_range_inputs_clamp_to_the_ends() {
        let curve = gpu_cooling_curve();
        assert_eq!(curve.eval(-40.0), 20.0);
        assert_eq!(curve.eval(0.0), 20.0);
        assert_eq!(curve.eval(200.0), 100.0);
        // A broken reading (NaN, infinity) must never command full speed: it
        // maps to the minimum, and the rule's fallback handles it explicitly.
        assert_eq!(curve.eval(f64::NAN), curve.min_output());
        assert_eq!(curve.eval(f64::INFINITY), curve.min_output());
        assert_eq!(curve.eval(f64::NEG_INFINITY), curve.min_output());
    }

    #[test]
    fn points_are_sorted_and_bounds_reported() {
        let curve = Curve::expect([(80.0, 80.0), (40.0, 20.0), (60.0, 35.0)]);
        assert_eq!(curve.points()[0], ControlPoint::new(40.0, 20.0));
        assert_eq!(curve.min_input(), 40.0);
        assert_eq!(curve.max_input(), 80.0);
        assert_eq!(curve.min_output(), 20.0);
        assert_eq!(curve.max_output(), 80.0);
        assert_eq!(curve.len(), 3);
        assert!(!curve.is_empty());
    }

    #[test]
    fn invalid_curves_are_rejected() {
        assert!(Curve::new([(40.0, 20.0)]).is_err());
        assert!(Curve::new(Vec::<(f64, f64)>::new()).is_err());
        assert!(Curve::new([(40.0, 20.0), (40.0, 30.0)]).is_err());
        assert!(Curve::new([(40.0, 20.0), (f64::NAN, 30.0)]).is_err());
        // Points are sorted for the user, but a repeated input is ambiguous.
        assert!(Curve::new([(60.0, 20.0), (40.0, 30.0), (50.0, 40.0)]).is_ok());
        let err = Curve::new([(60.0, 20.0), (40.0, 30.0), (60.0, 40.0)]).unwrap_err();
        assert!(err.to_string().contains("strictly increase"), "{err}");
    }

    #[test]
    fn serde_roundtrip_uses_pairs() {
        let curve = gpu_cooling_curve();
        let yaml = serde_yaml_ng::to_string(&curve).unwrap();
        assert!(yaml.contains("- - 40.0"), "unexpected yaml:\n{yaml}");
        let back: Curve = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(back, curve);

        let json = serde_json::to_value(&curve).unwrap();
        assert_eq!(json[0][0], 40.0);
        assert_eq!(json[0][1], 20.0);
    }

    #[test]
    fn deserialising_an_invalid_curve_fails() {
        let result: Result<Curve, _> = serde_yaml_ng::from_str("- [40, 20]");
        assert!(result.is_err(), "one point is not a curve");
    }

    #[test]
    fn flat_curve_and_reverse_lookup() {
        let flat = Curve::flat(70.0);
        assert_eq!(flat.eval(10.0), 70.0);
        assert_eq!(flat.eval(90.0), 70.0);

        let curve = gpu_cooling_curve();
        let trigger = curve.input_for_output(35.0).unwrap();
        assert!((trigger - 60.0).abs() < 1e-9);
        assert!(curve.input_for_output(999.0).is_none());
    }

    #[test]
    fn non_monotonic_output_is_allowed() {
        // Outputs may go down as well as up; only inputs must be ordered.
        let curve = Curve::expect([(30.0, 0.0), (50.0, 100.0), (90.0, 40.0)]);
        assert_eq!(curve.eval(50.0), 100.0);
        assert_eq!(curve.eval(70.0), 70.0);
    }
}
