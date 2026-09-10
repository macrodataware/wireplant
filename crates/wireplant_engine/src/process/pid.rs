use serde::{Deserialize, Serialize};

/// Mode determining how the derivative term is computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DerivativeMode {
    /// Differentiates process variable: `-Kd * d(PV)/dt`. Eliminates setpoint kick.
    #[default]
    OnMeasurement,
    /// Differentiates error: `Kd * d(SP - PV)/dt`. Classic textbook formulation.
    OnError,
}

/// Configuration defining PID loop gains, limits, and dynamic compensation filters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidConfig {
    pub kp: f64,
    pub ki: f64,
    pub kd: f64,
    pub min_output: f64,
    pub max_output: f64,
    #[serde(default = "PidConfig::default_anti_windup")]
    pub anti_windup: bool,
    /// Derivative filter coefficient N (typically 5.0 to 20.0). Higher values reduce filtering.
    #[serde(default = "PidConfig::default_filter_n")]
    pub derivative_filter_n: f64,
    /// Calculation mode for the derivative action.
    #[serde(default)]
    pub derivative_mode: DerivativeMode,
}

impl PidConfig {
    pub const DEFAULT_ANTI_WINDUP: bool = true;
    pub const DEFAULT_DERIVATIVE_FILTER_N: f64 = 10.0;

    const fn default_anti_windup() -> bool {
        Self::DEFAULT_ANTI_WINDUP
    }

    const fn default_filter_n() -> f64 {
        Self::DEFAULT_DERIVATIVE_FILTER_N
    }
}

impl Default for PidConfig {
    fn default() -> Self {
        Self {
            kp: 1.0,
            ki: 0.0,
            kd: 0.0,
            min_output: 0.0,
            max_output: 100.0,
            anti_windup: Self::DEFAULT_ANTI_WINDUP,
            derivative_filter_n: Self::DEFAULT_DERIVATIVE_FILTER_N,
            derivative_mode: DerivativeMode::OnMeasurement,
        }
    }
}

/// Industrial PID controller with anti-windup clamping and filtered derivative.
#[derive(Debug, Clone)]
pub struct PidController {
    pub config: PidConfig,
    pub integral: f64,
    pub prev_process_variable: f64,
    pub prev_error: f64,
    pub filtered_derivative: f64,
    pub initialized: bool,
}

impl PidController {
    /// Creates a new PID controller instance with zeroed historical states.
    pub fn new(config: PidConfig) -> Self {
        Self {
            config,
            integral: 0.0,
            prev_process_variable: 0.0,
            prev_error: 0.0,
            filtered_derivative: 0.0,
            initialized: false,
        }
    }

    /// Advances the controller by `dt_secs`, returning the clamped control output.
    pub fn compute(&mut self, setpoint: f64, process_variable: f64, dt_secs: f64) -> f64 {
        if dt_secs <= 0.0 {
            return self.config.min_output;
        }

        // Defensive check: if min_output >= max_output or either is not finite, fallback safely
        let (min_out, max_out) = if self.config.min_output < self.config.max_output
            && self.config.min_output.is_finite()
            && self.config.max_output.is_finite()
        {
            (self.config.min_output, self.config.max_output)
        } else {
            (0.0, 100.0)
        };

        let error = setpoint - process_variable;
        let p = self.config.kp * error;

        // Bumpless initialization: suppress derivative kick on the first evaluated step
        let delta_pv = if self.initialized {
            process_variable - self.prev_process_variable
        } else {
            0.0
        };
        let delta_error = if self.initialized {
            error - self.prev_error
        } else {
            0.0
        };
        self.initialized = true;

        // 1. Filtered Derivative Calculation
        let d_raw = match self.config.derivative_mode {
            DerivativeMode::OnMeasurement => {
                // Derivative on measurement: -Kd * d(PV)/dt
                -self.config.kd * (delta_pv / dt_secs)
            }
            DerivativeMode::OnError => {
                // Classic derivative on error: Kd * d(error)/dt
                self.config.kd * (delta_error / dt_secs)
            }
        };

        // First-order low-pass filter on derivative action
        let d = if self.config.derivative_filter_n > 0.0 && self.config.kd > 0.0 {
            let tau = if self.config.kp > 0.0 {
                self.config.kd / (self.config.kp * self.config.derivative_filter_n)
            } else {
                // When Kp is zero, estimate tau directly from Kd and filter coefficient N
                // assuming nominal unit gain: tau = Kd / N
                self.config.kd / self.config.derivative_filter_n.max(1.0)
            };
            let alpha = (dt_secs / (tau + dt_secs)).clamp(0.0, 1.0);
            self.filtered_derivative = (1.0 - alpha) * self.filtered_derivative + alpha * d_raw;
            self.filtered_derivative
        } else {
            self.filtered_derivative = d_raw;
            d_raw
        };

        // 2. Candidate Integral Calculation
        let new_integral = self.integral + self.config.ki * error * dt_secs;

        // 3. Unclamped Output Estimation
        let raw_output = p + new_integral + d;
        let clamped = if raw_output.is_nan() {
            min_out
        } else {
            raw_output.clamp(min_out, max_out)
        };

        // 4. Anti-Windup Conditional Integration
        // If saturated high, allow integration only if error < 0 (unwinding).
        // If saturated low, allow integration only if error > 0 (unwinding).
        if self.config.anti_windup {
            if raw_output > max_out {
                if error < 0.0 {
                    self.integral = new_integral;
                }
            } else if raw_output < min_out {
                if error > 0.0 {
                    self.integral = new_integral;
                }
            } else {
                self.integral = new_integral;
            }
        } else {
            self.integral = new_integral;
        }

        self.prev_process_variable = process_variable;
        self.prev_error = error;

        clamped
    }

    /// Resets integrator and historical derivative states to zero.
    pub fn reset(&mut self) {
        self.integral = 0.0;
        self.prev_process_variable = 0.0;
        self.prev_error = 0.0;
        self.filtered_derivative = 0.0;
        self.initialized = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivative_on_measurement_suppresses_setpoint_step_kick() {
        let config = PidConfig {
            kp: 1.0,
            ki: 0.0,
            kd: 5.0,
            min_output: 0.0,
            max_output: 100.0,
            anti_windup: true,
            derivative_filter_n: 10.0,
            derivative_mode: DerivativeMode::OnMeasurement,
        };
        let mut pid = PidController::new(config);

        // Prime the initial measurement at 50.0
        pid.compute(50.0, 50.0, 1.0);

        // Step change setpoint from 50.0 to 100.0 with constant PV=50.0
        // Because PV did not move, derivative on measurement should not spike!
        let out = pid.compute(100.0, 50.0, 1.0);

        // Out should equal proportional term only (Kp * 50 = 50.0) without any Kd spike
        assert!((out - 50.0).abs() < 1e-4, "expected 50.0, got {out}");
    }

    #[test]
    fn anti_windup_unwinds_immediately_when_error_reverses() {
        let config_windup = PidConfig {
            kp: 0.1,
            ki: 1.0,
            kd: 0.0,
            min_output: 0.0,
            max_output: 10.0,
            anti_windup: false,
            derivative_filter_n: 10.0,
            derivative_mode: DerivativeMode::OnMeasurement,
        };
        let config_anti = PidConfig {
            anti_windup: true,
            ..config_windup.clone()
        };

        let mut pid_windup = PidController::new(config_windup);
        let mut pid_anti = PidController::new(config_anti);

        // Saturate both controllers for 20 steps with positive error
        for _ in 0..20 {
            pid_windup.compute(5.0, 0.0, 1.0);
            pid_anti.compute(5.0, 0.0, 1.0);
        }

        // Without anti-windup, integral accumulated heavily. With anti-windup, accumulation was frozen.
        assert!(pid_windup.integral > 50.0);
        assert!(pid_anti.integral <= 10.0);

        // Reverse error on the very next step (SP = 0.0, PV = 1.0 -> error = -1.0)
        let out_windup = pid_windup.compute(0.0, 1.0, 1.0);
        let out_anti = pid_anti.compute(0.0, 1.0, 1.0);

        // Controller with anti-windup immediately drops below saturation
        assert!(
            out_anti < 10.0,
            "Anti-windup controller must unwind immediately"
        );
        // Controller without anti-windup remains stuck at max_output
        assert_eq!(
            out_windup, 10.0,
            "Wound-up controller must remain saturated"
        );
    }

    #[test]
    fn proportional_output_tracks_the_error() {
        let config = PidConfig {
            kp: 2.0,
            ki: 0.0,
            kd: 0.0,
            min_output: 0.0,
            max_output: 100.0,
            anti_windup: true,
            derivative_filter_n: 10.0,
            derivative_mode: DerivativeMode::OnMeasurement,
        };
        let mut pid = PidController::new(config);

        // Proportional gain responds immediately to loop error
        let out = pid.compute(50.0, 30.0, 1.0);
        assert!((out - 40.0).abs() < 1e-6);
    }

    #[test]
    fn integral_term_accumulates_over_time() {
        let config = PidConfig {
            kp: 0.0,
            ki: 1.0,
            kd: 0.0,
            min_output: 0.0,
            max_output: 100.0,
            anti_windup: false,
            derivative_filter_n: 10.0,
            derivative_mode: DerivativeMode::OnMeasurement,
        };
        let mut pid = PidController::new(config);

        let out1 = pid.compute(10.0, 0.0, 1.0);
        assert!((out1 - 10.0).abs() < 1e-6);

        let out2 = pid.compute(10.0, 0.0, 1.0);
        assert!((out2 - 20.0).abs() < 1e-6);
    }

    #[test]
    fn anti_windup_freezes_integral_accumulation_during_saturation() {
        let config = PidConfig {
            kp: 10.0,
            ki: 5.0,
            kd: 0.0,
            min_output: 0.0,
            max_output: 100.0,
            anti_windup: true,
            derivative_filter_n: 10.0,
            derivative_mode: DerivativeMode::OnMeasurement,
        };
        let mut pid = PidController::new(config);

        let out = pid.compute(50.0, 0.0, 1.0);
        assert_eq!(out, 100.0);
        assert_eq!(pid.integral, 0.0);
    }

    #[test]
    fn derivative_mode_on_error_responds_to_error_rate_of_change() {
        let config = PidConfig {
            kp: 0.0,
            ki: 0.0,
            kd: 2.0,
            min_output: -100.0,
            max_output: 100.0,
            anti_windup: true,
            derivative_filter_n: 0.0, // bypass filter
            derivative_mode: DerivativeMode::OnError,
        };
        let mut pid = PidController::new(config);

        // Prime initial state
        pid.compute(0.0, 0.0, 2.0);

        // Step error from 0.0 to 10.0 over 2.0s -> derivative = 5.0 -> Kd * 5.0 = 10.0
        let out1 = pid.compute(10.0, 0.0, 2.0);
        assert!((out1 - 10.0).abs() < 1e-6);

        // Constant error on next step -> derivative = 0.0 -> Kd * 0 = 0.0
        let out2 = pid.compute(10.0, 0.0, 2.0);
        assert!((out2 - 0.0).abs() < 1e-6);
    }
}
