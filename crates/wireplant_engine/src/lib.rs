//! # WirePlant Engine (`wireplant_engine`)
//!
//! Discrete-time physics engine, industrial process loop models (Tank, Thermal, PID),
//! mathematical waveform generators, and ANSI/ISA-18.2 compliant alarm state machines.
//!
//! ## Execution Pipeline
//!
//! On each tick cycle, [`SimulationEngine`]:
//! 1. Snapshots operator-driven actuator input tags from [`wireplant_core::memory::PlantMemory`].
//! 2. Steps autonomous waveforms (sines, ramps, stochastic walks).
//! 3. Advances closed-loop process models ([`TankProcess`], [`ThermalProcess`], [`PidController`]).
//! 4. Evaluates reactive trip thresholds.
//! 5. Advances ISA-18.2 alarm state lifecycles.
//! 6. Commits calculated values atomically to the Process Image.

pub mod alarm;
pub mod evaluator;
pub mod historian;
pub mod process;
pub mod ticker;
pub mod waveforms;

pub use alarm::{
    ActiveAlarm, AlarmConfig, AlarmError, AlarmManager, AlarmSeverity, AlarmType, IsaAlarmState,
};
pub use evaluator::evaluate_threshold;
pub use historian::{Historian, HistorianConfig, HistorianError, HistorianTagMeta};
pub use process::{
    ActiveProcess, PidConfig, PidController, ProcessModelSpec, TankModelConfig, TankProcess,
    ThermalModelConfig, ThermalProcess,
};
pub use ticker::{
    EngineTickEvent, RealTimeClock, SimulationClock, SimulationEngine, SimulationEngineBuilder,
    TagBinding, VirtualTestClock,
};
pub use waveforms::{ActiveWaveform, WaveformSpec};
