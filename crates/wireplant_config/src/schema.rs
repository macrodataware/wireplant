//! The scenario schema.
//!
//! Two kinds of struct live here, and they default differently.
//!
//! Config *sections* (`[network.modbus]`, `[historian]`, `[memory]`, ...) can be
//! left out entirely. Each one has a hand-written `Default` impl and carries
//! `#[serde(default)]` on the struct, so any missing key falls back to that one
//! impl. No `default = "fn"` attributes on their fields: a second source for
//! the same default is how a field ends up with two different defaults.
//!
//! *Records* (`[[tags]]`, `[[fault_profiles]]`, the root document) have keys
//! that must be present, like a tag's `id` or `address`, so they can't
//! implement a meaningful `Default`. Their optional keys get a plain
//! field-level `#[serde(default)]`, which uses the field type's own default.
//!
//! A combination with no sensible default — a protocol enabled on an empty
//! bind address — is a validation error, not a silently-applied guess.

use serde::{Deserialize, Serialize};
use wireplant_core::types::{Endianness, MemoryArea, TagDataType};
pub use wireplant_engine::alarm::{AlarmConfig, AlarmSeverity, AlarmType, IsaAlarmState};
pub use wireplant_engine::process::{
    DerivativeMode, PidConfig, ProcessModelSpec, TankModelConfig, ThermalModelConfig,
};
use wireplant_engine::waveforms::WaveformSpec;

use crate::errors::ConfigError;
use crate::validation::validate_scenario;

/// Top-level scenario root matching the TOML document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioConfig {
    pub facility: FacilityConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub network: NetworkConfig,
    pub tags: Vec<TagConfig>,
    #[serde(default)]
    pub processes: Vec<ProcessModelSpec>,
    #[serde(default)]
    pub fault_profiles: Vec<FaultProfile>,
    #[serde(default)]
    pub alarms: Vec<AlarmConfig>,
    #[serde(default)]
    pub historian: HistorianConfigSchema,
}

impl ScenarioConfig {
    /// Deserializes a TOML scenario string and validates all spans and cross-references.
    pub fn from_toml(s: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(s).map_err(|e| ConfigError::ParseError(e.to_string()))?;
        validate_scenario(&config)?;
        Ok(config)
    }

    /// Returns a reference to the tag configuration matching `id`, if present.
    pub fn tag(&self, id: &str) -> Option<&TagConfig> {
        self.tags.iter().find(|t| t.id == id)
    }

    /// Returns the index of the tag matching `id`, if present.
    pub fn tag_index(&self, id: &str) -> Option<usize> {
        self.tags.iter().position(|t| t.id == id)
    }
}

/// Facility-level metadata and global simulation parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FacilityConfig {
    pub name: String,
    pub description: Option<String>,
    #[serde(default = "FacilityConfig::default_tick_rate_ms")]
    pub tick_rate_ms: u64,
}

impl FacilityConfig {
    pub const DEFAULT_TICK_RATE_MS: u64 = 100;

    const fn default_tick_rate_ms() -> u64 {
        Self::DEFAULT_TICK_RATE_MS
    }
}

/// Modbus TCP server configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModbusConfig {
    pub enabled: bool,
    pub bind_addr: String,
    pub default_unit_id: u8,
    pub allowed_unit_ids: Vec<u8>,
    pub max_connections: usize,
    pub serial_enabled: bool,
    pub serial_port: Option<String>,
    pub baud_rate: u32,
    pub parity: String,
    pub data_bits: u8,
    pub stop_bits: u8,
}

/// User credentials for OPC-UA username/password authentication.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OpcUaUserConfig {
    pub username: String,
    pub password: String,
}

/// OPC-UA server configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OpcUaConfig {
    pub enabled: bool,
    pub bind_addr: String,
    pub allow_anonymous: bool,
    pub security_policies: Vec<String>,
    pub users: Vec<OpcUaUserConfig>,
    pub pki_dir: String,
    pub trust_client_certs: bool,
}

/// HTTP REST server configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HttpConfig {
    pub enabled: bool,
    pub bind_addr: String,
    pub api_key: Option<String>,
}

/// MQTT client configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MqttConfig {
    pub enabled: bool,
    pub broker_addr: String,
    /// Topic and client id prefix. Empty means "derive it from the facility name".
    pub client_id: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    /// Subscribe to incoming command topics. On by default (see the `Default` impl),
    /// so an enabled client accepts writes unless you opt out.
    pub subscribe_commands: bool,
    /// Custom command topic override. If None, defaults to wireplant/<facility_slug>/command.
    pub command_topic: Option<String>,
}

impl MqttConfig {
    /// Returns the configured client ID, or derives "wireplant_<facility_slug>"
    /// if none was specified.
    pub fn client_id_or_default(&self, facility_name: &str) -> String {
        self.client_id
            .clone()
            .unwrap_or_else(|| Self::default_client_id(facility_name))
    }

    /// Default client ID derived from a facility name.
    pub fn default_client_id(facility_name: &str) -> String {
        let clean = facility_name.replace(' ', "_").to_lowercase();
        format!("wireplant_{clean}")
    }
}

/// Server port bindings and external integration endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    pub modbus: ModbusConfig,
    pub opcua: OpcUaConfig,
    pub http: HttpConfig,
    pub mqtt: MqttConfig,
}

impl Default for ModbusConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind_addr: "0.0.0.0:5020".to_string(),
            default_unit_id: 1,
            allowed_unit_ids: Vec::new(),
            max_connections: 100,
            serial_enabled: false,
            serial_port: None,
            baud_rate: 9600,
            parity: "none".to_string(),
            data_bits: 8,
            stop_bits: 1,
        }
    }
}

impl Default for OpcUaConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind_addr: "0.0.0.0:4840".to_string(),
            allow_anonymous: true,
            security_policies: vec!["Basic256Sha256".to_string()],
            users: Vec::new(),
            pki_dir: "./pki".to_string(),
            trust_client_certs: false,
        }
    }
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind_addr: "0.0.0.0:8080".to_string(),
            api_key: None,
        }
    }
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            broker_addr: "127.0.0.1:1883".to_string(),
            client_id: None,
            username: None,
            password: None,
            subscribe_commands: true,
            command_topic: None,
        }
    }
}

/// Persistent time-series historian configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HistorianConfigSchema {
    pub enabled: bool,
    pub db_path: String,
    pub flush_interval_ms: u64,
    pub batch_size: usize,
    pub retention_hours: u32,
}

impl Default for HistorianConfigSchema {
    fn default() -> Self {
        Self {
            enabled: true,
            db_path: "wireplant_history.db".to_string(),
            flush_interval_ms: 1000,
            batch_size: 500,
            retention_hours: 48,
        }
    }
}

/// How much Process Image to allocate.
///
/// Both numbers exist so an undeclared address answers zero instead of an
/// out-of-range exception: `padding_registers` covers the address a tag's last
/// word reaches past, and `min_table_registers` covers a SCADA client that polls
/// ahead of the tag list. Raising the floor costs memory footprint and response
/// size on `GET /api/v1/state`, which returns the holding-register table whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryConfig {
    pub min_table_registers: usize,
    pub padding_registers: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            min_table_registers: 128,
            padding_registers: 8,
        }
    }
}

/// Predefined fault injection scenario profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaultProfile {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub actions: Vec<FaultAction>,
}

/// Action within a fault profile overriding a specific tag value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaultAction {
    pub target_tag: String,
    pub force_value: f64,
    /// Seconds the override stays in force; absent means the operator clears it.
    pub duration_secs: Option<f64>,
}

impl wireplant_core::codec::TagAddress for TagConfig {
    fn area(&self) -> MemoryArea {
        self.area
    }

    fn address(&self) -> usize {
        self.address as usize
    }

    fn data_type(&self) -> TagDataType {
        self.data_type
    }

    fn endianness(&self) -> Endianness {
        self.endianness
    }
}

/// Configuration schema for a single simulated industrial tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagConfig {
    pub id: String,
    pub description: Option<String>,
    pub area: MemoryArea,
    pub address: u16,
    pub data_type: TagDataType,
    #[serde(default)]
    pub endianness: Endianness,
    #[serde(default)]
    pub units: String,
    pub waveform: WaveformSpec,
}
