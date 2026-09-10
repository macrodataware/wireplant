use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::{interval, MissedTickBehavior};
use tracing::{error, info};
use wireplant_core::codec::{decode_tag, encode_tag, TagAddress};
use wireplant_core::memory::SharedProcessImage;
use wireplant_core::overrides::{create_shared_overrides, SharedOverrides};
use wireplant_core::types::{Endianness, MemoryArea, TagDataType};

use crate::alarm::AlarmManager;
use crate::evaluator::evaluate_threshold;
use crate::process::ActiveProcess;
use crate::waveforms::{ActiveWaveform, WaveformSpec};

/// Clock trait permitting production real-time intervals or test virtual stepping.
pub trait SimulationClock: Send + Sync {
    /// How far the simulation advances on each tick.
    fn delta_time(&self) -> Duration;
    /// Awaits the next tick in production, or steps forward immediately in tests.
    fn tick(&mut self) -> impl std::future::Future<Output = ()> + Send + '_;
}

/// Production clock tied to monotonic tokio::time::interval.
pub struct RealTimeClock {
    interval: tokio::time::Interval,
    dt: Duration,
}

impl std::fmt::Debug for RealTimeClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealTimeClock")
            .field("dt", &self.dt)
            .finish()
    }
}

impl RealTimeClock {
    pub fn new(dt: Duration) -> Self {
        let mut interval = interval(dt);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        Self { interval, dt }
    }
}

impl SimulationClock for RealTimeClock {
    fn delta_time(&self) -> Duration {
        self.dt
    }

    async fn tick(&mut self) {
        self.interval.tick().await;
    }
}

/// Deterministic test clock advancing virtual time on command.
#[derive(Debug, Clone)]
pub struct VirtualTestClock {
    dt: Duration,
    total_elapsed: Duration,
}

impl VirtualTestClock {
    pub fn new(dt: Duration) -> Self {
        Self {
            dt,
            total_elapsed: Duration::ZERO,
        }
    }
}

impl SimulationClock for VirtualTestClock {
    fn delta_time(&self) -> Duration {
        self.dt
    }

    async fn tick(&mut self) {
        self.total_elapsed += self.dt;
    }
}

/// Metadata connecting an active waveform to a specific memory slot.
#[derive(Debug, Clone)]
pub struct TagBinding {
    pub id: String,
    pub area: MemoryArea,
    pub address: usize,
    pub data_type: TagDataType,
    pub endianness: Endianness,
    pub waveform: ActiveWaveform,
}

impl TagBinding {
    pub fn new(
        id: impl Into<String>,
        area: MemoryArea,
        address: usize,
        data_type: TagDataType,
        endianness: Endianness,
        waveform: ActiveWaveform,
    ) -> Self {
        Self {
            id: id.into(),
            area,
            address,
            data_type,
            endianness,
            waveform,
        }
    }
}

impl TagAddress for TagBinding {
    fn area(&self) -> MemoryArea {
        self.area
    }

    fn address(&self) -> usize {
        self.address
    }

    fn data_type(&self) -> TagDataType {
        self.data_type
    }

    fn endianness(&self) -> Endianness {
        self.endianness
    }
}

/// Broadcast event emitted to subscribers on every completed simulation tick.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineTickEvent {
    pub tick_number: u64,
    pub elapsed_secs: f64,
    pub tag_values: HashMap<String, f64>,
}

/// Owns the tag bindings, process models, and alarms, advancing all of them once per tick.
#[derive(Debug)]
pub struct SimulationEngine {
    pub bindings: Vec<TagBinding>,
    pub processes: Vec<ActiveProcess>,
    pub alarms: AlarmManager,
    pub overrides: SharedOverrides,
    shared_memory: SharedProcessImage,
    tick_rate_ms: u64,
    event_tx: watch::Sender<EngineTickEvent>,
}

/// RAII Guard that asserts liveness on creation and clears it on drop (even upon panic).
struct EngineLivenessGuard(Arc<AtomicBool>);

impl EngineLivenessGuard {
    fn new(flag: Arc<AtomicBool>) -> Self {
        flag.store(true, Ordering::SeqCst);
        Self(flag)
    }
}

impl Drop for EngineLivenessGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl SimulationEngine {
    /// Convenience constructor for a scenario with no `[[processes]]` at all —
    /// pure waveform generation onto a Process Image.
    pub fn new(
        bindings: Vec<TagBinding>,
        shared_memory: SharedProcessImage,
        tick_rate_ms: u64,
    ) -> (Self, watch::Receiver<EngineTickEvent>) {
        Self::new_with_processes(bindings, Vec::new(), shared_memory, tick_rate_ms)
    }

    /// Builds the engine and the `watch` receiver subscribers listen on.
    ///
    /// The channel is seeded with a tick-zero event so a subscriber that starts
    /// before the first real tick sees a well-formed frame instead of blocking.
    pub fn new_with_processes(
        bindings: Vec<TagBinding>,
        processes: Vec<ActiveProcess>,
        shared_memory: SharedProcessImage,
        tick_rate_ms: u64,
    ) -> (Self, watch::Receiver<EngineTickEvent>) {
        Self::new_with_all(
            bindings,
            processes,
            AlarmManager::default(),
            create_shared_overrides(),
            shared_memory,
            tick_rate_ms,
        )
    }

    /// Full constructor supporting explicit alarm manager and override configuration.
    pub fn new_with_all(
        bindings: Vec<TagBinding>,
        processes: Vec<ActiveProcess>,
        alarms: AlarmManager,
        overrides: SharedOverrides,
        shared_memory: SharedProcessImage,
        tick_rate_ms: u64,
    ) -> (Self, watch::Receiver<EngineTickEvent>) {
        let initial_event = EngineTickEvent {
            tick_number: 0,
            elapsed_secs: 0.0,
            tag_values: HashMap::new(),
        };
        let (event_tx, event_rx) = watch::channel(initial_event);
        (
            Self {
                bindings,
                processes,
                alarms,
                overrides,
                shared_memory,
                tick_rate_ms,
                event_tx,
            },
            event_rx,
        )
    }

    /// Names of every tag that a closed-loop process reads as an actuator input.
    ///
    /// The tick loop uses this to decide which binding values are operator-owned:
    /// an input tag's value comes from memory (written over Modbus/HTTP), not
    /// from its waveform, so Pass 2 must not overwrite it.
    fn collect_process_input_tags(&self) -> HashSet<String> {
        let mut set = HashSet::new();
        for process in &self.processes {
            for tag in process.input_tags() {
                set.insert(tag.to_string());
            }
        }
        set
    }

    /// Executes one discrete simulation step across all registered waveforms and process models.
    ///
    /// Execution follows a staged pipeline:
    /// 1. Snapshot operator-driven input tags from the shared Process Image.
    /// 2. Compute autonomous waveforms (e.g. sines, ramps, stochastic walks).
    /// 3. Step closed-loop physics processes (Tank, Thermal, PID) with current actuator inputs.
    /// 4. Evaluate reactive threshold trips against updated process values.
    /// 5. Acquire exclusive memory write lock to atomic commit all register changes.
    ///
    /// The memory write lock is acquired only during step 5 to minimize contention with
    /// external protocol clients (Modbus, OPC-UA, HTTP).
    pub async fn tick_once(
        &mut self,
        dt_secs: f64,
        tick_count: u64,
        total_elapsed: f64,
    ) -> HashMap<String, f64> {
        // Step 0: Expire any timed overrides that have passed their deadline
        {
            let mut overrides_guard = self.overrides.write().await;
            let expired = overrides_guard.expire_elapsed(total_elapsed);
            for tag_id in expired {
                info!(
                    "Timed override expired for tag '{}' at t={:.2}s",
                    tag_id, total_elapsed
                );
            }
        }

        // Snapshot active overrides for this tick
        let active_overrides = self.overrides.read().await.snapshot_values();

        let mut calculated_values: HashMap<String, f64> =
            HashMap::with_capacity(self.bindings.len() + self.processes.len());

        // Step 1: Operator inputs: read memory value or active override
        let required_input_tags = self.collect_process_input_tags();
        if !required_input_tags.is_empty() {
            let memory = self.shared_memory.read().await;
            for binding in &self.bindings {
                if !required_input_tags.contains(&binding.id) {
                    continue;
                }

                if let Some(&forced) = active_overrides.get(&binding.id) {
                    calculated_values.insert(binding.id.clone(), forced);
                } else {
                    match decode_tag(&memory, binding) {
                        Ok(val) => {
                            calculated_values.insert(binding.id.clone(), val);
                        }
                        Err(e) => error!(
                            "Tag '{}' could not be read at {:?}[{}]: {}",
                            binding.id, binding.area, binding.address, e
                        ),
                    }
                }
            }
        }

        // Step 2: Autonomous waveforms advance, but overridden tags stay pinned
        for binding in &mut self.bindings {
            let is_trip = matches!(binding.waveform.spec, WaveformSpec::TripThreshold { .. });
            if !is_trip && !required_input_tags.contains(&binding.id) {
                if let Some(&forced) = active_overrides.get(&binding.id) {
                    // Tag is pinned by manual override! Do not overwrite with waveform.
                    binding.waveform.current_value = forced;
                    calculated_values.insert(binding.id.clone(), forced);
                } else {
                    let val = binding.waveform.tick(dt_secs);
                    calculated_values.insert(binding.id.clone(), val);
                }
            }
        }

        // Step 3: Closed-loop process models (tank, thermal, PID)
        for active in &mut self.processes {
            let (out_tag, out_val) = active.step(&calculated_values, dt_secs);
            let final_val = active_overrides.get(&out_tag).copied().unwrap_or(out_val);
            calculated_values.insert(out_tag.clone(), final_val);
            if let Some(b) = self.bindings.iter_mut().find(|b| b.id == out_tag) {
                b.waveform.current_value = final_val;
            }
        }

        // Step 4: Trip thresholds (evaluate against current calculated values)
        for binding in &mut self.bindings {
            if let WaveformSpec::TripThreshold {
                source_tag,
                threshold,
                inverted,
            } = &binding.waveform.spec
            {
                let trip_num = if let Some(&forced) = active_overrides.get(&binding.id) {
                    forced
                } else {
                    let source_val = calculated_values.get(source_tag).copied().unwrap_or(0.0);
                    let tripped = evaluate_threshold(source_val, *threshold, *inverted);
                    if tripped {
                        1.0
                    } else {
                        0.0
                    }
                };
                binding.waveform.current_value = trip_num;
                calculated_values.insert(binding.id.clone(), trip_num);
            }
        }

        // Step 5: Advance ISA-18.2 alarms
        self.alarms
            .step(
                &calculated_values,
                Duration::from_secs_f64(dt_secs),
                total_elapsed,
            )
            .await;

        // Step 6: Atomic commit pass to SharedProcessImage
        {
            let mut memory = self.shared_memory.write().await;
            for binding in &self.bindings {
                let is_operator_input = required_input_tags.contains(&binding.id);
                let is_process_output = self
                    .processes
                    .iter()
                    .any(|process| process.output_tag() == binding.id);
                let is_trip = matches!(binding.waveform.spec, WaveformSpec::TripThreshold { .. });
                let is_autonomous = !is_operator_input;
                let is_overridden = active_overrides.contains_key(&binding.id);

                // Commit if it is a process output, trip, autonomous waveform, OR overridden tag
                if !(is_process_output || is_trip || is_autonomous || is_overridden) {
                    continue;
                }

                let val = *calculated_values.get(&binding.id).unwrap_or(&0.0);
                if let Err(e) = encode_tag(&mut memory, binding, val) {
                    error!(
                        "Failed to write tag '{}' at {:?}[{}]: {}",
                        binding.id, binding.area, binding.address, e
                    );
                }
            }

            // Synchronize alarms with Process Image (coils, registers, Modbus pulses)
            self.alarms.sync_to_process_image(&mut memory).await;
        }

        // Broadcast tick event to subscribers
        let event = EngineTickEvent {
            tick_number: tick_count,
            elapsed_secs: total_elapsed,
            tag_values: calculated_values.clone(),
        };
        let _ = self.event_tx.send(event);

        calculated_values
    }

    /// Drives ticks off the wall clock at the configured rate with an optional liveness indicator.
    pub async fn run_supervised(
        self,
        cancel_rx: tokio::sync::broadcast::Receiver<()>,
        liveness: Arc<AtomicBool>,
    ) {
        let _guard = EngineLivenessGuard::new(liveness);
        self.run(cancel_rx).await;
    }

    /// Drives ticks off the wall clock at the configured rate.
    pub async fn run(self, cancel_rx: tokio::sync::broadcast::Receiver<()>) {
        let clock = RealTimeClock::new(Duration::from_millis(self.tick_rate_ms));
        self.run_with_clock(clock, cancel_rx).await;
    }

    /// The loop itself. Tests pass a [`VirtualTestClock`] here to step time
    /// deterministically instead of sleeping through it.
    pub async fn run_with_clock<C: SimulationClock>(
        mut self,
        mut clock: C,
        mut cancel_rx: tokio::sync::broadcast::Receiver<()>,
    ) {
        let dt_secs = clock.delta_time().as_secs_f64();
        let mut tick_count: u64 = 0;
        let mut total_elapsed: f64 = 0.0;

        info!(
            "Simulation engine running with clock interval {:?}",
            clock.delta_time()
        );

        loop {
            tokio::select! {
                _ = cancel_rx.recv() => {
                    info!("Simulation engine received cancellation signal, stopping loop");
                    break;
                }
                _ = clock.tick() => {
                    tick_count += 1;
                    total_elapsed += dt_secs;
                    self.tick_once(dt_secs, tick_count, total_elapsed).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{PidConfig, ProcessModelSpec, TankModelConfig};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tokio::sync::RwLock;
    use wireplant_core::endian::{float32_to_registers, registers_to_float32};
    use wireplant_core::memory::PlantMemory;

    /// The engine reads `delta_time` for the physics step and `total_elapsed`
    /// only when it broadcasts a tick, so the clock has to keep both in step.
    #[tokio::test]
    async fn the_virtual_clock_advances_by_exactly_one_delta_per_tick() {
        let mut clock = VirtualTestClock::new(Duration::from_millis(100));
        assert_eq!(clock.delta_time(), Duration::from_millis(100));

        clock.tick().await;
        clock.tick().await;
        assert_eq!(clock.total_elapsed, Duration::from_millis(200));
    }

    #[tokio::test]
    async fn one_tick_writes_its_waveforms_into_memory() {
        let memory = Arc::new(RwLock::new(PlantMemory::new(10, 10, 10, 10)));

        let bindings = vec![
            TagBinding::new(
                "pressure",
                MemoryArea::HoldingRegister,
                0,
                TagDataType::Float32,
                Endianness::BigEndian,
                ActiveWaveform::new(WaveformSpec::Constant { value: 150.0 }),
            ),
            TagBinding::new(
                "temperature",
                MemoryArea::InputRegister,
                2,
                TagDataType::Float32,
                Endianness::BigEndian,
                ActiveWaveform::new(WaveformSpec::Constant { value: 75.5 }),
            ),
            TagBinding::new(
                "high_pressure_trip",
                MemoryArea::Coil,
                0,
                TagDataType::Boolean,
                Endianness::BigEndian,
                ActiveWaveform::new(WaveformSpec::TripThreshold {
                    source_tag: "pressure".to_string(),
                    threshold: 100.0,
                    inverted: false,
                }),
            ),
            TagBinding::new(
                "low_pressure_ok",
                MemoryArea::DiscreteInput,
                1,
                TagDataType::Boolean,
                Endianness::BigEndian,
                ActiveWaveform::new(WaveformSpec::TripThreshold {
                    source_tag: "pressure".to_string(),
                    threshold: 100.0,
                    inverted: true,
                }),
            ),
        ];

        let (mut engine, mut event_rx) = SimulationEngine::new(bindings, Arc::clone(&memory), 100);

        let calculated = engine.tick_once(0.1, 1, 0.1).await;
        assert_eq!(*calculated.get("pressure").unwrap(), 150.0);
        assert_eq!(*calculated.get("temperature").unwrap(), 75.5);
        assert_eq!(*calculated.get("high_pressure_trip").unwrap(), 1.0);
        assert_eq!(*calculated.get("low_pressure_ok").unwrap(), 0.0);

        // Verify shared memory was written
        let mem = memory.read().await;
        // Holding registers 0 and 1
        let r0 = mem
            .read_registers_slice(MemoryArea::HoldingRegister, 0, 1)
            .unwrap()[0];
        let r1 = mem
            .read_registers_slice(MemoryArea::HoldingRegister, 1, 1)
            .unwrap()[0];
        let pressure_f32 = registers_to_float32(r0, r1, Endianness::BigEndian);
        assert!((pressure_f32 - 150.0).abs() < 1e-4);

        // Input registers 2 and 3
        let ir0 = mem
            .read_registers_slice(MemoryArea::InputRegister, 2, 1)
            .unwrap()[0];
        let ir1 = mem
            .read_registers_slice(MemoryArea::InputRegister, 3, 1)
            .unwrap()[0];
        let temp_f32 = registers_to_float32(ir0, ir1, Endianness::BigEndian);
        assert!((temp_f32 - 75.5).abs() < 1e-4);

        // Coil 0
        assert!(mem.read_bit(MemoryArea::Coil, 0).unwrap());

        // Discrete Input 1
        assert!(!mem.read_bit(MemoryArea::DiscreteInput, 1).unwrap());

        // Verify watch event
        let evt = event_rx.borrow_and_update();
        assert_eq!(evt.tick_number, 1);
        assert!((evt.elapsed_secs - 0.1).abs() < 1e-9);
        assert_eq!(*evt.tag_values.get("pressure").unwrap(), 150.0);
    }

    #[tokio::test]
    async fn a_tank_and_pid_step_together_and_commit_their_outputs() {
        let memory = Arc::new(RwLock::new(PlantMemory::new(10, 10, 10, 10)));

        // Pre-write inlet_valve = 1.0 (100% open) into holding register 0
        {
            let mut mem = memory.write().await;
            let (r0, r1) = float32_to_registers(1.0, Endianness::BigEndian);
            mem.write_registers_slice(MemoryArea::HoldingRegister, 0, &[r0])
                .unwrap();
            mem.write_registers_slice(MemoryArea::HoldingRegister, 1, &[r1])
                .unwrap();
        }

        let bindings = vec![
            TagBinding::new(
                "inlet_valve",
                MemoryArea::HoldingRegister,
                0,
                TagDataType::Float32,
                Endianness::BigEndian,
                ActiveWaveform::new(WaveformSpec::Constant { value: 0.0 }),
            ),
            TagBinding::new(
                "drain_pump",
                MemoryArea::HoldingRegister,
                2,
                TagDataType::Float32,
                Endianness::BigEndian,
                ActiveWaveform::new(WaveformSpec::Constant { value: 0.0 }),
            ),
            TagBinding::new(
                "tank_level",
                MemoryArea::InputRegister,
                0,
                TagDataType::Float32,
                Endianness::BigEndian,
                ActiveWaveform::new(WaveformSpec::Constant { value: 0.0 }),
            ),
        ];

        let tank_cfg = TankModelConfig {
            surface_area_m2: 1.0,
            max_height_m: 10.0,
            max_inflow_m3_s: 2.0,
            max_outflow_m3_s: 1.0,
            gravity_drain_coeff: 0.0,
            initial_level_m: Some(0.0),
        };
        let tank_spec = ProcessModelSpec::Tank {
            level_tag: "tank_level".to_string(),
            inlet_valve_tag: "inlet_valve".to_string(),
            drain_pump_tag: "drain_pump".to_string(),
            config: tank_cfg,
        };

        let pid_cfg = PidConfig {
            kp: 1.0,
            ki: 0.0,
            kd: 0.0,
            min_output: 0.0,
            max_output: 1.0,
            anti_windup: true,
            ..Default::default()
        };
        let pid_spec = ProcessModelSpec::PidController {
            setpoint_tag: "setpoint".to_string(),
            process_var_tag: "tank_level".to_string(),
            output_tag: "pid_output".to_string(),
            config: pid_cfg,
        };

        let processes = vec![
            ActiveProcess::from_spec(tank_spec),
            ActiveProcess::from_spec(pid_spec),
        ];

        let (mut engine, _) =
            SimulationEngine::new_with_processes(bindings, processes, Arc::clone(&memory), 100);

        // Tick 1s: with inlet_valve read as 1.0 from holding reg 0, inflow = 2.0 m3/s * 1s = 2.0 m3
        // Surface area 1.0 -> level = 2.0 m
        let vals = engine.tick_once(1.0, 1, 1.0).await;
        assert_eq!(*vals.get("tank_level").unwrap(), 2.0);

        // Check input register was updated with 2.0
        let mem = memory.read().await;
        let r0 = mem
            .read_registers_slice(MemoryArea::InputRegister, 0, 1)
            .unwrap()[0];
        let r1 = mem
            .read_registers_slice(MemoryArea::InputRegister, 1, 1)
            .unwrap()[0];
        let level_f32 = registers_to_float32(r0, r1, Endianness::BigEndian);
        assert!((level_f32 - 2.0).abs() < 1e-4);
    }

    #[tokio::test]
    async fn the_engine_stops_when_its_cancel_signal_fires() {
        let memory = Arc::new(RwLock::new(PlantMemory::new(10, 10, 10, 10)));
        let bindings = vec![TagBinding::new(
            "flow_rate",
            MemoryArea::HoldingRegister,
            0,
            TagDataType::Float32,
            Endianness::BigEndian,
            ActiveWaveform::new(WaveformSpec::Ramp {
                min: 0.0,
                max: 100.0,
                step_per_tick: 10.0,
            }),
        )];

        let (engine, mut event_rx) = SimulationEngine::new(bindings, Arc::clone(&memory), 10);
        let (cancel_tx, cancel_rx) = broadcast::channel(1);

        let handle = tokio::spawn(async move {
            engine.run(cancel_rx).await;
        });

        // Wait for at least 2 ticks
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Verify tick events were received
        {
            let evt = event_rx.borrow_and_update();
            assert!(evt.tick_number >= 1);
        }

        // Cancel
        cancel_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_millis(200), handle)
            .await
            .expect("Engine should terminate cleanly upon cancel signal")
            .unwrap();
    }
}
