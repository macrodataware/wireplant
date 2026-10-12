use wireplant_config::defaults::get_default_scenario_toml;
use wireplant_config::memory::process_image_sizes;
use wireplant_config::schema::ScenarioConfig;

/// Runs validation on the specified scenario path or default embedded scenario.
/// Prints a structured report to standard output and returns `Ok(())` on success
/// or an error message on failure.
pub fn run_validation(scenario_path: Option<&str>) -> Result<(), String> {
    let (source_name, toml_str) = match scenario_path {
        Some(path) => {
            let content = std::fs::read_to_string(path)
                .map_err(|e| format!("Cannot read scenario file '{path}': {e}"))?;
            (path.to_string(), content)
        }
        None => (
            "<embedded default>".to_string(),
            get_default_scenario_toml().to_string(),
        ),
    };

    let scenario = ScenarioConfig::from_toml(&toml_str)
        .map_err(|e| format!("Scenario validation failed for '{source_name}':\n  {e}"))?;

    let mem_sizes = process_image_sizes(&scenario);

    println!("WirePlant Scenario Validation");
    println!("=============================");
    println!("Target:        {source_name}");
    println!("Facility:      {}", scenario.facility.name);
    if let Some(desc) = &scenario.facility.description {
        println!("Description:   {desc}");
    }
    println!("Tick Rate:     {} ms", scenario.facility.tick_rate_ms);
    println!();

    println!("Network Protocols:");
    if scenario.network.modbus.enabled {
        println!(
            "  - Modbus TCP:  {} (Unit ID: {}, Max Connections: {})",
            scenario.network.modbus.bind_addr,
            scenario.network.modbus.default_unit_id,
            scenario.network.modbus.max_connections
        );
    } else {
        println!("  - Modbus TCP:  Disabled");
    }

    if scenario.network.modbus.serial_enabled {
        println!(
            "  - Modbus RTU:  {} ({} baud, {}-bit, {}-stop)",
            scenario
                .network
                .modbus
                .serial_port
                .as_deref()
                .unwrap_or("<unspecified>"),
            scenario.network.modbus.baud_rate,
            scenario.network.modbus.data_bits,
            scenario.network.modbus.stop_bits
        );
    } else {
        println!("  - Modbus RTU:  Disabled");
    }

    if scenario.network.opcua.enabled {
        let sec = if scenario.network.opcua.security_policies.is_empty() {
            "None".to_string()
        } else {
            scenario.network.opcua.security_policies.join(", ")
        };
        println!(
            "  - OPC-UA:      {} (Security: {sec}, Anonymous: {})",
            scenario.network.opcua.bind_addr, scenario.network.opcua.allow_anonymous
        );
    } else {
        println!("  - OPC-UA:      Disabled");
    }

    if scenario.network.http.enabled {
        let auth_status = if scenario.network.http.api_key.is_some() {
            "Protected (API Key configured)"
        } else {
            "Open (No API Key)"
        };
        println!(
            "  - HTTP REST:   {} ({auth_status})",
            scenario.network.http.bind_addr
        );
    } else {
        println!("  - HTTP REST:   Disabled");
    }

    if scenario.network.mqtt.enabled {
        let auth_status = if scenario.network.mqtt.username.is_some() {
            "Authenticated"
        } else {
            "Anonymous"
        };
        println!(
            "  - MQTT:        {} (Client: {}, {auth_status})",
            scenario.network.mqtt.broker_addr,
            scenario
                .network
                .mqtt
                .client_id_or_default(&scenario.facility.name)
        );
    } else {
        println!("  - MQTT:        Disabled");
    }

    if scenario.historian.enabled {
        println!(
            "  - Historian:   {} (Retention: {}h, Flush: {}ms)",
            scenario.historian.db_path,
            scenario.historian.retention_hours,
            scenario.historian.flush_interval_ms
        );
    } else {
        println!("  - Historian:   Disabled");
    }
    println!();

    println!("Process Image Memory Allocation:");
    println!("  - Coils (0x):             {} bits", mem_sizes.coils);
    println!(
        "  - Discrete Inputs (1x):   {} bits",
        mem_sizes.discrete_inputs
    );
    println!(
        "  - Input Registers (3x):   {} words",
        mem_sizes.input_registers
    );
    println!(
        "  - Holding Registers (4x): {} words",
        mem_sizes.holding_registers
    );
    println!();

    println!("Components:");
    println!("  - Declared Tags:          {}", scenario.tags.len());
    println!("  - Process Control Loops:  {}", scenario.processes.len());
    println!("  - ISA-18.2 Alarms:        {}", scenario.alarms.len());
    println!(
        "  - Fault Profiles:         {}",
        scenario.fault_profiles.len()
    );
    println!();

    println!("Validation Status: PASSED (scenario is structurally and semantically valid).");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scenario_validates_successfully() {
        assert!(run_validation(None).is_ok());
    }

    #[test]
    fn missing_scenario_file_returns_error() {
        let res = run_validation(Some("/path/that/does/not/exist_12345.toml"));
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Cannot read scenario file"));
    }
}
