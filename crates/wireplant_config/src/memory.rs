//! How much Process Image a scenario asks for.
//!
//! One function, three callers: the binary builds the tables with it, and the
//! integration tests size theirs with it too, so a test can catch a sizing bug
//! instead of quietly disagreeing with production about where a tag lives.

use crate::schema::ScenarioConfig;
use wireplant_core::memory::PlantMemory;
use wireplant_core::types::MemoryArea;

/// Table lengths in the order `PlantMemory::new` wants them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessImageSizes {
    pub coils: usize,
    pub discrete_inputs: usize,
    pub input_registers: usize,
    pub holding_registers: usize,
}

impl ProcessImageSizes {
    pub fn allocate(&self) -> PlantMemory {
        PlantMemory::new(
            self.coils,
            self.discrete_inputs,
            self.input_registers,
            self.holding_registers,
        )
    }
}

/// Sizes each table from the highest address any tag reaches.
///
/// A floor and a margin rather than an exact fit, so a SCADA client polling an
/// address no tag declares gets a zero instead of an out-of-range exception.
/// The margin comes from the tag's own register span, which is why a `Float64`
/// at the last address still has all four of its words.
pub fn process_image_sizes(scenario: &ScenarioConfig) -> ProcessImageSizes {
    let floor = scenario.memory.min_table_registers;
    let padding = scenario.memory.padding_registers;
    let mut sizes = [floor; 4];

    for tag in &scenario.tags {
        let slot = match tag.area {
            MemoryArea::Coil => 0,
            MemoryArea::DiscreteInput => 1,
            MemoryArea::InputRegister => 2,
            MemoryArea::HoldingRegister => 3,
        };
        let end = tag.address as usize + tag.data_type.register_span();
        sizes[slot] = sizes[slot].max(end + padding);
    }

    // Alarms write into coils and holding registers directly, so their addresses
    // must count toward the allocated table size just like declared tags do.
    for alarm in &scenario.alarms {
        let coil_addrs = [
            alarm.active_coil_addr,
            alarm.unack_coil_addr,
            alarm.ack_coil_addr,
            alarm.reset_coil_addr,
        ];
        for addr in coil_addrs.into_iter().flatten() {
            sizes[0] = sizes[0].max(addr + 1 + padding);
        }

        if let Some(reg_addr) = alarm.state_reg_addr {
            sizes[3] = sizes[3].max(reg_addr + 1 + padding);
        }
    }

    ProcessImageSizes {
        coils: sizes[0],
        discrete_inputs: sizes[1],
        input_registers: sizes[2],
        holding_registers: sizes[3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ScenarioConfig;

    fn scenario(toml: &str) -> ScenarioConfig {
        ScenarioConfig::from_toml(toml).expect("fixture must parse")
    }

    #[test]
    fn an_empty_tag_list_still_gets_the_configured_floor() {
        let config = scenario(
            r#"
[facility]
name = "Empty"
tick_rate_ms = 100

[memory]
min_table_registers = 32
padding_registers = 4

[[tags]]
id = "unused"
area = "coil"
address = 0
data_type = "boolean"
waveform = { type = "constant", value = 0.0 }
"#,
        );

        let sizes = process_image_sizes(&config);

        assert_eq!(sizes.coils, 32);
        assert_eq!(sizes.holding_registers, 32);
    }

    /// The margin is measured from the end of the tag, so a four-word type at
    /// the highest address is what sets the table length.
    #[test]
    fn a_wide_tag_at_the_top_sets_the_table_length() {
        let config = scenario(
            r#"
[facility]
name = "Wide"
tick_rate_ms = 100

[memory]
min_table_registers = 8
padding_registers = 2

[[tags]]
id = "wide"
area = "input_register"
address = 100
data_type = "float64"
waveform = { type = "constant", value = 0.0 }
"#,
        );

        let sizes = process_image_sizes(&config);

        assert_eq!(sizes.input_registers, 106, "100 + 4 words + 2 padding");
        assert_eq!(sizes.coils, 8, "untouched tables keep the floor");
    }

    #[test]
    fn an_alarm_address_sets_the_table_length_past_the_floor() {
        let config = scenario(
            r#"
[facility]
name = "AlarmSized"
tick_rate_ms = 100

[memory]
min_table_registers = 10
padding_registers = 2

[[tags]]
id = "sensor"
area = "input_register"
address = 0
data_type = "float32"
waveform = { type = "constant", value = 50.0 }

[[alarms]]
id = "hi"
source_tag = "sensor"
alarm_type = "high"
limit = 80.0
active_coil_addr = 50
state_reg_addr = 60
"#,
        );

        let sizes = process_image_sizes(&config);

        assert_eq!(sizes.coils, 53, "active_coil_addr 50 + 1 bit + 2 padding");
        assert_eq!(
            sizes.holding_registers, 63,
            "state_reg_addr 60 + 1 word + 2 padding"
        );
    }
}
