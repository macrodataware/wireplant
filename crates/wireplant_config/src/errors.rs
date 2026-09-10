use thiserror::Error;
use wireplant_core::types::MemoryArea;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    #[error("Failed to parse TOML configuration: {0}")]
    ParseError(String),
    #[error("Invalid facility setting: {0}")]
    InvalidFacility(String),
    #[error("Invalid tick rate {0}ms: must be between 1 and 10000ms")]
    InvalidTickRate(u64),
    #[error("Duplicate tag ID detected: '{0}'")]
    DuplicateTagId(String),
    #[error("Register collision in area {area:?} at address {address}: occupied by '{tag_a}' and '{tag_b}'")]
    RegisterCollision {
        area: MemoryArea,
        address: u16,
        tag_a: String,
        tag_b: String,
    },
    #[error("Address overflow calculating span for tag '{0}'")]
    AddressOverflow(String),
    #[error("Tag '{dependent_tag}' references nonexistent source tag '{missing_source}'")]
    DanglingTagReference {
        dependent_tag: String,
        missing_source: String,
    },
    #[error("Process '{process_type}' references nonexistent tag '{missing_tag}'")]
    InvalidProcessTagReference {
        process_type: String,
        missing_tag: String,
    },
    #[error("Operator input tag '{tag_id}' in process '{process_type}' has non-writable memory area {area:?}: must be HoldingRegister or Coil")]
    InvalidOperatorInputArea {
        process_type: String,
        tag_id: String,
        area: MemoryArea,
    },
    #[error("Fault profile '{profile_id}' references nonexistent target tag '{missing_tag}'")]
    InvalidFaultTagReference {
        profile_id: String,
        missing_tag: String,
    },
    #[error("{0} is enabled but has no bind address")]
    EmptyBindAddress(String),
    #[error("{protocol} endpoint '{addr}' is unusable: {reason}")]
    InvalidEndpoint {
        protocol: String,
        addr: String,
        reason: String,
    },
    #[error("Invalid memory sizing: {0}")]
    InvalidMemorySizing(&'static str),
    #[error("Tag '{tag_id}': {data_type:?} cannot live in area {area:?}")]
    DataTypeNotInArea {
        tag_id: String,
        area: MemoryArea,
        data_type: wireplant_core::types::TagDataType,
    },
    #[error("Tag '{tag_id}' has invalid waveform parameter: {reason}")]
    InvalidWaveform { tag_id: String, reason: String },
    #[error(
        "Process '{process_type}' output tag '{tag_id}' must be in a register area, got {area:?}"
    )]
    InvalidProcessOutputArea {
        process_type: String,
        tag_id: String,
        area: MemoryArea,
    },
    #[error("Alarm '{alarm_id}' has invalid parameter: {reason}")]
    InvalidAlarm { alarm_id: String, reason: String },
    #[error("Duplicate alarm ID detected: '{0}'")]
    DuplicateAlarmId(String),
    #[error("Process '{process_type}' has invalid configuration: {reason}")]
    InvalidProcessConfig {
        process_type: String,
        reason: String,
    },
    #[error(
        "Non-finite numeric value in {context} for parameter '{field}': must be a finite number"
    )]
    NonFiniteParameter { context: String, field: String },
    #[error("Invalid serial configuration parameter '{field}': {reason}")]
    InvalidSerialParameter { field: String, reason: String },
    #[error("Unsupported OPC-UA security policy '{policy}': supported policies are 'None', 'Basic256Sha256', 'Aes128Sha256RsaOaep'")]
    InvalidSecurityPolicy { policy: String },
    #[error("OPC-UA server has no usable endpoints: anonymous access is disabled and no security policies with valid users are configured")]
    NoUsableOpcUaEndpoint,
}
