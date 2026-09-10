pub mod pid;
pub mod tank;
pub mod thermal;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use pid::{DerivativeMode, PidConfig, PidController};
pub use tank::{TankModelConfig, TankProcess};
pub use thermal::{ThermalModelConfig, ThermalProcess};

/// Declarative specification of a physics or control process model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessModelSpec {
    Tank {
        level_tag: String,
        inlet_valve_tag: String,
        drain_pump_tag: String,
        config: TankModelConfig,
    },
    Thermal {
        temp_tag: String,
        heater_tag: String,
        config: ThermalModelConfig,
    },
    PidController {
        setpoint_tag: String,
        process_var_tag: String,
        output_tag: String,
        config: PidConfig,
    },
}

/// Active stateful runtime instance of a simulated physical process or control loop.
#[derive(Debug, Clone)]
pub enum ActiveProcess {
    Tank {
        level_tag: String,
        inlet_valve_tag: String,
        drain_pump_tag: String,
        model: TankProcess,
    },
    Thermal {
        temp_tag: String,
        heater_tag: String,
        model: ThermalProcess,
    },
    Pid {
        setpoint_tag: String,
        process_var_tag: String,
        output_tag: String,
        model: PidController,
    },
}

impl ActiveProcess {
    pub fn from_spec(spec: ProcessModelSpec) -> Self {
        match spec {
            ProcessModelSpec::Tank {
                level_tag,
                inlet_valve_tag,
                drain_pump_tag,
                config,
            } => ActiveProcess::Tank {
                level_tag,
                inlet_valve_tag,
                drain_pump_tag,
                model: TankProcess::new(config),
            },
            ProcessModelSpec::Thermal {
                temp_tag,
                heater_tag,
                config,
            } => ActiveProcess::Thermal {
                temp_tag,
                heater_tag,
                model: ThermalProcess::new(config),
            },
            ProcessModelSpec::PidController {
                setpoint_tag,
                process_var_tag,
                output_tag,
                config,
            } => ActiveProcess::Pid {
                setpoint_tag,
                process_var_tag,
                output_tag,
                model: PidController::new(config),
            },
        }
    }

    /// Primary output tag driven by this process.
    pub fn output_tag(&self) -> &str {
        match self {
            ActiveProcess::Tank { level_tag, .. } => level_tag,
            ActiveProcess::Thermal { temp_tag, .. } => temp_tag,
            ActiveProcess::Pid { output_tag, .. } => output_tag,
        }
    }

    /// Input actuator tags required by this process.
    pub fn input_tags(&self) -> Vec<&str> {
        match self {
            ActiveProcess::Tank {
                inlet_valve_tag,
                drain_pump_tag,
                ..
            } => vec![inlet_valve_tag.as_str(), drain_pump_tag.as_str()],
            ActiveProcess::Thermal { heater_tag, .. } => vec![heater_tag.as_str()],
            ActiveProcess::Pid {
                setpoint_tag,
                process_var_tag,
                ..
            } => vec![setpoint_tag.as_str(), process_var_tag.as_str()],
        }
    }

    /// Steps the model and answers with `(output_tag, new_value)`.
    ///
    /// Actuator values are looked up in `inputs` by tag name; a named tag that
    /// is missing is treated as zero rather than as an error, so a scenario that
    /// references an undefined actuator still ticks.
    pub fn step(&mut self, inputs: &HashMap<String, f64>, dt_secs: f64) -> (String, f64) {
        match self {
            ActiveProcess::Tank {
                level_tag,
                inlet_valve_tag,
                drain_pump_tag,
                model,
            } => {
                let inlet = inputs.get(inlet_valve_tag).copied().unwrap_or(0.0);
                let drain = inputs.get(drain_pump_tag).copied().unwrap_or(0.0);
                (level_tag.clone(), model.step(inlet, drain, dt_secs))
            }
            ActiveProcess::Thermal {
                temp_tag,
                heater_tag,
                model,
            } => {
                let heater = inputs.get(heater_tag).copied().unwrap_or(0.0);
                (temp_tag.clone(), model.step(heater, dt_secs))
            }
            ActiveProcess::Pid {
                setpoint_tag,
                process_var_tag,
                output_tag,
                model,
            } => {
                let setpoint = inputs.get(setpoint_tag).copied().unwrap_or(0.0);
                let process_variable = inputs.get(process_var_tag).copied().unwrap_or(0.0);
                (
                    output_tag.clone(),
                    model.compute(setpoint, process_variable, dt_secs),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tank_steps_from_the_spec_that_built_it() {
        let spec = ProcessModelSpec::Tank {
            level_tag: "level".to_string(),
            inlet_valve_tag: "inlet".to_string(),
            drain_pump_tag: "drain".to_string(),
            config: TankModelConfig {
                surface_area_m2: 1.0,
                max_height_m: 10.0,
                max_inflow_m3_s: 2.0,
                max_outflow_m3_s: 1.0,
                gravity_drain_coeff: 0.0,
                initial_level_m: Some(0.0),
            },
        };
        let mut process = ActiveProcess::from_spec(spec);
        let inputs = HashMap::from([("inlet".to_string(), 1.0)]);

        let (tag, level) = process.step(&inputs, 1.0);

        assert_eq!(tag, "level");
        assert_eq!(level, 2.0);
    }
}
