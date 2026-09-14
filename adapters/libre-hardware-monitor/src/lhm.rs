//! Parsing LibreHardwareMonitor's JSON tree and its human readable values.
//!
//! LHM's built-in web server answers `GET /data.json` with a nested tree:
//!
//! ```json
//! {"id":"/","Text":"Sensor","Children":[
//!   {"id":"/cpu/0","Text":"AMD Ryzen 9 7950X","HardwareType":"CPU","Children":[
//!     {"id":"/cpu/0/temperature/0","Text":"CPU Package","Value":"68.2 °C",
//!      "Type":"Temperature","SensorId":"/cpu/0/temperature/0"},
//!     {"id":"/lpc/nct6687d/control/0","Text":"Fan Control #1","Value":"50.0 %",
//!      "Type":"Control","SensorId":"/lpc/nct6687d/control/0"}
//!   ]}
//! ]}
//! ```
//!
//! Two things matter here and both are defensive:
//!
//! * the tree shape varies between LHM versions, so the walker only relies on
//!   `Children` and the sensor fields, never on a fixed nesting depth;
//! * values arrive as **display strings** (`"68.2 °C"`, `"1120 RPM"`, `"N/A"`),
//!   so every value is parsed through [`parse_value`], which returns `None`
//!   rather than a wrong number when it does not recognise the text.

use ohm_device_model::Unit;
use serde::Deserialize;
use serde_json::Value as Json;

/// One node of the LHM tree (hardware, group or sensor).
#[derive(Debug, Clone, PartialEq)]
pub struct LhmNode {
    /// LHM's internal id, e.g. `/lpc/nct6687d/control/0`. This is what
    /// `GET /Sensor?action=Set&id=...` expects, so it is the key for writes.
    pub id: String,
    /// Display name, e.g. `Fan Control #1`.
    pub text: String,
    /// `Temperature`, `Fan`, `Control`, `Load`, ... (`None` for hardware nodes).
    pub sensor_type: Option<String>,
    /// `CPU`, `GpuNvidia`, `Motherboard`, `SuperIO`, `Storage`, ...
    pub hardware_type: Option<String>,
    /// `Lpc`, `Motherboard`, ... for nested hardware.
    pub hardware_name: Option<String>,
    /// Parsed numeric value, when the node is a sensor with a readable value.
    pub value: Option<f64>,
    /// Unit implied by the value string.
    pub unit: Unit,
    /// Raw value string, kept for diagnostics.
    pub raw_value: Option<String>,
    pub children: Vec<LhmNode>,
}

impl LhmNode {
    /// Is this node a sensor (as opposed to a hardware container)?
    pub fn is_sensor(&self) -> bool {
        self.sensor_type.is_some()
    }

    /// `true` when the value string was `N/A` or empty.
    pub fn is_value_missing(&self) -> bool {
        self.raw_value.is_none()
    }

    /// Depth first walk over this node and its descendants.
    pub fn walk<'a>(&'a self, visit: &mut impl FnMut(&'a LhmNode, usize)) {
        self.walk_at(0, visit);
    }

    fn walk_at<'a>(&'a self, depth: usize, visit: &mut impl FnMut(&'a LhmNode, usize)) {
        visit(self, depth);
        for child in &self.children {
            child.walk_at(depth + 1, visit);
        }
    }

    /// Every node in the tree, depth first.
    pub fn flatten(&self) -> Vec<&LhmNode> {
        let mut out = Vec::new();
        self.walk(&mut |node, _| out.push(node));
        out
    }

    /// Every sensor under this node.
    pub fn sensors(&self) -> Vec<&LhmNode> {
        self.flatten()
            .into_iter()
            .filter(|n| n.is_sensor())
            .collect()
    }

    /// Sensors directly below this node.
    pub fn own_sensors(&self) -> Vec<&LhmNode> {
        self.children.iter().filter(|n| n.is_sensor()).collect()
    }

    /// Hardware children (nodes that carry a `HardwareType`).
    pub fn hardware_children(&self) -> Vec<&LhmNode> {
        self.children
            .iter()
            .filter(|n| n.hardware_type.is_some())
            .collect()
    }

    /// Effective hardware type, falling back to this node's own type.
    pub fn effective_hardware_type(&self) -> &str {
        self.hardware_type.as_deref().unwrap_or("")
    }

    /// Find a sensor by its LHM id anywhere in the tree.
    pub fn find_by_id(&self, id: &str) -> Option<&LhmNode> {
        self.flatten().into_iter().find(|node| node.id == id)
    }

    /// The trailing index of an LHM id (`/lpc/.../control/3` -> `3`).
    pub fn id_index(&self) -> Option<usize> {
        self.id.rsplit('/').next()?.parse().ok()
    }

    /// First number appearing in the display name (`Fan #12` -> `12`).
    pub fn name_index(&self) -> Option<usize> {
        let digits: String = self
            .text
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    }
}

/// Parse the raw JSON of `/data.json`.
pub fn parse_tree(json: &str) -> ohm_core::Result<LhmNode> {
    let value: Json = serde_json::from_str(json)?;
    Ok(convert(&value, 0))
}

fn convert(value: &Json, depth: usize) -> LhmNode {
    // A hostile or truncated document must not make us recurse forever.
    if depth > 32 {
        return LhmNode {
            id: String::new(),
            text: "<too deep>".into(),
            sensor_type: None,
            hardware_type: None,
            hardware_name: None,
            value: None,
            unit: Unit::None,
            raw_value: None,
            children: Vec::new(),
        };
    }

    let text = json_str(value, "Text")
        .or_else(|| json_str(value, "Name"))
        .unwrap_or_default();
    let id = json_str(value, "id")
        .or_else(|| json_str(value, "SensorId"))
        .unwrap_or_default();
    let sensor_type = json_str(value, "Type").filter(|t| !t.is_empty());
    let hardware_type = json_str(value, "HardwareType").filter(|t| !t.is_empty());
    let hardware_name =
        json_str(value, "HardwareId").map(|id| id.rsplit('/').next().unwrap_or(&id).to_string());
    let raw_value = json_str(value, "Value").filter(|v| !is_missing_text(v));
    let (parsed, unit) = match raw_value.as_deref() {
        Some(text) => match parse_value(text) {
            Some((number, unit)) => (Some(number), unit),
            None => (None, Unit::None),
        },
        None => (None, Unit::None),
    };

    let children = value
        .get("Children")
        .and_then(Json::as_array)
        .map(|children| {
            children
                .iter()
                .map(|child| convert(child, depth + 1))
                .collect()
        })
        .unwrap_or_default();

    LhmNode {
        id,
        text,
        sensor_type,
        hardware_type,
        hardware_name,
        value: parsed,
        unit,
        raw_value,
        children,
    }
}

fn json_str(value: &Json, key: &str) -> Option<String> {
    value.get(key).and_then(|v| match v {
        Json::String(text) => Some(text.clone()),
        Json::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

/// `true` for LHM's "no value" spellings.
pub fn is_missing_text(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("n/a")
        || trimmed.eq_ignore_ascii_case("na")
        || trimmed == "-"
        || trimmed.eq_ignore_ascii_case("not available")
}

/// Parse an LHM display value such as `"68.2 °C"`, `"1120 RPM"`, `"50.0 %"`.
///
/// Returns `None` when the text carries no usable number, so a missing sensor
/// becomes `unavailable` instead of a fake zero.
pub fn parse_value(text: &str) -> Option<(f64, Unit)> {
    let text = text.trim();
    if is_missing_text(text) {
        return None;
    }
    // LHM localises the decimal separator in some builds.
    let normalised = text.replace(',', ".");
    let number: String = normalised
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
        .collect();
    if number.is_empty() {
        return None;
    }
    let value: f64 = number.parse().ok()?;
    if !value.is_finite() {
        return None;
    }

    let lower = normalised.to_ascii_lowercase();
    let unit = if lower.contains("°c") || lower.contains("degc") || lower.contains("celsius") {
        Unit::Celsius
    } else if lower.contains("°f") || lower.contains("fahrenheit") {
        Unit::Fahrenheit
    } else if lower.contains("rpm") {
        Unit::Rpm
    } else if lower.contains('%') {
        Unit::Percent
    } else if lower.contains("mhz") {
        Unit::Megahertz
    } else if lower.contains("ghz") {
        // GHz is rendered as a small number; we keep the raw magnitude and let
        // the unit say MHz is not implied, so callers must convert.
        return Some((value * 1000.0, Unit::Megahertz));
    } else if lower.contains("hz") {
        Unit::Hertz
    } else if lower.contains("mw") {
        Unit::Milliwatt
    } else if lower.contains('w') {
        Unit::Watt
    } else if lower.contains("mv") || lower.contains('v') {
        Unit::Volt
    } else if lower.contains("ma") || lower.contains('a') {
        Unit::Ampere
    } else if lower.contains("gb") {
        return Some((value * 1_000_000_000.0, Unit::Byte));
    } else if lower.contains("mb") {
        return Some((value * 1_000_000.0, Unit::Byte));
    } else if lower.contains("kb") {
        return Some((value * 1_000.0, Unit::Byte));
    } else if lower.contains('b') {
        Unit::Byte
    } else {
        Unit::None
    };
    Some((value, unit))
}

/// Parse the small amount of JSON the web API returns for writes.
#[derive(Debug, Clone, Deserialize)]
pub struct LhmWriteAck {
    /// LHM answers with the resulting value string when the write succeeded.
    #[serde(default)]
    pub value: Option<String>,
}

/// Classify an LHM sensor type into the runtime's vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LhmSensorKind {
    Temperature,
    Fan,
    Control,
    Load,
    Power,
    Voltage,
    Current,
    Clock,
    Data,
    Other,
}

impl LhmSensorKind {
    pub fn from_type(sensor_type: &str) -> Self {
        match sensor_type.trim().to_ascii_lowercase().as_str() {
            "temperature" => Self::Temperature,
            "fan" => Self::Fan,
            "control" => Self::Control,
            "load" | "level" | "percentage" => Self::Load,
            "power" | "energy" => Self::Power,
            "voltage" => Self::Voltage,
            "current" => Self::Current,
            "clock" | "frequency" => Self::Clock,
            "data" | "smalldata" | "throughput" | "capacity" => Self::Data,
            _ => Self::Other,
        }
    }

    /// `true` for sensor types the runtime exposes as readings.
    pub fn is_readable(&self) -> bool {
        !matches!(self, Self::Other)
    }

    /// `true` for LHM `Control` sensors, which accept writes.
    pub fn is_writable(&self) -> bool {
        matches!(self, Self::Control)
    }

    /// Default unit when the value string did not carry one.
    pub fn default_unit(&self) -> Unit {
        match self {
            Self::Temperature => Unit::Celsius,
            Self::Fan => Unit::Rpm,
            Self::Control | Self::Load => Unit::Percent,
            Self::Power => Unit::Watt,
            Self::Voltage => Unit::Volt,
            Self::Current => Unit::Ampere,
            Self::Clock => Unit::Megahertz,
            Self::Data => Unit::Byte,
            Self::Other => Unit::None,
        }
    }
}

/// Map an LHM hardware type to the runtime's device type.
pub fn device_type_of(hardware_type: &str) -> ohm_device_model::DeviceType {
    use ohm_device_model::DeviceType as T;
    let normalised = hardware_type.trim().to_ascii_lowercase();
    match normalised.as_str() {
        "cpu" => T::Cpu,
        "gpunvidia" | "gpuamd" | "gpuintel" | "gpu" => T::Gpu,
        "motherboard" => T::Motherboard,
        "superio" | "lpc" | "embeddedcontroller" => T::Motherboard,
        "memory" | "ram" => T::Memory,
        "storage" | "nvme" | "hdd" => T::Storage,
        "network" => T::Unknown,
        "cooler" | "cooling" => T::Fan,
        _ => T::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../tests/fixtures/data.json");

    #[test]
    fn value_parsing_handles_every_lhm_spelling() {
        assert_eq!(parse_value("68.2 °C"), Some((68.2, Unit::Celsius)));
        assert_eq!(parse_value("68,2 °C"), Some((68.2, Unit::Celsius)));
        assert_eq!(parse_value("1120 RPM"), Some((1120.0, Unit::Rpm)));
        assert_eq!(parse_value("50.0 %"), Some((50.0, Unit::Percent)));
        assert_eq!(parse_value("405.2 W"), Some((405.2, Unit::Watt)));
        assert_eq!(parse_value("1.25 V"), Some((1.25, Unit::Volt)));
        assert_eq!(parse_value("4512 MHz"), Some((4512.0, Unit::Megahertz)));
        assert_eq!(parse_value("4.51 GHz"), Some((4510.0, Unit::Megahertz)));
        assert_eq!(parse_value("14.5 GB"), Some((14_500_000_000.0, Unit::Byte)));
        assert_eq!(parse_value("12.5 MB"), Some((12_500_000.0, Unit::Byte)));
        assert_eq!(parse_value("42"), Some((42.0, Unit::None)));
        assert_eq!(parse_value("-5.0 °C"), Some((-5.0, Unit::Celsius)));
    }

    #[test]
    fn missing_values_are_not_zero() {
        assert_eq!(parse_value("N/A"), None);
        assert_eq!(parse_value("n/a"), None);
        assert_eq!(parse_value(""), None);
        assert_eq!(parse_value("-"), None);
        assert_eq!(parse_value("Not Available"), None);
        assert!(is_missing_text(" N/A "));
        assert!(!is_missing_text("68 °C"));
    }

    #[test]
    fn tree_parsing_is_structure_tolerant() {
        let tree = parse_tree(SAMPLE).unwrap();
        assert_eq!(tree.text, "Sensor");
        assert!(!tree.children.is_empty());
        let sensors = tree.sensors();
        assert!(sensors.len() > 10, "expected a rich fixture");

        // Deeply nested sensors are still found.
        assert!(
            tree.find_by_id("/lpc/nct6687d/control/0")
                .is_some_and(|node| node.text == "Fan Control #1")
        );
        assert!(tree.find_by_id("/does/not/exist").is_none());
    }

    #[test]
    fn hardware_nodes_are_typed() {
        let tree = parse_tree(SAMPLE).unwrap();
        let types: Vec<&str> = tree
            .flatten()
            .into_iter()
            .filter_map(|node| node.hardware_type.as_deref())
            .collect();
        assert!(types.contains(&"CPU"));
        assert!(types.contains(&"GpuNvidia"));
        assert!(types.contains(&"Storage"));
    }

    #[test]
    fn node_helpers() {
        let tree = parse_tree(SAMPLE).unwrap();
        let control = tree.find_by_id("/lpc/nct6687d/control/0").unwrap();
        assert!(control.is_sensor());
        assert_eq!(control.sensor_type.as_deref(), Some("Control"));
        assert_eq!(control.id_index(), Some(0));
        assert_eq!(control.name_index(), Some(1));
        assert_eq!(control.value, Some(45.0));
        assert!(!control.is_value_missing());

        let missing = tree.find_by_id("/gpu/0/fan/0");
        if let Some(node) = missing {
            assert!(node.is_value_missing());
            assert_eq!(node.value, None);
        }

        let root_children = tree.hardware_children();
        assert!(!root_children.is_empty());
        assert!(
            tree.own_sensors().is_empty(),
            "the root only holds hardware"
        );
    }

    #[test]
    fn sensor_kinds_map_to_the_model() {
        assert_eq!(
            LhmSensorKind::from_type("Temperature"),
            LhmSensorKind::Temperature
        );
        assert_eq!(LhmSensorKind::from_type("control"), LhmSensorKind::Control);
        assert_eq!(LhmSensorKind::from_type("SmallData"), LhmSensorKind::Data);
        assert!(LhmSensorKind::Control.is_writable());
        assert!(!LhmSensorKind::Fan.is_writable());
        assert!(LhmSensorKind::Fan.is_readable());
        assert!(!LhmSensorKind::Other.is_readable());
        assert_eq!(LhmSensorKind::Temperature.default_unit(), Unit::Celsius);
        assert_eq!(LhmSensorKind::Control.default_unit(), Unit::Percent);
    }

    #[test]
    fn hardware_types_map_to_device_types() {
        use ohm_device_model::DeviceType as T;
        assert_eq!(device_type_of("CPU"), T::Cpu);
        assert_eq!(device_type_of("GpuNvidia"), T::Gpu);
        assert_eq!(device_type_of("SuperIO"), T::Motherboard);
        assert_eq!(device_type_of("Storage"), T::Storage);
        assert_eq!(device_type_of("Something new"), T::Unknown);
    }

    #[test]
    fn malformed_json_is_reported() {
        assert!(parse_tree("{not json").is_err());
        // An empty object is legal but yields an empty tree.
        let tree = parse_tree("{}").unwrap();
        assert!(tree.children.is_empty());
        assert_eq!(tree.text, "");
    }

    #[test]
    fn absurd_nesting_is_bounded() {
        // A hostile or corrupt document must not be able to exhaust the stack.
        let mut json = String::from(r#"{"Text":"leaf"}"#);
        for _ in 0..60 {
            json = format!(r#"{{"Text":"node","Children":[{json}]}}"#);
        }
        let tree = parse_tree(&json).unwrap();
        assert!(
            tree.flatten().len() < 40,
            "recursion must be bounded, got {}",
            tree.flatten().len()
        );
    }
}
