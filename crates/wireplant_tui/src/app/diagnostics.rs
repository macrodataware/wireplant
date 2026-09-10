use wireplant_core::codec::encode_tag;
use wireplant_core::types::MemoryArea;
use wireplant_engine::process::ProcessModelSpec;

use super::push_capped;
use super::types::{NetworkServiceDiag, PidLoopState, TankProcessState, ThermalProcessState};
use super::TuiApp;

impl TuiApp {
    /// Extracts real-time process model states from current scenario and live tick readings.
    pub fn get_process_states(
        &self,
    ) -> (
        Vec<TankProcessState>,
        Vec<ThermalProcessState>,
        Vec<PidLoopState>,
    ) {
        let mut tanks = Vec::new();
        let mut thermals = Vec::new();
        let mut pids = Vec::new();

        for spec in &self.scenario.processes {
            match spec {
                ProcessModelSpec::Tank {
                    level_tag,
                    inlet_valve_tag,
                    drain_pump_tag,
                    config,
                } => {
                    let level = self.get_tag_reading(level_tag).unwrap_or(0.0);
                    let inlet = self.get_tag_reading(inlet_valve_tag).unwrap_or(0.0);
                    let drain = self.get_tag_reading(drain_pump_tag).unwrap_or(0.0);
                    tanks.push(TankProcessState {
                        level_tag: level_tag.clone(),
                        inlet_valve_tag: inlet_valve_tag.clone(),
                        drain_pump_tag: drain_pump_tag.clone(),
                        current_level: level,
                        max_height: config.max_height_m,
                        surface_area: config.surface_area_m2,
                        inflow_rate: inlet * config.max_inflow_m3_s,
                        outflow_rate: drain * config.max_outflow_m3_s,
                        gravity_drain_coeff: config.gravity_drain_coeff,
                    });
                }
                ProcessModelSpec::Thermal {
                    temp_tag,
                    heater_tag,
                    config,
                } => {
                    let temp = self
                        .get_tag_reading(temp_tag)
                        .unwrap_or(config.ambient_temp_c);
                    let heater = self.get_tag_reading(heater_tag).unwrap_or(0.0);
                    thermals.push(ThermalProcessState {
                        temp_tag: temp_tag.clone(),
                        heater_tag: heater_tag.clone(),
                        current_temp: temp,
                        ambient_temp: config.ambient_temp_c,
                        heater_input: heater,
                    });
                }
                ProcessModelSpec::PidController {
                    setpoint_tag,
                    process_var_tag,
                    output_tag,
                    config,
                } => {
                    let sp = self.get_tag_reading(setpoint_tag).unwrap_or(0.0);
                    let pv = self.get_tag_reading(process_var_tag).unwrap_or(0.0);
                    let cv = self.get_tag_reading(output_tag).unwrap_or(0.0);
                    pids.push(PidLoopState {
                        setpoint_tag: setpoint_tag.clone(),
                        process_var_tag: process_var_tag.clone(),
                        output_tag: output_tag.clone(),
                        setpoint: sp,
                        process_variable: pv,
                        output: cv,
                        error: sp - pv,
                        config: config.clone(),
                    });
                }
            }
        }

        (tanks, thermals, pids)
    }

    /// Diagnostic descriptors for all network and storage services.
    pub fn get_network_diagnostics(&self) -> Vec<NetworkServiceDiag> {
        let net = &self.scenario.network;
        let mut services = Vec::new();

        // 1. Modbus TCP Gateway
        let active_sessions = self.sessions.active();
        let max_sessions = net.modbus.max_connections;
        let modbus_status = if net.modbus.enabled {
            "ONLINE / LISTENING"
        } else {
            "DISABLED"
        };
        services.push(NetworkServiceDiag {
            name: "Modbus TCP Gateway".to_string(),
            endpoint: net.modbus.bind_addr.clone(),
            is_enabled: net.modbus.enabled,
            status_text: modbus_status.to_string(),
            details: format!("Live Sessions: {} / {} max", active_sessions, max_sessions),
        });

        // 2. Modbus RTU Serial Gateway
        let rtu_status = if net.modbus.serial_enabled {
            "ONLINE / BOUND"
        } else {
            "DISABLED"
        };
        services.push(NetworkServiceDiag {
            name: "Modbus RTU Serial Gateway".to_string(),
            endpoint: net.modbus.serial_port.clone().unwrap_or_default(),
            is_enabled: net.modbus.serial_enabled,
            status_text: rtu_status.to_string(),
            details: format!(
                "{} baud, {}-{}-{}",
                net.modbus.baud_rate, net.modbus.data_bits, net.modbus.parity, net.modbus.stop_bits
            ),
        });

        // 3. OPC-UA Binary Endpoint
        let opcua_status = if net.opcua.enabled {
            "ONLINE / LISTENING"
        } else {
            "DISABLED"
        };
        services.push(NetworkServiceDiag {
            name: "OPC-UA Binary Endpoint".to_string(),
            endpoint: net.opcua.bind_addr.clone(),
            is_enabled: net.opcua.enabled,
            status_text: opcua_status.to_string(),
            details: format!(
                "Anonymous: {} | Users: {}",
                if net.opcua.allow_anonymous {
                    "YES"
                } else {
                    "NO"
                },
                net.opcua.users.len()
            ),
        });

        // 4. Axum HTTP REST Inspection API
        let http_status = if net.http.enabled {
            "ONLINE / LISTENING"
        } else {
            "DISABLED"
        };
        services.push(NetworkServiceDiag {
            name: "Axum HTTP REST API".to_string(),
            endpoint: net.http.bind_addr.clone(),
            is_enabled: net.http.enabled,
            status_text: http_status.to_string(),
            details: "GET /health, /api/v1/state, /api/v1/alarms, /api/v1/history".to_string(),
        });

        // 5. MQTT Telemetry Publisher
        let mqtt_status = if net.mqtt.enabled {
            "ACTIVE / PUBLISHING"
        } else {
            "DISABLED"
        };
        let client_id = net.mqtt.client_id_or_default(&self.scenario.facility.name);
        services.push(NetworkServiceDiag {
            name: "MQTT Telemetry".to_string(),
            endpoint: net.mqtt.broker_addr.clone(),
            is_enabled: net.mqtt.enabled,
            status_text: mqtt_status.to_string(),
            details: format!("Client ID: {}", client_id),
        });

        // 6. SQLite Embedded Historian
        let (hist_status, hist_path, hist_details) = match &self.historian {
            Some(h) => (
                "ACTIVE / INGESTING",
                h.config.db_path.to_string_lossy().to_string(),
                format!(
                    "WAL Mode | Batch: {} | Flush: {:?} | Retain: {}h",
                    h.config.batch_size, h.config.flush_interval, h.config.retention_hours
                ),
            ),
            None => (
                "DISABLED",
                self.scenario.historian.db_path.clone(),
                "Historian persistence disabled".to_string(),
            ),
        };
        services.push(NetworkServiceDiag {
            name: "SQLite Historian".to_string(),
            endpoint: hist_path,
            is_enabled: self.historian.is_some(),
            status_text: hist_status.to_string(),
            details: hist_details,
        });

        services
    }

    pub fn get_tag_reading(&self, tag_id: &str) -> Option<f64> {
        self.active_overrides
            .get(tag_id)
            .map(|o| o.value)
            .or_else(|| self.sparkline_data.get(tag_id)?.back().copied())
    }

    pub(crate) fn push_sparkline(&mut self, tag_id: &str, value: f64) {
        if let Some(buf) = self.sparkline_data.get_mut(tag_id) {
            push_capped(buf, value);
        }
    }

    pub(crate) fn read_bit(&self, area: MemoryArea, address: usize) -> bool {
        match self.shared_memory.try_read() {
            Ok(mem) => mem.read_bit(area, address).unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Writes `value` into the tag's memory slot.
    pub(crate) fn write_tag(&self, tag_id: &str, value: f64) -> Result<(), String> {
        let Some(tag) = self.scenario.tag(tag_id) else {
            return Err("tag is not in this scenario".to_string());
        };

        if !tag.area.is_writable() {
            return Err(format!(
                "tag '{}' is in read-only area '{}'",
                tag.id,
                tag.area.as_str()
            ));
        }

        let mut mem = self
            .shared_memory
            .try_write()
            .map_err(|_| "Process Image busy with a simulation tick".to_string())?;
        encode_tag(&mut mem, tag, value).map_err(|e| e.to_string())
    }
}
