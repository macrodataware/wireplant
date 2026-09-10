use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, RwLock};
use wireplant_core::endian::{float32_to_registers, registers_to_float32};
use wireplant_core::memory::PlantMemory;
use wireplant_core::types::{Endianness, MemoryArea, TagDataType};
use wireplant_engine::{
    ActiveWaveform, SimulationClock, SimulationEngine, TagBinding, VirtualTestClock, WaveformSpec,
};

#[tokio::test]
async fn the_engine_runs_a_scheduled_scenario_under_a_virtual_clock() {
    let memory = Arc::new(RwLock::new(PlantMemory::new(16, 16, 16, 16)));

    let bindings = vec![
        TagBinding::new(
            "temperature",
            MemoryArea::HoldingRegister,
            0,
            TagDataType::Float32,
            Endianness::BigEndian,
            ActiveWaveform::new(WaveformSpec::Ramp {
                min: 20.0,
                max: 100.0,
                step_per_tick: 10.0,
            }),
        ),
        TagBinding::new(
            "overheat_alarm",
            MemoryArea::Coil,
            5,
            TagDataType::Boolean,
            Endianness::BigEndian,
            ActiveWaveform::new(WaveformSpec::TripThreshold {
                source_tag: "temperature".to_string(),
                threshold: 50.0,
                inverted: false,
            }),
        ),
    ];

    let (mut engine, mut event_rx) = SimulationEngine::new(bindings, Arc::clone(&memory), 100);

    // Initial check on receiver before any ticks
    {
        let initial = event_rx.borrow();
        assert_eq!(initial.tick_number, 0);
    }

    // Step 1: tick (dt = 0.1s)
    // Temperature starts at min (20.0), step_per_tick is 10.0 -> 30.0.
    // 30.0 < 50.0 -> overheat_alarm = 0.0 (false)
    let vals1 = engine.tick_once(0.1, 1, 0.1).await;
    assert_eq!(*vals1.get("temperature").unwrap(), 30.0);
    assert_eq!(*vals1.get("overheat_alarm").unwrap(), 0.0);

    {
        let mem = memory.read().await;
        let r0 = mem
            .read_registers_slice(MemoryArea::HoldingRegister, 0, 1)
            .unwrap()[0];
        let r1 = mem
            .read_registers_slice(MemoryArea::HoldingRegister, 1, 1)
            .unwrap()[0];
        let val = registers_to_float32(r0, r1, Endianness::BigEndian);
        assert!((val - 30.0).abs() < 1e-4);
        assert!(!mem.read_bit(MemoryArea::Coil, 5).unwrap());
    }

    // Step 2: tick (dt = 0.1s, total = 0.2s)
    // Temperature -> 40.0. overheat_alarm -> 0.0 (false)
    let vals2 = engine.tick_once(0.1, 2, 0.2).await;
    assert_eq!(*vals2.get("temperature").unwrap(), 40.0);
    assert_eq!(*vals2.get("overheat_alarm").unwrap(), 0.0);

    // Step 3: tick (dt = 0.1s, total = 0.3s)
    // Temperature -> 50.0. overheat_alarm -> 1.0 (true, since >= 50.0)
    let vals3 = engine.tick_once(0.1, 3, 0.3).await;
    assert_eq!(*vals3.get("temperature").unwrap(), 50.0);
    assert_eq!(*vals3.get("overheat_alarm").unwrap(), 1.0);

    {
        let mem = memory.read().await;
        let r0 = mem
            .read_registers_slice(MemoryArea::HoldingRegister, 0, 1)
            .unwrap()[0];
        let r1 = mem
            .read_registers_slice(MemoryArea::HoldingRegister, 1, 1)
            .unwrap()[0];
        let val = registers_to_float32(r0, r1, Endianness::BigEndian);
        assert!((val - 50.0).abs() < 1e-4);
        assert!(mem.read_bit(MemoryArea::Coil, 5).unwrap());
    }

    // Verify watch notification updated to tick 3
    let evt = event_rx.borrow_and_update();
    assert_eq!(evt.tick_number, 3);
    assert_eq!(*evt.tag_values.get("temperature").unwrap(), 50.0);
    assert_eq!(*evt.tag_values.get("overheat_alarm").unwrap(), 1.0);
}

#[tokio::test]
async fn the_engine_publishes_ticks_and_stops_on_cancel() {
    let memory = Arc::new(RwLock::new(PlantMemory::new(10, 10, 10, 10)));
    let bindings = vec![TagBinding::new(
        "speed",
        MemoryArea::HoldingRegister,
        0,
        TagDataType::Float32,
        Endianness::BigEndian,
        ActiveWaveform::new(WaveformSpec::Constant { value: 1200.0 }),
    )];

    let (engine, mut event_rx) = SimulationEngine::new(bindings, Arc::clone(&memory), 10);
    let (cancel_tx, cancel_rx) = broadcast::channel(1);

    let virtual_clock = VirtualTestClock::new(Duration::from_millis(10));
    assert_eq!(virtual_clock.delta_time(), Duration::from_millis(10));

    let handle = tokio::spawn(async move {
        engine.run_with_clock(virtual_clock, cancel_rx).await;
    });

    // Wait for at least one tick to be published
    event_rx
        .changed()
        .await
        .expect("Expected at least one tick");
    assert!(event_rx.borrow().tick_number >= 1);

    // Send cancellation signal
    cancel_tx.send(()).unwrap();
    handle.await.expect("Engine loop should shut down cleanly");
}

#[tokio::test]
async fn a_coupled_tank_thermal_and_pid_loop_settles() {
    use wireplant_engine::process::{
        ActiveProcess, PidConfig, ProcessModelSpec, TankModelConfig, ThermalModelConfig,
    };

    let memory = Arc::new(RwLock::new(PlantMemory::new(32, 32, 32, 32)));

    // Set up a closed-loop water heating process:
    // Level target: 5.0m, tank surface area: 2.0 m2, heater heats tank water.
    let tank_spec = ProcessModelSpec::Tank {
        level_tag: "tank_level".to_string(),
        inlet_valve_tag: "inlet_valve".to_string(),
        drain_pump_tag: "drain_pump".to_string(),
        config: TankModelConfig {
            surface_area_m2: 2.0,
            max_height_m: 10.0,
            max_inflow_m3_s: 0.5,
            max_outflow_m3_s: 0.2,
            gravity_drain_coeff: 0.05,
            initial_level_m: Some(2.0),
        },
    };

    let thermal_spec = ProcessModelSpec::Thermal {
        temp_tag: "water_temp".to_string(),
        heater_tag: "heater_duty".to_string(),
        config: ThermalModelConfig {
            ambient_temp_c: 20.0,
            max_heating_power_watts: 5000.0,
            heat_capacity_j_k: 2500.0,
            heat_loss_coeff_w_k: 25.0,
            initial_temp_c: Some(20.0),
        },
    };

    let pid_spec = ProcessModelSpec::PidController {
        setpoint_tag: "target_temp".to_string(),
        process_var_tag: "water_temp".to_string(),
        output_tag: "heater_duty".to_string(),
        config: PidConfig {
            kp: 0.1,
            ki: 0.02,
            kd: 0.01,
            min_output: 0.0,
            max_output: 1.0,
            anti_windup: true,
            ..Default::default()
        },
    };

    // Note execution order: PID computes heater_duty from water_temp and target_temp,
    // then Thermal consumes heater_duty to update water_temp on the next step.
    let processes = vec![
        ActiveProcess::from_spec(pid_spec.clone()),
        ActiveProcess::from_spec(thermal_spec.clone()),
        ActiveProcess::from_spec(tank_spec.clone()),
    ];

    let bindings = vec![
        TagBinding::new(
            "inlet_valve",
            MemoryArea::HoldingRegister,
            0,
            TagDataType::Float32,
            Endianness::BigEndian,
            ActiveWaveform::new(WaveformSpec::Constant { value: 0.8 }),
        ),
        TagBinding::new(
            "drain_pump",
            MemoryArea::HoldingRegister,
            2,
            TagDataType::Float32,
            Endianness::BigEndian,
            ActiveWaveform::new(WaveformSpec::Constant { value: 0.1 }),
        ),
        TagBinding::new(
            "target_temp",
            MemoryArea::HoldingRegister,
            4,
            TagDataType::Float32,
            Endianness::BigEndian,
            ActiveWaveform::new(WaveformSpec::Constant { value: 65.0 }),
        ),
        TagBinding::new(
            "tank_level",
            MemoryArea::HoldingRegister,
            6,
            TagDataType::Float32,
            Endianness::BigEndian,
            ActiveWaveform::new(WaveformSpec::Constant { value: 2.0 }),
        ),
        TagBinding::new(
            "water_temp",
            MemoryArea::HoldingRegister,
            8,
            TagDataType::Float32,
            Endianness::BigEndian,
            ActiveWaveform::new(WaveformSpec::Constant { value: 20.0 }),
        ),
        TagBinding::new(
            "heater_duty",
            MemoryArea::HoldingRegister,
            10,
            TagDataType::Float32,
            Endianness::BigEndian,
            ActiveWaveform::new(WaveformSpec::Constant { value: 0.0 }),
        ),
    ];

    let (mut engine, _event_rx) =
        SimulationEngine::new_with_processes(bindings, processes, Arc::clone(&memory), 100);

    // In WirePlant's architecture, process actuator inputs are operator-owned:
    // the ticker reads them from shared memory (not waveform generator).
    // Pre-populate initial actuator setpoints in memory:
    {
        let mut mem = memory.write().await;
        // inlet_valve = 0.8 at register 0
        let (r0, r1) = float32_to_registers(0.8, Endianness::BigEndian);
        mem.write_registers_slice(MemoryArea::HoldingRegister, 0, &[r0])
            .unwrap();
        mem.write_registers_slice(MemoryArea::HoldingRegister, 1, &[r1])
            .unwrap();

        // drain_pump = 0.1 at register 2
        let (r2, r3) = float32_to_registers(0.1, Endianness::BigEndian);
        mem.write_registers_slice(MemoryArea::HoldingRegister, 2, &[r2])
            .unwrap();
        mem.write_registers_slice(MemoryArea::HoldingRegister, 3, &[r3])
            .unwrap();

        // target_temp = 65.0 at register 4
        let (r4, r5) = float32_to_registers(65.0, Endianness::BigEndian);
        mem.write_registers_slice(MemoryArea::HoldingRegister, 4, &[r4])
            .unwrap();
        mem.write_registers_slice(MemoryArea::HoldingRegister, 5, &[r5])
            .unwrap();

        // heater_duty = 1.0 at register 10
        let (r10, r11) = float32_to_registers(1.0, Endianness::BigEndian);
        mem.write_registers_slice(MemoryArea::HoldingRegister, 10, &[r10])
            .unwrap();
        mem.write_registers_slice(MemoryArea::HoldingRegister, 11, &[r11])
            .unwrap();
    }

    // Simulate 300 seconds in 1-second steps to allow PID loop to settle
    for step in 1..=300 {
        let values = engine.tick_once(1.0, step, step as f64).await;
        let level = *values.get("tank_level").unwrap();

        // Level must rise from initial 2.0m since inflow (0.8 * 0.5 = 0.4 m3/s) exceeds gravity + pump
        assert!(level > 2.0);
    }

    let final_values = engine.tick_once(1.0, 301, 301.0).await;
    let final_temp = *final_values.get("water_temp").unwrap();
    assert!(
        (final_temp - 65.0).abs() < 1.0,
        "Closed-loop PID must settle near target 65.0 °C (got {final_temp})"
    );
}
