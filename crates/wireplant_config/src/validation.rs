use std::collections::{HashMap, HashSet};
use wireplant_core::types::{MemoryArea, TagDataType};
use wireplant_engine::waveforms::WaveformSpec;

use crate::errors::ConfigError;
use crate::schema::ScenarioConfig;

/// The words a tag occupies, or a refusal if its type cannot live in its area.
///
/// The bit areas are one bit wide no matter what, which is why a register type
/// aimed at a coil is caught here rather than silently truncated at the edge.
fn register_span(tag: &crate::schema::TagConfig) -> Result<u16, ConfigError> {
    let bits = matches!(tag.area, MemoryArea::Coil | MemoryArea::DiscreteInput);
    match (bits, tag.data_type) {
        (true, TagDataType::Boolean) | (false, _) => Ok(tag.data_type.register_span() as u16),
        (true, data_type) => Err(ConfigError::DataTypeNotInArea {
            tag_id: tag.id.clone(),
            area: tag.area,
            data_type,
        }),
    }
}

/// An enabled protocol needs somewhere to listen, and nothing listens on "".
fn validate_bind_addr(protocol: &str, enabled: bool, addr: &str) -> Result<(), ConfigError> {
    if !enabled {
        return Ok(());
    }
    if addr.trim().is_empty() {
        return Err(ConfigError::EmptyBindAddress(protocol.to_string()));
    }
    validate_host_port(protocol, addr)
}

fn validate_host_port(protocol: &str, addr: &str) -> Result<(), ConfigError> {
    wireplant_core::net::parse_host_port(addr, 0).map_err(|e| ConfigError::InvalidEndpoint {
        protocol: protocol.to_string(),
        addr: addr.to_string(),
        reason: e.to_string(),
    })?;
    Ok(())
}

/// Validates that an f64 value is strictly finite (not NaN, not +inf, not -inf).
fn require_finite(val: f64, field_name: &str, context: &str) -> Result<(), ConfigError> {
    if val.is_finite() {
        Ok(())
    } else {
        Err(ConfigError::NonFiniteParameter {
            context: context.to_string(),
            field: field_name.to_string(),
        })
    }
}

/// Validates that a slice of (value, field_name) pairs are all strictly finite.
fn require_all_finite(fields: &[(f64, &str)], context: &str) -> Result<(), ConfigError> {
    for &(val, name) in fields {
        require_finite(val, name, context)?;
    }
    Ok(())
}

/// Validates that an optional f64 value is strictly finite if present.
fn require_opt_finite(
    val: Option<f64>,
    field_name: &str,
    context: &str,
) -> Result<(), ConfigError> {
    if let Some(v) = val {
        require_finite(v, field_name, context)?;
    }
    Ok(())
}

fn validate_waveform(tag_id: &str, waveform: &WaveformSpec) -> Result<(), ConfigError> {
    let ctx = format!("tag '{tag_id}' waveform");
    match waveform {
        WaveformSpec::Constant { value } => {
            require_finite(*value, "value", &ctx)?;
        }
        WaveformSpec::Sine {
            min,
            max,
            period_secs,
        } => {
            require_all_finite(
                &[(*min, "min"), (*max, "max"), (*period_secs, "period_secs")],
                &ctx,
            )?;
            if *min >= *max {
                return Err(ConfigError::InvalidWaveform {
                    tag_id: tag_id.to_string(),
                    reason: format!("min ({min}) must be strictly less than max ({max})"),
                });
            }
            if *period_secs <= 0.0 {
                return Err(ConfigError::InvalidWaveform {
                    tag_id: tag_id.to_string(),
                    reason: format!("period_secs ({period_secs}) must be positive"),
                });
            }
        }
        WaveformSpec::Triangle {
            min,
            max,
            period_secs,
        } => {
            require_all_finite(
                &[(*min, "min"), (*max, "max"), (*period_secs, "period_secs")],
                &ctx,
            )?;
            if *min >= *max {
                return Err(ConfigError::InvalidWaveform {
                    tag_id: tag_id.to_string(),
                    reason: format!("min ({min}) must be strictly less than max ({max})"),
                });
            }
            if *period_secs <= 0.0 {
                return Err(ConfigError::InvalidWaveform {
                    tag_id: tag_id.to_string(),
                    reason: format!("period_secs ({period_secs}) must be positive"),
                });
            }
        }
        WaveformSpec::Ramp {
            min,
            max,
            step_per_tick,
        } => {
            require_all_finite(
                &[
                    (*min, "min"),
                    (*max, "max"),
                    (*step_per_tick, "step_per_tick"),
                ],
                &ctx,
            )?;
            if *min >= *max {
                return Err(ConfigError::InvalidWaveform {
                    tag_id: tag_id.to_string(),
                    reason: format!("min ({min}) must be strictly less than max ({max})"),
                });
            }
            if *step_per_tick <= 0.0 {
                return Err(ConfigError::InvalidWaveform {
                    tag_id: tag_id.to_string(),
                    reason: format!("step_per_tick ({step_per_tick}) must be positive"),
                });
            }
        }
        WaveformSpec::RandomWalk {
            center,
            max_jitter,
            drift_sigma,
        } => {
            require_all_finite(
                &[
                    (*center, "center"),
                    (*max_jitter, "max_jitter"),
                    (*drift_sigma, "drift_sigma"),
                ],
                &ctx,
            )?;
            if *max_jitter < 0.0 {
                return Err(ConfigError::InvalidWaveform {
                    tag_id: tag_id.to_string(),
                    reason: format!("max_jitter ({max_jitter}) cannot be negative"),
                });
            }
            if *drift_sigma < 0.0 {
                return Err(ConfigError::InvalidWaveform {
                    tag_id: tag_id.to_string(),
                    reason: format!("drift_sigma ({drift_sigma}) cannot be negative"),
                });
            }
        }
        WaveformSpec::TripThreshold { threshold, .. } => {
            require_finite(*threshold, "threshold", &ctx)?;
        }
    }
    Ok(())
}

fn check_tag_ref(
    process_type: &str,
    tag_id: &str,
    tag_areas: &HashMap<&str, MemoryArea>,
) -> Result<MemoryArea, ConfigError> {
    tag_areas
        .get(tag_id)
        .copied()
        .ok_or_else(|| ConfigError::InvalidProcessTagReference {
            process_type: process_type.to_string(),
            missing_tag: tag_id.to_string(),
        })
}

fn check_writable_input(
    process_type: &str,
    tag_id: &str,
    tag_areas: &HashMap<&str, MemoryArea>,
) -> Result<(), ConfigError> {
    let area = check_tag_ref(process_type, tag_id, tag_areas)?;
    if area != MemoryArea::HoldingRegister && area != MemoryArea::Coil {
        Err(ConfigError::InvalidOperatorInputArea {
            process_type: process_type.to_string(),
            tag_id: tag_id.to_string(),
            area,
        })
    } else {
        Ok(())
    }
}

fn check_register_output(
    process_type: &str,
    tag_id: &str,
    tag_areas: &HashMap<&str, MemoryArea>,
) -> Result<(), ConfigError> {
    let area = check_tag_ref(process_type, tag_id, tag_areas)?;
    if area != MemoryArea::HoldingRegister && area != MemoryArea::InputRegister {
        Err(ConfigError::InvalidProcessOutputArea {
            process_type: process_type.to_string(),
            tag_id: tag_id.to_string(),
            area,
        })
    } else {
        Ok(())
    }
}

/// Validates scenario semantics, checking address spans, collisions, and tag dependencies.
pub fn validate_scenario(config: &ScenarioConfig) -> Result<(), ConfigError> {
    if config.facility.name.trim().is_empty() {
        return Err(ConfigError::InvalidFacility(
            "Facility name cannot be empty".into(),
        ));
    }
    if config.facility.tick_rate_ms == 0 || config.facility.tick_rate_ms > 10_000 {
        return Err(ConfigError::InvalidTickRate(config.facility.tick_rate_ms));
    }
    if config.memory.min_table_registers == 0 {
        return Err(ConfigError::InvalidMemorySizing(
            "min_table_registers must be at least 1",
        ));
    }

    validate_bind_addr(
        "modbus",
        config.network.modbus.enabled,
        &config.network.modbus.bind_addr,
    )?;
    if config.network.modbus.serial_enabled {
        match &config.network.modbus.serial_port {
            Some(p) if !p.trim().is_empty() => {}
            _ => {
                return Err(ConfigError::EmptyBindAddress(
                    "modbus serial port".to_string(),
                ))
            }
        }
        let parity_str = config.network.modbus.parity.trim().to_lowercase();
        if parity_str != "none" && parity_str != "even" && parity_str != "odd" {
            return Err(ConfigError::InvalidSerialParameter {
                field: "parity".into(),
                reason: format!("expected 'none', 'even', or 'odd', got '{parity_str}'"),
            });
        }
        if config.network.modbus.data_bits != 7 && config.network.modbus.data_bits != 8 {
            return Err(ConfigError::InvalidSerialParameter {
                field: "data_bits".into(),
                reason: format!("expected 7 or 8, got {}", config.network.modbus.data_bits),
            });
        }
        if config.network.modbus.stop_bits != 1 && config.network.modbus.stop_bits != 2 {
            return Err(ConfigError::InvalidSerialParameter {
                field: "stop_bits".into(),
                reason: format!("expected 1 or 2, got {}", config.network.modbus.stop_bits),
            });
        }
        if config.network.modbus.baud_rate == 0 {
            return Err(ConfigError::InvalidSerialParameter {
                field: "baud_rate".into(),
                reason: "baud_rate must be greater than 0".into(),
            });
        }
    }
    validate_bind_addr(
        "opcua",
        config.network.opcua.enabled,
        &config.network.opcua.bind_addr,
    )?;
    if config.network.opcua.enabled {
        let mut supported_count = 0;
        for policy in &config.network.opcua.security_policies {
            if policy.eq_ignore_ascii_case("none")
                || policy.eq_ignore_ascii_case("basic256sha256")
                || policy.eq_ignore_ascii_case("aes128sha256rsaoaep")
            {
                supported_count += 1;
            } else {
                return Err(ConfigError::InvalidSecurityPolicy {
                    policy: policy.clone(),
                });
            }
        }
        if !config.network.opcua.allow_anonymous
            && (supported_count == 0 || config.network.opcua.users.is_empty())
        {
            return Err(ConfigError::NoUsableOpcUaEndpoint);
        }
    }
    validate_bind_addr(
        "http",
        config.network.http.enabled,
        &config.network.http.bind_addr,
    )?;
    if config.network.mqtt.enabled {
        validate_host_port("mqtt", &config.network.mqtt.broker_addr)?;
    }

    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut occupied_spans: HashMap<MemoryArea, HashMap<u16, String>> = HashMap::new();

    // Check unique tag IDs and allocate address spans
    for tag in &config.tags {
        if !seen_ids.insert(tag.id.clone()) {
            return Err(ConfigError::DuplicateTagId(tag.id.clone()));
        }

        validate_waveform(&tag.id, &tag.waveform)?;

        let span_len = register_span(tag)?;

        let area_map = occupied_spans.entry(tag.area).or_default();
        for offset in 0..span_len {
            let addr = tag
                .address
                .checked_add(offset)
                .ok_or_else(|| ConfigError::AddressOverflow(tag.id.clone()))?;
            if let Some(existing_tag) = area_map.insert(addr, tag.id.clone()) {
                return Err(ConfigError::RegisterCollision {
                    area: tag.area,
                    address: addr,
                    tag_a: existing_tag,
                    tag_b: tag.id.clone(),
                });
            }
        }
    }

    // Verify cross-tag trip threshold sources exist
    for tag in &config.tags {
        if let WaveformSpec::TripThreshold { source_tag, .. } = &tag.waveform {
            if !seen_ids.contains(source_tag) {
                return Err(ConfigError::DanglingTagReference {
                    dependent_tag: tag.id.clone(),
                    missing_source: source_tag.clone(),
                });
            }
        }
    }

    // Map tag ID to MemoryArea for fast lookup of memory areas
    let tag_areas: HashMap<&str, MemoryArea> = config
        .tags
        .iter()
        .map(|t| (t.id.as_str(), t.area))
        .collect();

    // Verify processes
    for process in &config.processes {
        match process {
            wireplant_engine::process::ProcessModelSpec::Tank {
                level_tag,
                inlet_valve_tag,
                drain_pump_tag,
                config: tank_cfg,
            } => {
                check_register_output("Tank", level_tag, &tag_areas)?;
                check_writable_input("Tank", inlet_valve_tag, &tag_areas)?;
                check_writable_input("Tank", drain_pump_tag, &tag_areas)?;

                let ctx = "Process Tank";
                require_all_finite(
                    &[
                        (tank_cfg.surface_area_m2, "surface_area_m2"),
                        (tank_cfg.max_height_m, "max_height_m"),
                        (tank_cfg.max_inflow_m3_s, "max_inflow_m3_s"),
                        (tank_cfg.max_outflow_m3_s, "max_outflow_m3_s"),
                        (tank_cfg.gravity_drain_coeff, "gravity_drain_coeff"),
                    ],
                    ctx,
                )?;
                require_opt_finite(tank_cfg.initial_level_m, "initial_level_m", ctx)?;
                if tank_cfg.surface_area_m2 <= 0.0 || tank_cfg.max_height_m <= 0.0 {
                    return Err(ConfigError::InvalidProcessConfig {
                        process_type: "Tank".into(),
                        reason: "surface_area_m2 and max_height_m must be strictly positive".into(),
                    });
                }
            }
            wireplant_engine::process::ProcessModelSpec::Thermal {
                temp_tag,
                heater_tag,
                config: therm_cfg,
            } => {
                check_register_output("Thermal", temp_tag, &tag_areas)?;
                check_writable_input("Thermal", heater_tag, &tag_areas)?;

                let ctx = "Process Thermal";
                require_all_finite(
                    &[
                        (therm_cfg.ambient_temp_c, "ambient_temp_c"),
                        (therm_cfg.max_heating_power_watts, "max_heating_power_watts"),
                        (therm_cfg.heat_capacity_j_k, "heat_capacity_j_k"),
                        (therm_cfg.heat_loss_coeff_w_k, "heat_loss_coeff_w_k"),
                    ],
                    ctx,
                )?;
                require_opt_finite(therm_cfg.initial_temp_c, "initial_temp_c", ctx)?;
                if therm_cfg.heat_capacity_j_k <= 0.0 {
                    return Err(ConfigError::InvalidProcessConfig {
                        process_type: "Thermal".into(),
                        reason: "heat_capacity_j_k must be strictly positive".into(),
                    });
                }
            }
            wireplant_engine::process::ProcessModelSpec::PidController {
                setpoint_tag,
                process_var_tag,
                output_tag,
                config: pid_cfg,
            } => {
                check_writable_input("PidController", setpoint_tag, &tag_areas)?;
                check_tag_ref("PidController", process_var_tag, &tag_areas)?;
                check_register_output("PidController", output_tag, &tag_areas)?;

                let ctx = "Process PidController";
                require_all_finite(
                    &[
                        (pid_cfg.kp, "kp"),
                        (pid_cfg.ki, "ki"),
                        (pid_cfg.kd, "kd"),
                        (pid_cfg.min_output, "min_output"),
                        (pid_cfg.max_output, "max_output"),
                        (pid_cfg.derivative_filter_n, "derivative_filter_n"),
                    ],
                    ctx,
                )?;

                if pid_cfg.min_output >= pid_cfg.max_output {
                    return Err(ConfigError::InvalidProcessConfig {
                        process_type: "PidController".into(),
                        reason: format!(
                            "min_output ({}) must be strictly less than max_output ({})",
                            pid_cfg.min_output, pid_cfg.max_output
                        ),
                    });
                }
            }
        }
    }

    // Verify fault profiles
    for profile in &config.fault_profiles {
        let ctx = format!("FaultProfile '{}'", profile.id);
        for action in &profile.actions {
            if !tag_areas.contains_key(action.target_tag.as_str()) {
                return Err(ConfigError::InvalidFaultTagReference {
                    profile_id: profile.id.clone(),
                    missing_tag: action.target_tag.clone(),
                });
            }
            require_finite(action.force_value, "force_value", &ctx)?;
            if let Some(d) = action.duration_secs {
                require_finite(d, "duration_secs", &ctx)?;
                if d <= 0.0 {
                    return Err(ConfigError::NonFiniteParameter {
                        context: ctx.clone(),
                        field: "duration_secs must be positive".into(),
                    });
                }
            }
        }
    }

    // Verify alarms
    let mut seen_alarm_ids = HashSet::new();
    for alarm in &config.alarms {
        if !seen_alarm_ids.insert(alarm.id.clone()) {
            return Err(ConfigError::DuplicateAlarmId(alarm.id.clone()));
        }
        if !tag_areas.contains_key(alarm.source_tag.as_str()) {
            return Err(ConfigError::InvalidAlarm {
                alarm_id: alarm.id.clone(),
                reason: format!("source_tag '{}' does not exist", alarm.source_tag),
            });
        }

        let ctx = format!("Alarm '{}'", alarm.id);
        require_all_finite(
            &[
                (alarm.limit, "limit"),
                (alarm.deadband, "deadband"),
                (alarm.on_delay_secs, "on_delay_secs"),
                (alarm.off_delay_secs, "off_delay_secs"),
                (alarm.max_shelve_duration_secs, "max_shelve_duration_secs"),
            ],
            &ctx,
        )?;

        if alarm.deadband < 0.0 {
            return Err(ConfigError::InvalidAlarm {
                alarm_id: alarm.id.clone(),
                reason: "deadband must be non-negative".into(),
            });
        }
        if alarm.on_delay_secs < 0.0 || alarm.off_delay_secs < 0.0 {
            return Err(ConfigError::InvalidAlarm {
                alarm_id: alarm.id.clone(),
                reason: "delays must be non-negative".into(),
            });
        }
        if alarm.max_shelve_duration_secs < 0.0 {
            return Err(ConfigError::InvalidAlarm {
                alarm_id: alarm.id.clone(),
                reason: "max_shelve_duration_secs must be non-negative".into(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validator_rejects_pid_with_inverted_output_limits() {
        let toml_str = r#"
[facility]
name = "PID Test"
tick_rate_ms = 100

[[tags]]
id = "sp"
area = "holding_register"
address = 0
data_type = "float32"
waveform = { type = "constant", value = 50.0 }

[[tags]]
id = "pv"
area = "input_register"
address = 2
data_type = "float32"
waveform = { type = "constant", value = 40.0 }

[[tags]]
id = "out"
area = "holding_register"
address = 4
data_type = "float32"
waveform = { type = "constant", value = 0.0 }

[[processes]]
type = "pid_controller"
setpoint_tag = "sp"
process_var_tag = "pv"
output_tag = "out"
config = { kp = 1.0, ki = 0.1, kd = 0.05, min_output = 100.0, max_output = 0.0 }
"#;
        let err = ScenarioConfig::from_toml(toml_str).unwrap_err();
        match err {
            ConfigError::InvalidProcessConfig {
                process_type,
                reason,
            } => {
                assert_eq!(process_type, "PidController");
                assert!(reason.contains("min_output"));
            }
            other => panic!("expected InvalidProcessConfig, got {other:?}"),
        }
    }

    #[test]
    fn validator_rejects_nan_in_waveform_and_alarm() {
        let toml_nan_waveform = r#"
[facility]
name = "NaN Test"
tick_rate_ms = 100

[[tags]]
id = "temp"
area = "holding_register"
address = 0
data_type = "float32"
waveform = { type = "constant", value = nan }
"#;
        let err = ScenarioConfig::from_toml(toml_nan_waveform).unwrap_err();
        match err {
            ConfigError::NonFiniteParameter { field, .. } => {
                assert_eq!(field, "value");
            }
            other => panic!("expected NonFiniteParameter, got {other:?}"),
        }
    }

    #[test]
    fn parser_rejects_unknown_toml_fields() {
        let toml_str = r#"
[facility]
name = "Typo Test"
tick_rate_ms = 100

[network.modbus]
bind_adr = "127.0.0.1:5020"

[[tags]]
id = "temp"
area = "holding_register"
address = 0
data_type = "float32"
waveform = { type = "constant", value = 25.0 }
"#;
        let err = ScenarioConfig::from_toml(toml_str).unwrap_err();
        match err {
            ConfigError::ParseError(msg) => {
                assert!(msg.contains("unknown field `bind_adr`"));
            }
            other => panic!("expected ParseError for unknown field, got {other:?}"),
        }
    }

    #[test]
    fn parser_accepts_uint16_and_uint32_data_types() {
        let toml_str = r#"
[facility]
name = "Unsigned Test"
tick_rate_ms = 100

[[tags]]
id = "count16"
area = "holding_register"
address = 0
data_type = "uint16"
waveform = { type = "constant", value = 1234.0 }

[[tags]]
id = "count32"
area = "holding_register"
address = 1
data_type = "uint32"
waveform = { type = "constant", value = 56789.0 }
"#;
        let config =
            ScenarioConfig::from_toml(toml_str).expect("uint16 and uint32 must parse cleanly");
        assert_eq!(config.tags[0].data_type, TagDataType::UInt16);
        assert_eq!(config.tags[1].data_type, TagDataType::UInt32);
    }

    #[test]
    fn validator_rejects_invalid_serial_options() {
        let toml_str = r#"
[facility]
name = "Serial Test"
tick_rate_ms = 100

[network.modbus]
serial_enabled = true
serial_port = "/dev/ttyUSB0"
parity = "evenn"
baud_rate = 9600

[[tags]]
id = "tag1"
area = "holding_register"
address = 0
data_type = "float32"
waveform = { type = "constant", value = 1.0 }
"#;
        let err = ScenarioConfig::from_toml(toml_str).unwrap_err();
        match err {
            ConfigError::InvalidSerialParameter { field, reason } => {
                assert_eq!(field, "parity");
                assert!(reason.contains("evenn"));
            }
            other => panic!("expected InvalidSerialParameter, got {other:?}"),
        }
    }
}
