use wireplant_config::schema::ScenarioConfig;
use wireplant_config::validation::validate_scenario;
use wireplant_core::types::MemoryArea;
use wireplant_engine::process::ActiveProcess;
use wireplant_engine::waveforms::WaveformSpec;
use wireplant_engine::{ActiveWaveform, SimulationEngine, TagBinding};

mod common;

fn load_and_validate_scenario(toml_str: &str) -> ScenarioConfig {
    let config: ScenarioConfig = toml::from_str(toml_str).expect("Scenario TOML parsing failed");
    validate_scenario(&config).expect("validate_scenario failed for scenario");
    config
}

async fn run_scenario_conformance_test(
    toml_str: &str,
    expected_facility: &str,
    expected_tag_count: usize,
) {
    let scenario = load_and_validate_scenario(toml_str);
    assert_eq!(scenario.facility.name, expected_facility);
    assert_eq!(scenario.tags.len(), expected_tag_count);

    // Sized by the binary's own rule, so a layout bug here is a layout bug in
    // production instead of a second opinion that always agrees.
    let memory = common::process_image(&scenario);

    // Create tag bindings
    let bindings: Vec<TagBinding> = scenario
        .tags
        .iter()
        .map(|tag| {
            TagBinding::new(
                tag.id.clone(),
                tag.area,
                tag.address as usize,
                tag.data_type,
                tag.endianness,
                ActiveWaveform::new(tag.waveform.clone()),
            )
        })
        .collect();

    // The scenario's processes run too, not just its waveforms: a conformance
    // test that skipped them would report a tank level that never moves.
    let processes = scenario
        .processes
        .iter()
        .map(|spec| ActiveProcess::from_spec(spec.clone()))
        .collect();

    let dt = (scenario.facility.tick_rate_ms as f64) / 1000.0;
    let (mut engine, _) = SimulationEngine::builder(
        memory.clone(),
        scenario.facility.tick_rate_ms,
    )
    .with_bindings(bindings)
    .with_processes(processes)
    .build();

    // Spawn a tick cycle
    let tag_values = engine.tick_once(dt, 1, dt).await;
    assert_eq!(tag_values.len(), scenario.tags.len());

    // Every tag keeps its shape tick over tick, and the value the engine
    // computed is the value a protocol client would read back.
    let first = engine.tick_once(dt, 1, dt).await;
    let mut moved = false;

    for tick in 2..12u64 {
        let values = engine.tick_once(dt, tick, dt * tick as f64).await;
        let mem = memory.read().await;

        for tag in &scenario.tags {
            let value = *values
                .get(&tag.id)
                .unwrap_or_else(|| panic!("Tag '{}' missing from calculated values", tag.id));

            // A process output is driven by that process, not by the
            // waveform it declares as a starting value, so the shape check
            // only applies to tags nothing else owns.
            if !is_process_owned(&scenario, &tag.id) {
                assert_within_waveform_range(tag, value);
            }
            if !is_operator_input(&scenario, &tag.id) {
                assert_memory_matches(&mem, tag, value);
            }

            if first.get(&tag.id) != Some(&value) {
                moved = true;
            }
        }
    }

    assert!(
        moved,
        "no tag changed across eleven ticks, so nothing under test actually advanced"
    );
}

/// The value a tag reports lies inside the range its waveform declares.
fn assert_within_waveform_range(tag: &wireplant_config::schema::TagConfig, value: f64) {
    let in_range = |lo: f64, hi: f64| value >= lo - 1e-3 && value <= hi + 1e-3;

    match &tag.waveform {
        WaveformSpec::Constant { value: expected } => assert!(
            (value - expected).abs() < 1e-4,
            "Tag {}: constant expected {expected}, got {value}",
            tag.id
        ),
        WaveformSpec::Sine { min, max, .. }
        | WaveformSpec::Ramp { min, max, .. }
        | WaveformSpec::Triangle { min, max, .. } => assert!(
            in_range(*min, *max),
            "Tag {}: value {value} outside [{min}, {max}]",
            tag.id
        ),
        WaveformSpec::RandomWalk {
            center, max_jitter, ..
        } => assert!(
            in_range(center - max_jitter, center + max_jitter),
            "Tag {}: random walk value {value} outside its jitter band",
            tag.id
        ),
        WaveformSpec::TripThreshold { .. } => assert!(
            value == 0.0 || value == 1.0,
            "Tag {}: a trip must be 0.0 or 1.0, got {value}",
            tag.id
        ),
    }
}

/// Whether a closed-loop process writes this tag as its output.
fn is_process_owned(scenario: &ScenarioConfig, tag_id: &str) -> bool {
    scenario.processes.iter().any(|process| match process {
        wireplant_engine::process::ProcessModelSpec::Tank { level_tag, .. } => level_tag == tag_id,
        wireplant_engine::process::ProcessModelSpec::Thermal { temp_tag, .. } => temp_tag == tag_id,
        wireplant_engine::process::ProcessModelSpec::PidController { output_tag, .. } => {
            output_tag == tag_id
        }
    })
}

/// Whether a closed-loop process owns this tag as an actuator.
///
/// Operator inputs are whatever the operator last wrote, so the engine leaves
/// them alone; asserting a value for one would be asserting the fixture.
fn is_operator_input(scenario: &ScenarioConfig, tag_id: &str) -> bool {
    scenario.processes.iter().any(|process| match process {
        wireplant_engine::process::ProcessModelSpec::Tank {
            inlet_valve_tag,
            drain_pump_tag,
            ..
        } => inlet_valve_tag == tag_id || drain_pump_tag == tag_id,
        wireplant_engine::process::ProcessModelSpec::Thermal { heater_tag, .. } => {
            heater_tag == tag_id
        }
        wireplant_engine::process::ProcessModelSpec::PidController {
            setpoint_tag,
            process_var_tag,
            ..
        } => setpoint_tag == tag_id || process_var_tag == tag_id,
    })
}

/// The Process Image holds the value the engine just reported for this tag.
fn assert_memory_matches(
    mem: &wireplant_core::memory::PlantMemory,
    tag: &wireplant_config::schema::TagConfig,
    value: f64,
) {
    match tag.area {
        MemoryArea::Coil => assert_eq!(
            mem.read_bit(MemoryArea::Coil, tag.address as usize)
                .unwrap(),
            value >= 0.5,
            "coil for {}",
            tag.id
        ),
        MemoryArea::DiscreteInput => assert_eq!(
            mem.read_bit(MemoryArea::DiscreteInput, tag.address as usize)
                .unwrap(),
            value >= 0.5,
            "discrete input for {}",
            tag.id
        ),
        area => {
            let span = tag.data_type.register_span();
            let words = mem
                .read_registers_slice(area, tag.address as usize, span)
                .unwrap();
            let decoded = wireplant_core::codec::decode(
                mem,
                area,
                tag.address as usize,
                tag.data_type,
                tag.endianness,
            )
            .unwrap();
            // The wire is 32-bit and the engine works in f64, so the
            // tolerance scales with the value rather than being a fixed 1e-3.
            let tolerance = 1e-3 * value.abs().max(1.0);
            assert!(
                (decoded - value).abs() <= tolerance && !words.iter().all(|w| *w == 0),
                "{} reads back as {decoded}, engine computed {value}",
                tag.id
            );
        }
    }
}

#[tokio::test]
async fn the_default_hydro_scenario_loads_and_its_values_reach_memory() {
    run_scenario_conformance_test(
        include_str!("../scenarios/default_hydro.toml"),
        "Hydroelectric Substation 04",
        4,
    )
    .await;
}

#[tokio::test]
async fn the_water_treatment_scenario_loads_and_its_values_reach_memory() {
    run_scenario_conformance_test(
        include_str!("../scenarios/water_treatment.toml"),
        "ClearFlow Municipal Water Filtration Plant",
        15,
    )
    .await;
}

#[tokio::test]
async fn the_petrochemical_scenario_loads_and_its_values_reach_memory() {
    run_scenario_conformance_test(
        include_str!("../scenarios/petrochemical.toml"),
        "Gulf Coast Refinery - Unit 12 Fluid Catalytic Cracker",
        16,
    )
    .await;
}

#[tokio::test]
async fn the_closed_loop_water_plant_scenario_loads_and_its_values_reach_memory() {
    run_scenario_conformance_test(
        include_str!("../scenarios/closed_loop_water_plant.toml"),
        "ClearLake Water Treatment & Storage",
        5,
    )
    .await;
}

/// Range membership alone cannot tell a ramp from a frozen constant, so each
/// waveform shape gets a known-answer check: exact values at exact phases.
mod waveform_shapes {
    use wireplant_engine::waveforms::{ActiveWaveform, WaveformSpec};

    #[test]
    fn a_ramp_climbs_by_its_step_and_wraps_to_the_floor() {
        let mut ramp = ActiveWaveform::new(WaveformSpec::Ramp {
            min: 10.0,
            max: 25.0,
            step_per_tick: 4.0,
        });

        assert_eq!(ramp.tick(0.1), 14.0);
        assert_eq!(ramp.tick(0.1), 18.0);
        assert_eq!(ramp.tick(0.1), 22.0);
        // The next step would pass the ceiling, so it restarts at the floor.
        assert_eq!(ramp.tick(0.1), 10.0);
    }

    #[test]
    fn a_sine_hits_the_midpoint_at_zero_and_the_peak_a_quarter_period_later() {
        let mut sine = ActiveWaveform::new(WaveformSpec::Sine {
            min: 100.0,
            max: 200.0,
            period_secs: 8.0,
        });

        assert_eq!(sine.tick(0.0), 150.0, "a sine starts at its midpoint");
        assert!(
            (sine.tick(2.0) - 200.0).abs() < 1e-9,
            "a quarter period in should be the peak"
        );
        assert!(
            (sine.tick(2.0) - 150.0).abs() < 1e-9,
            "half a period in should be back at the midpoint"
        );
        assert!(
            (sine.tick(2.0) - 100.0).abs() < 1e-9,
            "three quarters in should be the trough"
        );
        assert!(
            (sine.tick(2.0) - 150.0).abs() < 1e-9,
            "a full period should return to the midpoint"
        );
    }

    #[test]
    fn a_triangle_rises_to_the_peak_at_half_a_period_and_falls_back() {
        let mut triangle = ActiveWaveform::new(WaveformSpec::Triangle {
            min: 0.0,
            max: 40.0,
            period_secs: 8.0,
        });

        assert_eq!(triangle.tick(2.0), 20.0);
        assert_eq!(triangle.tick(2.0), 40.0);
        assert_eq!(triangle.tick(2.0), 20.0);
        assert_eq!(triangle.tick(2.0), 0.0);
    }

    #[test]
    fn a_constant_never_moves() {
        let mut constant = ActiveWaveform::new(WaveformSpec::Constant { value: -3.5 });

        for _ in 0..20 {
            assert_eq!(constant.tick(0.1), -3.5);
        }
    }

    /// A random walk is random, so the assertions are about its envelope and
    /// about it actually moving: a frozen implementation passes every range
    /// check and fails the second half of this test.
    #[test]
    fn a_random_walk_stays_inside_its_band_and_still_moves() {
        let mut walk = ActiveWaveform::new(WaveformSpec::RandomWalk {
            center: 50.0,
            max_jitter: 5.0,
            drift_sigma: 1.0,
        });

        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let value = walk.tick(0.1);
            assert!(
                (45.0..=55.0).contains(&value),
                "random walk left its band at {value}"
            );
            seen.insert(value.to_bits());
        }

        assert!(seen.len() > 1, "the walk never moved, so it is not walking");
    }
}
