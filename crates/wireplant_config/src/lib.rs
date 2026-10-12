//! # WirePlant Config (`wireplant_config`)
//!
//! Declarative scenario TOML parsing, semantic validation, address overlap detection,
//! and Process Image table sizing for WirePlant.

pub mod defaults;
pub mod errors;
pub mod memory;
pub mod schema;
pub mod validation;

pub use defaults::get_default_scenario_toml;
pub use errors::ConfigError;
pub use memory::{process_image_sizes, ProcessImageSizes};
pub use schema::{
    DerivativeMode, FacilityConfig, FaultAction, FaultProfile, HttpConfig, MemoryConfig,
    ModbusConfig, MqttConfig, NetworkConfig, OpcUaConfig, PidConfig, ProcessModelSpec,
    ScenarioConfig, TagConfig, TankModelConfig, ThermalModelConfig,
};
pub use validation::validate_scenario;

#[cfg(test)]
mod tests {
    use super::*;
    use wireplant_core::types::MemoryArea;

    // Most tests below care about one thing: a tag layout, a network block, a
    // process wiring. These builders produce the rest of the document so each
    // test only spells out the part it is actually checking.

    /// Wraps `body` in a valid `[facility]` header.
    fn scenario(body: &str) -> String {
        facility("Test Plant", 100, body)
    }

    fn facility(name: &str, tick_rate_ms: u64, body: &str) -> String {
        format!("[facility]\nname = \"{name}\"\ntick_rate_ms = {tick_rate_ms}\n\n{body}")
    }

    /// One `[[tags]]` entry holding a constant. Validation never looks at the
    /// constant itself, so it is fixed at zero.
    fn tag(id: &str, area: &str, address: u16, data_type: &str) -> String {
        tag_with(
            id,
            area,
            address,
            data_type,
            r#"{ type = "constant", value = 0.0 }"#,
        )
    }

    fn tag_with(id: &str, area: &str, address: u16, data_type: &str, waveform: &str) -> String {
        format!(
            "[[tags]]\nid = \"{id}\"\narea = \"{area}\"\naddress = {address}\n\
             data_type = \"{data_type}\"\nwaveform = {waveform}\n\n"
        )
    }

    /// Smallest legal tag list, for tests that are about some other section.
    fn one_coil() -> String {
        tag("t1", "coil", 1, "boolean")
    }

    fn accept(toml: &str) -> ScenarioConfig {
        ScenarioConfig::from_toml(toml)
            .unwrap_or_else(|e| panic!("scenario should be valid, got {e:?}\n---\n{toml}"))
    }

    fn reject(toml: &str) -> ConfigError {
        match ScenarioConfig::from_toml(toml) {
            Ok(_) => panic!("scenario should have been rejected\n---\n{toml}"),
            Err(e) => e,
        }
    }

    // --- bundled scenarios -------------------------------------------------

    #[test]
    fn the_embedded_scenario_parses_and_validates() {
        let parsed = accept(defaults::get_default_scenario_toml());
        assert_eq!(parsed.facility.name, "Hydroelectric Substation 04");
        assert_eq!(parsed.facility.tick_rate_ms, 100);
        assert_eq!(parsed.tags.len(), 4);
    }

    #[test]
    fn the_bundled_scenario_files_parse() {
        let cases = [
            (
                include_str!("../scenarios/water_treatment.toml"),
                "ClearFlow Municipal Water Filtration Plant",
                15,
            ),
            (
                include_str!("../scenarios/petrochemical.toml"),
                "Gulf Coast Refinery - Unit 12 Fluid Catalytic Cracker",
                16,
            ),
            (
                include_str!("../scenarios/default_hydro.toml"),
                "Hydroelectric Substation 04",
                4,
            ),
        ];
        for (toml, name, tag_count) in cases {
            let parsed = accept(toml);
            assert_eq!(parsed.facility.name, name);
            assert_eq!(parsed.tags.len(), tag_count, "{name}");
        }
    }

    // --- facility ----------------------------------------------------------

    #[test]
    fn a_blank_facility_name_is_rejected() {
        let err = reject(&facility("   ", 100, &one_coil()));
        assert!(matches!(err, ConfigError::InvalidFacility(_)));
    }

    #[test]
    fn a_tick_rate_outside_the_allowed_range_is_rejected() {
        for rate in [0, 10_001] {
            let err = reject(&facility("Tick Test", rate, &one_coil()));
            assert_eq!(err, ConfigError::InvalidTickRate(rate));
        }
    }

    // --- tag layout --------------------------------------------------------

    #[test]
    fn two_tags_on_one_register_are_rejected() {
        // A float32 at 100 also owns 101.
        let err = reject(&scenario(&format!(
            "{}{}",
            tag("tag1", "holding_register", 100, "float32"),
            tag("tag2", "holding_register", 101, "float32"),
        )));
        assert!(matches!(
            err,
            ConfigError::RegisterCollision {
                area: MemoryArea::HoldingRegister,
                address: 101,
                tag_a,
                tag_b
            } if tag_a == "tag1" && tag_b == "tag2"
        ));
    }

    #[test]
    fn a_float64_tag_collides_with_the_tag_three_registers_later() {
        let err = reject(&scenario(&format!(
            "{}{}",
            tag("f64_tag", "input_register", 200, "float64"),
            tag("int16_tag", "input_register", 203, "int16"),
        )));
        assert!(matches!(
            err,
            ConfigError::RegisterCollision {
                area: MemoryArea::InputRegister,
                address: 203,
                ..
            }
        ));
    }

    #[test]
    fn a_wide_tag_spans_the_registers_its_type_needs() {
        // u32 at 10 takes 10..=11, so 12 is the first free word.
        let parsed = accept(&scenario(&format!(
            "{}{}",
            tag("u32_tag", "holding_register", 10, "u_int32"),
            tag("adjacent_tag", "holding_register", 12, "int16"),
        )));
        assert_eq!(parsed.tags.len(), 2);
    }

    #[test]
    fn the_same_address_in_different_areas_is_allowed() {
        let parsed = accept(&scenario(&format!(
            "{}{}{}{}",
            tag("coil_tag", "coil", 100, "boolean"),
            tag("di_tag", "discrete_input", 100, "boolean"),
            tag("ir_tag", "input_register", 100, "int16"),
            tag("hr_tag", "holding_register", 100, "int16"),
        )));
        assert_eq!(parsed.tags.len(), 4);
    }

    #[test]
    fn two_tags_with_the_same_id_are_rejected() {
        let err = reject(&scenario(&format!(
            "{}{}",
            tag("sensor_a", "holding_register", 10, "int16"),
            tag("sensor_a", "holding_register", 20, "int16"),
        )));
        assert!(matches!(err, ConfigError::DuplicateTagId(id) if id == "sensor_a"));
    }

    #[test]
    fn an_address_that_overflows_its_span_is_rejected() {
        let err = reject(&scenario(&tag(
            "overflow_tag",
            "holding_register",
            65535,
            "float32",
        )));
        assert!(matches!(err, ConfigError::AddressOverflow(id) if id == "overflow_tag"));
    }

    #[test]
    fn a_register_type_in_a_bit_area_is_rejected() {
        let err = reject(&scenario(&tag("float_in_a_coil", "coil", 1, "float32")));
        assert!(matches!(
            err,
            ConfigError::DataTypeNotInArea { tag_id, .. } if tag_id == "float_in_a_coil"
        ));
    }

    // --- waveforms ---------------------------------------------------------

    #[test]
    fn a_trip_pointing_at_a_missing_tag_is_rejected() {
        let err = reject(&scenario(&tag_with(
            "alarm_coil",
            "coil",
            1,
            "boolean",
            r#"{ type = "trip_threshold", source_tag = "nonexistent_pressure", threshold = 100.0, inverted = false }"#,
        )));
        assert!(matches!(
            err,
            ConfigError::DanglingTagReference {
                dependent_tag,
                missing_source
            } if dependent_tag == "alarm_coil" && missing_source == "nonexistent_pressure"
        ));
    }

    #[test]
    fn an_inverted_range_waveform_is_rejected() {
        let err = reject(&scenario(&tag_with(
            "bad_sine",
            "holding_register",
            100,
            "float32",
            r#"{ type = "sine", min = 100.0, max = 50.0, period_secs = 10.0 }"#,
        )));
        assert!(matches!(err, ConfigError::InvalidWaveform { tag_id, .. } if tag_id == "bad_sine"));
    }

    // --- process models ----------------------------------------------------

    #[test]
    fn a_scenario_with_all_three_process_kinds_parses() {
        let tags = [
            tag("water_level_m", "input_register", 100, "float32"),
            tag("inlet_valve_cmd", "holding_register", 200, "float32"),
            tag("drain_pump_cmd", "coil", 10, "boolean"),
            tag("furnace_temp_c", "input_register", 102, "float32"),
            tag("heater_duty", "holding_register", 202, "float32"),
            tag("target_level_sp", "holding_register", 204, "float32"),
            tag("pid_output", "holding_register", 206, "float32"),
        ]
        .concat();

        let processes = r#"
[[processes]]
type = "tank"
level_tag = "water_level_m"
inlet_valve_tag = "inlet_valve_cmd"
drain_pump_tag = "drain_pump_cmd"
[processes.config]
surface_area_m2 = 10.0
max_height_m = 12.0
max_inflow_m3_s = 2.0
max_outflow_m3_s = 1.5
gravity_drain_coeff = 0.05
initial_level_m = 1.0

[[processes]]
type = "thermal"
temp_tag = "furnace_temp_c"
heater_tag = "heater_duty"
[processes.config]
ambient_temp_c = 25.0
max_heating_power_watts = 5000.0
heat_capacity_j_k = 10000.0
heat_loss_coeff_w_k = 25.0
initial_temp_c = 25.0

[[processes]]
type = "pid_controller"
setpoint_tag = "target_level_sp"
process_var_tag = "water_level_m"
output_tag = "pid_output"
[processes.config]
kp = 1.5
ki = 0.1
kd = 0.05
min_output = 0.0
max_output = 1.0
"#;
        let parsed = accept(&scenario(&format!("{tags}{processes}")));
        assert_eq!(parsed.processes.len(), 3);
    }

    #[test]
    fn a_process_naming_a_missing_tag_is_rejected() {
        let body = format!(
            "{}{}{}",
            tag("level", "input_register", 100, "float32"),
            tag("valve", "holding_register", 200, "float32"),
            r#"
[[processes]]
type = "tank"
level_tag = "level"
inlet_valve_tag = "valve"
drain_pump_tag = "nonexistent_pump"
[processes.config]
surface_area_m2 = 10.0
max_height_m = 12.0
max_inflow_m3_s = 2.0
max_outflow_m3_s = 1.5
gravity_drain_coeff = 0.05
"#,
        );
        assert_eq!(
            reject(&scenario(&body)),
            ConfigError::InvalidProcessTagReference {
                process_type: "Tank".to_string(),
                missing_tag: "nonexistent_pump".to_string(),
            }
        );
    }

    #[test]
    fn an_operator_input_in_a_read_only_area_is_rejected() {
        // The heater is something the operator writes, so an input register
        // (read-only over Modbus) can't hold it.
        let body = format!(
            "{}{}{}",
            tag("furnace_temp", "input_register", 100, "float32"),
            tag("heater_sensor", "input_register", 102, "float32"),
            r#"
[[processes]]
type = "thermal"
temp_tag = "furnace_temp"
heater_tag = "heater_sensor"
[processes.config]
ambient_temp_c = 25.0
max_heating_power_watts = 5000.0
heat_capacity_j_k = 10000.0
heat_loss_coeff_w_k = 25.0
"#,
        );
        assert_eq!(
            reject(&scenario(&body)),
            ConfigError::InvalidOperatorInputArea {
                process_type: "Thermal".to_string(),
                tag_id: "heater_sensor".to_string(),
                area: MemoryArea::InputRegister,
            }
        );
    }

    // --- network -----------------------------------------------------------

    #[test]
    fn a_fully_specified_network_section_round_trips() {
        let network = r#"
[network.modbus]
enabled = false
bind_addr = "127.0.0.1:5021"
default_unit_id = 2
allowed_unit_ids = [1, 2, 3]
max_connections = 50

[network.opcua]
enabled = true
bind_addr = "0.0.0.0:4841"

[network.http]
enabled = false
bind_addr = "127.0.0.1:8081"

[network.mqtt]
enabled = true
broker_addr = "mqtt.internal:1883"
client_id = "test_station"

"#;
        let net = accept(&scenario(&format!("{network}{}", one_coil()))).network;

        assert!(!net.modbus.enabled);
        assert_eq!(net.modbus.bind_addr, "127.0.0.1:5021");
        assert_eq!(net.modbus.default_unit_id, 2);
        assert_eq!(net.modbus.allowed_unit_ids, vec![1, 2, 3]);
        assert_eq!(net.modbus.max_connections, 50);

        assert!(net.opcua.enabled);
        assert_eq!(net.opcua.bind_addr, "0.0.0.0:4841");

        assert!(!net.http.enabled);
        assert_eq!(net.http.bind_addr, "127.0.0.1:8081");

        assert!(net.mqtt.enabled);
        assert_eq!(net.mqtt.broker_addr, "mqtt.internal:1883");
        assert_eq!(net.mqtt.client_id, Some("test_station".to_string()));
    }

    #[test]
    fn omitted_mqtt_keys_fall_back_to_the_default_impl() {
        // subscribe_commands is the one bool here that defaults to true, so
        // it's the field most likely to drift if a second default sneaks in.
        let with_section = accept(&scenario(&format!(
            "[network.mqtt]\nenabled = true\n\n{}",
            one_coil()
        )));
        let without_section = accept(&scenario(&one_coil()));

        assert!(with_section.network.mqtt.subscribe_commands);
        assert_eq!(
            with_section.network.mqtt.broker_addr,
            MqttConfig::default().broker_addr
        );
        assert_eq!(without_section.network.mqtt, MqttConfig::default());
    }

    #[test]
    fn an_enabled_protocol_with_no_bind_address_is_rejected() {
        let err = reject(&scenario(&format!(
            "[network.opcua]\nenabled = true\nbind_addr = \"\"\n\n{}",
            one_coil()
        )));
        assert_eq!(err, ConfigError::EmptyBindAddress("opcua".to_string()));
    }

    #[test]
    fn a_disabled_protocol_may_leave_its_bind_address_empty() {
        let parsed = accept(&scenario(&format!(
            "[network.opcua]\nenabled = false\n\n{}",
            one_coil()
        )));
        assert!(!parsed.network.opcua.enabled);
    }

    #[test]
    fn a_broker_address_with_an_unusable_port_is_rejected() {
        let err = reject(&scenario(&format!(
            "[network.mqtt]\nenabled = true\nbroker_addr = \"127.0.0.1:1883x\"\n\n{}",
            one_coil()
        )));
        assert!(matches!(err, ConfigError::InvalidEndpoint { protocol, .. } if protocol == "mqtt"));
    }

    // --- fault profiles ----------------------------------------------------

    #[test]
    fn a_fault_profile_parses_with_its_optional_fields() {
        let body = format!(
            "{}{}{}",
            tag("pressure_psi", "holding_register", 100, "float32"),
            tag("pump_relay", "coil", 1, "boolean"),
            r#"
[[fault_profiles]]
id = "overpressure_burst"
name = "Penstock Overpressure Emergency"
description = "Forces inlet pressure to 195 PSI"
actions = [
    { target_tag = "pressure_psi", force_value = 195.0, duration_secs = 10.0 }
]

[[fault_profiles]]
id = "pump_cut"
name = "Emergency Pump Trip"
actions = [
    { target_tag = "pump_relay", force_value = 0.0 }
]
"#,
        );
        let profiles = accept(&scenario(&body)).fault_profiles;
        assert_eq!(profiles.len(), 2);

        let burst = &profiles[0];
        assert_eq!(burst.id, "overpressure_burst");
        assert_eq!(burst.actions[0].target_tag, "pressure_psi");
        assert_eq!(burst.actions[0].force_value, 195.0);
        assert_eq!(burst.actions[0].duration_secs, Some(10.0));

        let cut = &profiles[1];
        assert_eq!(cut.id, "pump_cut");
        assert_eq!(cut.description, None);
        assert_eq!(cut.actions[0].target_tag, "pump_relay");
        assert_eq!(cut.actions[0].force_value, 0.0);
        assert_eq!(cut.actions[0].duration_secs, None);
    }

    #[test]
    fn a_fault_profile_naming_a_missing_tag_is_rejected() {
        let body = format!(
            "{}{}",
            tag("pressure_psi", "holding_register", 100, "float32"),
            r#"
[[fault_profiles]]
id = "broken_profile"
name = "Broken Profile"
actions = [
    { target_tag = "nonexistent_tag", force_value = 123.0 }
]
"#,
        );
        assert_eq!(
            reject(&scenario(&body)),
            ConfigError::InvalidFaultTagReference {
                profile_id: "broken_profile".to_string(),
                missing_tag: "nonexistent_tag".to_string(),
            }
        );
    }
}
