//! Discrete-time physics engine and mathematical signal generators for WirePlant.

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
    EngineTickEvent, RealTimeClock, SimulationClock, SimulationEngine, TagBinding, VirtualTestClock,
};
pub use waveforms::{ActiveWaveform, WaveformSpec};
