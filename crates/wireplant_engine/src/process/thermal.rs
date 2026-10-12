use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThermalModelConfig {
    pub ambient_temp_c: f64,
    pub max_heating_power_watts: f64,
    pub heat_capacity_j_k: f64,
    pub heat_loss_coeff_w_k: f64,
    /// Starting temperature. Absent means "start at ambient", which is a
    /// different thing from starting at 0 °C.
    #[serde(default)]
    pub initial_temp_c: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ThermalProcess {
    pub config: ThermalModelConfig,
    pub current_temp_c: f64,
}

impl ThermalProcess {
    pub fn new(config: ThermalModelConfig) -> Self {
        let initial = config.initial_temp_c.unwrap_or(config.ambient_temp_c);
        Self {
            config,
            current_temp_c: initial,
        }
    }

    pub fn step(&mut self, heater_duty: f64, dt_secs: f64) -> f64 {
        if dt_secs <= 0.0 || self.config.heat_capacity_j_k <= 0.0 {
            return self.current_temp_c;
        }

        let q_in = heater_duty * self.config.max_heating_power_watts;
        let c = self.config.heat_capacity_j_k;
        let k = self.config.heat_loss_coeff_w_k;
        let t_amb = self.config.ambient_temp_c;

        if k > 0.0 {
            // Exact analytical solution: dT/dt = (q_in - k * (T - t_amb)) / C
            let decay = (-k * dt_secs / c).exp();
            let t_eq = t_amb + (q_in / k);
            self.current_temp_c = t_eq + (self.current_temp_c - t_eq) * decay;
        } else {
            // Pure adiabatic heating when heat_loss_coeff_w_k == 0
            self.current_temp_c += (q_in / c) * dt_secs;
        }

        self.current_temp_c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thermal_heats_at_heater_power() {
        let config = ThermalModelConfig {
            ambient_temp_c: 20.0,
            max_heating_power_watts: 1000.0,
            heat_capacity_j_k: 500.0,
            heat_loss_coeff_w_k: 10.0,
            initial_temp_c: Some(20.0),
        };
        let mut thermal = ThermalProcess::new(config);

        // Analytical exponential solution:
        // T(1.0) = T_eq + (T_0 - T_eq) * exp(-k * dt / C)
        // With T_eq = 20 + 1000/10 = 120, exp(-10/500) = exp(-0.02) ≈ 0.98019867...
        // T(1.0) ≈ 21.980133 °C
        let temp = thermal.step(1.0, 1.0);
        let expected = 120.0 + (20.0 - 120.0) * (-10.0f64 / 500.0).exp();
        assert!((temp - expected).abs() < 1e-6);

        // Pure adiabatic case with k = 0: exactly +2.0 °C
        let adiabatic_config = ThermalModelConfig {
            ambient_temp_c: 20.0,
            max_heating_power_watts: 1000.0,
            heat_capacity_j_k: 500.0,
            heat_loss_coeff_w_k: 0.0,
            initial_temp_c: Some(20.0),
        };
        let mut adiabatic = ThermalProcess::new(adiabatic_config);
        let temp_adiabatic = adiabatic.step(1.0, 1.0);
        assert!((temp_adiabatic - 22.0).abs() < 1e-6);
    }

    #[test]
    fn thermal_cools_toward_ambient() {
        let config = ThermalModelConfig {
            ambient_temp_c: 20.0,
            max_heating_power_watts: 1000.0,
            heat_capacity_j_k: 100.0,
            heat_loss_coeff_w_k: 10.0,
            initial_temp_c: Some(50.0),
        };
        let mut thermal = ThermalProcess::new(config);

        // Turn heater off (0.0 duty) and let it cool over several steps
        for _ in 0..100 {
            thermal.step(0.0, 1.0);
        }

        // Temp should asymptotically approach ambient (20.0)
        assert!((thermal.current_temp_c - 20.0).abs() < 0.1);
    }
}
