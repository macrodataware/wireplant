use rand_distr::{Distribution, StandardNormal};
use serde::{Deserialize, Serialize};

/// Declarative specification of a signal generation curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WaveformSpec {
    /// Static, unchanging numeric constant.
    Constant { value: f64 },
    /// Harmonic sine oscillation between min and max.
    Sine {
        min: f64,
        max: f64,
        period_secs: f64,
    },
    /// Sawtooth linear ramp increasing by step_per_tick with wraparound.
    Ramp {
        min: f64,
        max: f64,
        step_per_tick: f64,
    },
    /// Bidirectional triangle wave oscillating linearly.
    Triangle {
        min: f64,
        max: f64,
        period_secs: f64,
    },
    /// Stochastic random walk with Gaussian drift and bounded clamp.
    RandomWalk {
        center: f64,
        max_jitter: f64,
        drift_sigma: f64,
    },
    /// Reactive trip alarm driven by another tag's threshold crossing.
    TripThreshold {
        source_tag: String,
        threshold: f64,
        #[serde(default)]
        inverted: bool,
    },
}

/// Runtime instance holding current value and elapsed simulation time.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveWaveform {
    pub spec: WaveformSpec,
    pub current_value: f64,
    pub elapsed_secs: f64,
}

impl ActiveWaveform {
    /// Seeds `current_value` so the first tick starts somewhere sensible instead
    /// of at zero: a sine starts at its midpoint, a ramp at its floor, a random
    /// walk at its centre.
    pub fn new(spec: WaveformSpec) -> Self {
        let initial_val = match &spec {
            WaveformSpec::Constant { value } => *value,
            WaveformSpec::Sine { min, max, .. } => (min + max) / 2.0,
            WaveformSpec::Ramp { min, .. } => *min,
            WaveformSpec::Triangle { min, .. } => *min,
            WaveformSpec::RandomWalk { center, .. } => *center,
            WaveformSpec::TripThreshold { .. } => 0.0,
        };
        Self {
            spec,
            current_value: initial_val,
            elapsed_secs: 0.0,
        }
    }

    /// Advances the waveform by `dt_secs` and returns the newly computed value.
    pub fn tick(&mut self, dt_secs: f64) -> f64 {
        self.elapsed_secs += dt_secs;
        match &self.spec {
            WaveformSpec::Constant { value } => *value,
            WaveformSpec::Sine {
                min,
                max,
                period_secs,
            } => {
                let amplitude = (max - min) / 2.0;
                let mid = min + amplitude;
                let theta = if *period_secs == 0.0 {
                    0.0
                } else {
                    (2.0 * std::f64::consts::PI * self.elapsed_secs) / period_secs
                };
                self.current_value = mid + amplitude * theta.sin();
                self.current_value
            }
            WaveformSpec::Ramp {
                min,
                max,
                step_per_tick,
            } => {
                self.current_value += step_per_tick;
                if self.current_value > *max {
                    self.current_value = *min;
                }
                self.current_value
            }
            WaveformSpec::Triangle {
                min,
                max,
                period_secs,
            } => {
                let amplitude = max - min;
                let phase = if *period_secs == 0.0 {
                    0.0
                } else {
                    (self.elapsed_secs % period_secs) / period_secs
                };
                if phase < 0.5 {
                    self.current_value = min + amplitude * (phase * 2.0);
                } else {
                    self.current_value = max - amplitude * ((phase - 0.5) * 2.0);
                }
                self.current_value
            }
            WaveformSpec::RandomWalk {
                center,
                max_jitter,
                drift_sigma,
            } => {
                let step = if *drift_sigma > 0.0 && dt_secs > 0.0 {
                    let mut rng = rand::rng();
                    let z0: f64 = StandardNormal.sample(&mut rng);

                    // Scale by drift_sigma and sqrt(dt_secs) for continuous Wiener process invariance
                    drift_sigma * dt_secs.sqrt() * z0
                } else {
                    0.0
                };

                let min_val = center - max_jitter;
                let max_val = center + max_jitter;
                self.current_value = (self.current_value + step).clamp(min_val, max_val);
                self.current_value
            }
            WaveformSpec::TripThreshold { .. } => {
                // Evaluated during cross-tag pass
                self.current_value
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_waveform_holds_steady() {
        let mut constant = ActiveWaveform::new(WaveformSpec::Constant { value: 42.5 });
        assert_eq!(constant.current_value, 42.5);

        for _ in 0..10 {
            assert_eq!(constant.tick(0.1), 42.5);
        }
        assert!((constant.elapsed_secs - 1.0).abs() < 1e-9);
    }

    #[test]
    fn sine_waveform_bounded() {
        let mut sine = ActiveWaveform::new(WaveformSpec::Sine {
            min: 100.0,
            max: 200.0,
            period_secs: 10.0,
        });

        assert_eq!(sine.current_value, 150.0);

        for _ in 0..100 {
            let val = sine.tick(0.1);
            assert!(
                (99.999..=200.001).contains(&val),
                "Sine value {val} out of bounds"
            );
        }
    }

    #[test]
    fn ramp_wraps_at_ceiling() {
        let mut ramp = ActiveWaveform::new(WaveformSpec::Ramp {
            min: 0.0,
            max: 10.0,
            step_per_tick: 3.0,
        });

        assert_eq!(ramp.current_value, 0.0);
        assert_eq!(ramp.tick(0.1), 3.0);
        assert_eq!(ramp.tick(0.1), 6.0);
        assert_eq!(ramp.tick(0.1), 9.0);
        assert_eq!(ramp.tick(0.1), 0.0); // Wrapped around
        assert_eq!(ramp.tick(0.1), 3.0);
    }

    #[test]
    fn triangle_waveform_reaches_peak_and_cycles() {
        let mut tri = ActiveWaveform::new(WaveformSpec::Triangle {
            min: 0.0,
            max: 100.0,
            period_secs: 10.0,
        });

        assert_eq!(tri.current_value, 0.0);

        // Quarter period: linear rise to midpoint
        let val_quarter = tri.tick(2.5);
        assert!(
            (val_quarter - 50.0).abs() < 1e-6,
            "Expected ~50.0, got {val_quarter}"
        );

        // Half period: peak value
        let val_half = tri.tick(2.5);
        assert!(
            (val_half - 100.0).abs() < 1e-6,
            "Expected ~100.0, got {val_half}"
        );

        // Three-quarter period: linear return
        let val_three_quarter = tri.tick(2.5);
        assert!(
            (val_three_quarter - 50.0).abs() < 1e-6,
            "Expected ~50.0, got {val_three_quarter}"
        );

        // Ensure bounded over a full sweep
        for _ in 0..100 {
            let val = tri.tick(0.1);
            assert!(
                (-0.001..=100.001).contains(&val),
                "Triangle value {val} out of bounds"
            );
        }
    }

    #[test]
    fn random_walk_stays_within_jitter_bounds() {
        let mut rw = ActiveWaveform::new(WaveformSpec::RandomWalk {
            center: 50.0,
            max_jitter: 5.0,
            drift_sigma: 1.0,
        });

        assert_eq!(rw.current_value, 50.0);

        for _ in 0..500 {
            let val = rw.tick(0.05);
            assert!(
                (45.0..=55.0).contains(&val),
                "RandomWalk value {val} exceeded bounds [45.0, 55.0]"
            );
        }
    }

    #[test]
    fn trip_threshold_tick_preserves_internal_state() {
        let mut trip = ActiveWaveform::new(WaveformSpec::TripThreshold {
            source_tag: "pressure".to_string(),
            threshold: 100.0,
            inverted: false,
        });
        assert_eq!(trip.current_value, 0.0);
        // Ticking TripThreshold directly preserves current_value
        trip.current_value = 1.0;
        assert_eq!(trip.tick(0.1), 1.0);
    }
}
