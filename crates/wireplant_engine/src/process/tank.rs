use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TankModelConfig {
    pub surface_area_m2: f64,
    pub max_height_m: f64,
    pub max_inflow_m3_s: f64,
    pub max_outflow_m3_s: f64,
    pub gravity_drain_coeff: f64,
    /// Starting level. Absent means "start empty".
    #[serde(default)]
    pub initial_level_m: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct TankProcess {
    pub config: TankModelConfig,
    pub current_level_m: f64,
    pub current_volume_m3: f64,
}

impl TankProcess {
    pub fn new(config: TankModelConfig) -> Self {
        let initial_level = config.initial_level_m.unwrap_or(0.0);
        let initial_volume = initial_level * config.surface_area_m2;
        Self {
            config,
            current_level_m: initial_level,
            current_volume_m3: initial_volume,
        }
    }

    /// Advances simulation by dt_secs given inlet valve (0.0 to 1.0) and outlet pump (0.0 to 1.0)
    pub fn step(&mut self, inlet_cmd: f64, drain_cmd: f64, dt_secs: f64) -> f64 {
        if dt_secs <= 0.0 || self.config.surface_area_m2 <= 0.0 {
            return self.current_level_m;
        }

        let q_in = inlet_cmd * self.config.max_inflow_m3_s;

        // Torricelli / gravity drain component + forced pump drain
        let gravity_head = (2.0 * 9.81 * self.current_level_m).sqrt();
        let q_gravity = self.config.gravity_drain_coeff * gravity_head;
        let q_pump = drain_cmd * self.config.max_outflow_m3_s;
        let q_out = q_gravity + q_pump;

        let delta_v = (q_in - q_out) * dt_secs;
        let max_v = self.config.surface_area_m2 * self.config.max_height_m;

        self.current_volume_m3 = (self.current_volume_m3 + delta_v).clamp(0.0, max_v);
        self.current_level_m = self.current_volume_m3 / self.config.surface_area_m2;

        self.current_level_m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tank_fills_at_inlet_flow_rate() {
        let config = TankModelConfig {
            surface_area_m2: 2.0,
            max_height_m: 10.0,
            max_inflow_m3_s: 1.0,
            max_outflow_m3_s: 0.5,
            gravity_drain_coeff: 0.0,
            initial_level_m: Some(0.0),
        };
        let mut tank = TankProcess::new(config);

        // Constant inflow with closed drain over 2 seconds
        let level = tank.step(1.0, 0.0, 2.0);
        assert!((level - 1.0).abs() < 1e-6);
        assert!((tank.current_volume_m3 - 2.0).abs() < 1e-6);
    }

    #[test]
    fn tank_drains_at_pump_flow_rate() {
        let config = TankModelConfig {
            surface_area_m2: 2.0,
            max_height_m: 10.0,
            max_inflow_m3_s: 1.0,
            max_outflow_m3_s: 0.5,
            gravity_drain_coeff: 0.0,
            initial_level_m: Some(5.0),
        };
        let mut tank = TankProcess::new(config);

        // Active discharge pump draining from half capacity
        let level = tank.step(0.0, 1.0, 2.0);
        assert!((level - 4.5).abs() < 1e-6);
    }

    #[test]
    fn tank_level_clamps_to_geometric_limits() {
        let config = TankModelConfig {
            surface_area_m2: 1.0,
            max_height_m: 5.0,
            max_inflow_m3_s: 10.0,
            max_outflow_m3_s: 10.0,
            gravity_drain_coeff: 0.0,
            initial_level_m: Some(4.0),
        };
        let mut tank = TankProcess::new(config);

        let level_overflow = tank.step(1.0, 0.0, 10.0);
        assert_eq!(level_overflow, 5.0);
        assert_eq!(tank.current_volume_m3, 5.0);

        let level_empty = tank.step(0.0, 1.0, 10.0);
        assert_eq!(level_empty, 0.0);
        assert_eq!(tank.current_volume_m3, 0.0);
    }
}
