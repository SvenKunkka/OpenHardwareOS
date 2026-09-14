//! HTTP client for LibreHardwareMonitor's built-in web server.
//!
//! This is the transport that works **today**, on any machine where the user
//! already runs LHM — which is the realistic path, because LHM is the tool that
//! owns the kernel driver and the SuperIO access that fan control needs.
//!
//! # Enabling it
//!
//! LHM ships the web server **disabled**. In the LHM GUI:
//! `Options -> Remote Web Server -> Run`, then note the port (default `8085`).
//! The setting lives in `LibreHardwareMonitor.config` next to the executable
//! (`runWebServerMenuItem`, `listenerPort`, `authenticationEnabled`).
//!
//! # Endpoints used
//!
//! | Purpose | Request |
//! |---------|---------|
//! | read everything | `GET /data.json` |
//! | set a control | `GET /Sensor?action=Set&id=<sensor id>&value=<percent>` |
//! | release control | `GET /Sensor?action=Set&id=<sensor id>&value=null` |
//!
//! `value=null` is LHM's `SetDefault`, i.e. "hand this channel back to the
//! firmware" — exactly what the runtime does on shutdown.
//!
//! The GUI process runs elevated, so the server itself is the privileged part;
//! OpenHardwareOS does not need Administrator to *talk* to it, only to start it.

use std::time::Duration;

use ohm_core::OhmError;
use ohm_device_model::UnavailableReason;
use serde::{Deserialize, Serialize};

use crate::lhm::{LhmNode, parse_tree};

/// Default port of LHM's web server.
pub const DEFAULT_PORT: u16 = 8085;
/// Default base URL.
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8085";

/// Where and how to reach the LHM web server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LhmConfig {
    /// Base URL, without a trailing slash.
    pub base_url: String,
    /// Request timeout.
    pub timeout_ms: u64,
    /// Optional HTTP basic auth user.
    pub username: Option<String>,
    /// Optional HTTP basic auth password.
    pub password: Option<String>,
    /// Path of the sensors endpoint.
    pub data_path: String,
    /// Path of the control endpoint.
    pub sensor_path: String,
}

impl Default for LhmConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout_ms: 3_000,
            username: None,
            password: None,
            data_path: "/data.json".to_string(),
            sensor_path: "/Sensor".to_string(),
        }
    }
}

impl LhmConfig {
    /// Read the configuration from a settings bag (`adapter_settings["lhm"]`).
    pub fn from_json(value: Option<&serde_json::Value>) -> Self {
        let mut config = Self::default();
        let Some(value) = value else {
            return config;
        };
        if let Some(url) = value.get("base_url").and_then(|v| v.as_str()) {
            config.base_url = url.trim_end_matches('/').to_string();
        }
        if let Some(timeout) = value.get("timeout_ms").and_then(|v| v.as_u64()) {
            config.timeout_ms = timeout.clamp(100, 60_000);
        }
        if let Some(user) = value.get("username").and_then(|v| v.as_str()) {
            config.username = Some(user.to_string());
        }
        if let Some(password) = value.get("password").and_then(|v| v.as_str()) {
            config.password = Some(password.to_string());
        }
        config
    }

    /// Full URL of the sensor tree endpoint.
    pub fn data_url(&self) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), self.data_path)
    }

    /// URL used to write a control channel.
    ///
    /// `value = None` sends `null`, which LHM treats as `SetDefault`.
    pub fn set_url(&self, sensor_id: &str, value: Option<f64>) -> String {
        let value = match value {
            Some(value) => format!("{value}"),
            None => "null".to_string(),
        };
        format!(
            "{}{}?action=Set&id={}&value={}",
            self.base_url.trim_end_matches('/'),
            self.sensor_path,
            urlencode(sensor_id),
            value
        )
    }

    /// URL used to read one sensor back (`GET /Sensor?action=Get&id=...`).
    pub fn get_url(&self, sensor_id: &str) -> String {
        format!(
            "{}{}?action=Get&id={}",
            self.base_url.trim_end_matches('/'),
            self.sensor_path,
            urlencode(sensor_id)
        )
    }
}

fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Failures specific to talking to LHM, mapped onto the runtime's vocabulary.
#[derive(Debug, Clone, PartialEq)]
pub enum LhmError {
    /// Nothing is listening: LHM is not running, or the web server is off.
    Unreachable(String),
    /// The server answered but rejected the request.
    Rejected { status: u16, body: String },
    /// The answer was not the JSON tree we expect.
    Malformed(String),
    /// The write was accepted but LHM refused the value.
    ControlRefused(String),
}

impl LhmError {
    pub fn reason(&self) -> UnavailableReason {
        match self {
            Self::Unreachable(_) => UnavailableReason::NotPresent,
            Self::Rejected { .. } => UnavailableReason::PermissionDenied,
            Self::Malformed(_) => UnavailableReason::ReadError,
            Self::ControlRefused(_) => UnavailableReason::VendorLimitation,
        }
    }

    /// A message that tells the user exactly what to do next.
    pub fn detail(&self, base_url: &str) -> String {
        match self {
            Self::Unreachable(detail) => format!(
                "could not reach the LibreHardwareMonitor web server at {base_url} ({detail}). \
                 Open LibreHardwareMonitor, choose Options -> Remote Web Server -> Run, and make \
                 sure the port matches (default {DEFAULT_PORT})."
            ),
            Self::Rejected { status, body } => format!(
                "LibreHardwareMonitor answered HTTP {status}: {}",
                body.trim().chars().take(200).collect::<String>()
            ),
            Self::Malformed(detail) => {
                format!("unexpected answer from LibreHardwareMonitor: {detail}")
            }
            Self::ControlRefused(detail) => format!(
                "LibreHardwareMonitor refused the write: {detail}. SuperIO fan control requires \
                 LHM to run as Administrator and the motherboard to expose a controllable \
                 channel."
            ),
        }
    }
}

impl From<LhmError> for OhmError {
    fn from(error: LhmError) -> Self {
        OhmError::Adapter {
            adapter: crate::ADAPTER_ID.to_string(),
            detail: error.detail(DEFAULT_BASE_URL),
        }
    }
}

/// Blocking HTTP client for one LHM instance.
#[derive(Debug, Clone)]
pub struct LhmWebClient {
    config: LhmConfig,
    agent: ureq::Agent,
}

impl LhmWebClient {
    pub fn new(config: LhmConfig) -> Self {
        let ureq_config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(config.timeout_ms)))
            .http_status_as_error(false)
            .build();
        Self {
            config,
            agent: ureq::Agent::new_with_config(ureq_config),
        }
    }

    pub fn config(&self) -> &LhmConfig {
        &self.config
    }

    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }

    fn credential(&self) -> Option<String> {
        match (&self.config.username, &self.config.password) {
            (Some(user), Some(password)) => Some(format!("{user}:{password}")),
            (Some(user), None) => Some(format!("{user}:")),
            _ => None,
        }
    }

    fn request(&self, url: &str) -> std::result::Result<(u16, String), LhmError> {
        let mut request = self.agent.get(url);
        if let Some(credential) = self.credential() {
            request = request.header(
                "Authorization",
                &format!("Basic {}", base64(credential.as_bytes())),
            );
        }
        let mut response = request
            .call()
            .map_err(|error| LhmError::Unreachable(error.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| LhmError::Malformed(error.to_string()))?;
        Ok((status, body))
    }

    /// Fetch and parse `/data.json`.
    pub fn fetch_tree(&self) -> std::result::Result<LhmNode, LhmError> {
        let url = self.config.data_url();
        let (status, body) = self.request(&url)?;
        if status == 401 || status == 403 {
            return Err(LhmError::Rejected { status, body });
        }
        if !(200..300).contains(&status) {
            return Err(LhmError::Rejected { status, body });
        }
        parse_tree(&body).map_err(|error| LhmError::Malformed(error.to_string()))
    }

    /// Write a control channel. `None` releases the channel back to the
    /// firmware (`SetDefault`).
    pub fn set_sensor(
        &self,
        sensor_id: &str,
        value: Option<f64>,
    ) -> std::result::Result<(), LhmError> {
        let url = self.config.set_url(sensor_id, value);
        let (status, body) = self.request(&url)?;
        if !(200..300).contains(&status) {
            return Err(LhmError::Rejected { status, body });
        }
        let trimmed = body.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("ok") {
            return Ok(());
        }
        // LHM answers with the raw value string on success, and with an error
        // message otherwise. `null`/`N/A` mean the channel refused the value.
        if trimmed.eq_ignore_ascii_case("null") || trimmed.eq_ignore_ascii_case("n/a") {
            return Err(LhmError::ControlRefused(format!(
                "the channel reported `{trimmed}` after the write"
            )));
        }
        Ok(())
    }

    /// Read one sensor back (`GET /Sensor?action=Get&id=...`).
    pub fn read_sensor(&self, sensor_id: &str) -> std::result::Result<Option<f64>, LhmError> {
        let url = self.config.get_url(sensor_id);
        let (status, body) = self.request(&url)?;
        if !(200..300).contains(&status) {
            return Err(LhmError::Rejected { status, body });
        }
        Ok(crate::lhm::parse_value(&body).map(|(value, _)| value))
    }
}

/// Minimal base64, so basic auth needs no extra dependency.
fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((triple >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(triple & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_built_correctly() {
        let config = LhmConfig::default();
        assert_eq!(config.data_url(), "http://127.0.0.1:8085/data.json");
        assert_eq!(
            config.set_url("/lpc/nct6687d/control/0", Some(55.0)),
            "http://127.0.0.1:8085/Sensor?action=Set&id=/lpc/nct6687d/control/0&value=55"
        );
        assert_eq!(
            config.set_url("/gpu/0/control/0", None),
            "http://127.0.0.1:8085/Sensor?action=Set&id=/gpu/0/control/0&value=null"
        );
        assert_eq!(
            config.get_url("/cpu/0/temperature/0"),
            "http://127.0.0.1:8085/Sensor?action=Get&id=/cpu/0/temperature/0"
        );
    }

    #[test]
    fn trailing_slashes_are_tolerated() {
        let config = LhmConfig {
            base_url: "http://localhost:9000/".into(),
            ..LhmConfig::default()
        };
        assert_eq!(config.data_url(), "http://localhost:9000/data.json");
    }

    #[test]
    fn config_from_settings_json() {
        let json = serde_json::json!({
            "base_url": "http://192.168.1.5:8085/",
            "timeout_ms": 500,
            "username": "admin",
            "password": "secret"
        });
        let config = LhmConfig::from_json(Some(&json));
        assert_eq!(config.base_url, "http://192.168.1.5:8085");
        assert_eq!(config.timeout_ms, 500);
        assert_eq!(config.username.as_deref(), Some("admin"));

        // Out of range and unknown keys are handled.
        let json = serde_json::json!({"timeout_ms": 5, "unrelated": true});
        let config = LhmConfig::from_json(Some(&json));
        assert_eq!(config.timeout_ms, 100);
        assert_eq!(LhmConfig::from_json(None), LhmConfig::default());
    }

    #[test]
    fn errors_explain_what_to_do() {
        let unreachable = LhmError::Unreachable("connection refused".into());
        let detail = unreachable.detail(DEFAULT_BASE_URL);
        assert!(detail.contains("Remote Web Server"));
        assert_eq!(unreachable.reason(), UnavailableReason::NotPresent);

        let refused = LhmError::ControlRefused("N/A".into());
        assert!(refused.detail(DEFAULT_BASE_URL).contains("Administrator"));
        assert_eq!(refused.reason(), UnavailableReason::VendorLimitation);

        assert_eq!(
            LhmError::Rejected {
                status: 401,
                body: "no".into()
            }
            .reason(),
            UnavailableReason::PermissionDenied
        );
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
    }

    #[test]
    fn url_encoding_keeps_path_separators() {
        assert_eq!(
            urlencode("/lpc/nct6687d/control/0"),
            "/lpc/nct6687d/control/0"
        );
        assert_eq!(urlencode("a b&c"), "a%20b%26c");
    }
}
