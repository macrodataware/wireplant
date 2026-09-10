use wireplant_engine::process::PidConfig;

/// Points held per tag in the trend view.
pub const SPARKLINE_CAPACITY: usize = 100;

/// Lines kept in the event log. `ui.rs` prints this in the panel title.
pub const LOG_CAPACITY: usize = 500;

/// Active tab in the TUI navigation bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    ProcessTags,
    Alarms,
    ProcessControl,
    NetworkSessions,
    MemoryMap,
    FaultStudio,
    EventLog,
}

impl ActiveTab {
    pub const ALL: [ActiveTab; 7] = [
        ActiveTab::ProcessTags,
        ActiveTab::Alarms,
        ActiveTab::ProcessControl,
        ActiveTab::NetworkSessions,
        ActiveTab::MemoryMap,
        ActiveTab::FaultStudio,
        ActiveTab::EventLog,
    ];

    pub fn index(self) -> usize {
        ActiveTab::ALL
            .iter()
            .position(|tab| *tab == self)
            .expect("every variant is in ALL")
    }

    pub fn next(self) -> ActiveTab {
        ActiveTab::ALL[(self.index() + 1) % ActiveTab::ALL.len()]
    }

    pub fn prev(self) -> ActiveTab {
        ActiveTab::ALL[(self.index() + ActiveTab::ALL.len() - 1) % ActiveTab::ALL.len()]
    }
}

/// Filter for memory areas on Tab 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagAreaFilter {
    All,
    HoldingRegister,
    InputRegister,
    Coil,
    DiscreteInput,
}

impl TagAreaFilter {
    pub fn label(self) -> &'static str {
        match self {
            TagAreaFilter::All => "ALL",
            TagAreaFilter::HoldingRegister => "HOLDING_REG",
            TagAreaFilter::InputRegister => "INPUT_REG",
            TagAreaFilter::Coil => "COIL",
            TagAreaFilter::DiscreteInput => "DISCRETE_IN",
        }
    }

    pub fn next(self) -> Self {
        match self {
            TagAreaFilter::All => TagAreaFilter::HoldingRegister,
            TagAreaFilter::HoldingRegister => TagAreaFilter::InputRegister,
            TagAreaFilter::InputRegister => TagAreaFilter::Coil,
            TagAreaFilter::Coil => TagAreaFilter::DiscreteInput,
            TagAreaFilter::DiscreteInput => TagAreaFilter::All,
        }
    }
}

/// Memory table selected on Tab 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTableSelection {
    Coils,
    DiscreteInputs,
    InputRegisters,
    HoldingRegisters,
}

impl MemoryTableSelection {
    pub fn label(self) -> &'static str {
        match self {
            MemoryTableSelection::Coils => "Coils (0x0000..)",
            MemoryTableSelection::DiscreteInputs => "Discrete Inputs (1x0000..)",
            MemoryTableSelection::InputRegisters => "Input Registers (3x0000..)",
            MemoryTableSelection::HoldingRegisters => "Holding Registers (4x0000..)",
        }
    }

    pub fn next(self) -> Self {
        match self {
            MemoryTableSelection::Coils => MemoryTableSelection::DiscreteInputs,
            MemoryTableSelection::DiscreteInputs => MemoryTableSelection::InputRegisters,
            MemoryTableSelection::InputRegisters => MemoryTableSelection::HoldingRegisters,
            MemoryTableSelection::HoldingRegisters => MemoryTableSelection::Coils,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            MemoryTableSelection::Coils => MemoryTableSelection::HoldingRegisters,
            MemoryTableSelection::DiscreteInputs => MemoryTableSelection::Coils,
            MemoryTableSelection::InputRegisters => MemoryTableSelection::DiscreteInputs,
            MemoryTableSelection::HoldingRegisters => MemoryTableSelection::InputRegisters,
        }
    }
}

/// Filter level for the system event log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFilterLevel {
    All,
    WarnAndError,
    ErrorOnly,
}

impl LogFilterLevel {
    pub fn label(self) -> &'static str {
        match self {
            LogFilterLevel::All => "ALL",
            LogFilterLevel::WarnAndError => "WARN+ERROR",
            LogFilterLevel::ErrorOnly => "ERROR_ONLY",
        }
    }

    pub fn next(self) -> Self {
        match self {
            LogFilterLevel::All => LogFilterLevel::WarnAndError,
            LogFilterLevel::WarnAndError => LogFilterLevel::ErrorOnly,
            LogFilterLevel::ErrorOnly => LogFilterLevel::All,
        }
    }
}

/// Operational state mode of the TUI application.
#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    ValueInputModal {
        tag_id: String,
        input_buffer: String,
        duration_buffer: String,
        is_discrete: bool,
    },
    FaultProfileMenu {
        selected_index: usize,
    },
    ShelveAlarmModal {
        alarm_id: String,
        duration_buffer: String,
    },
    TagSearchModal {
        query_buffer: String,
    },
    HelpOverlay,
}

/// Severity of an event log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub const fn label(self) -> &'static str {
        match self {
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }
}

/// Structured entry for the event log buffer.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp_secs: u64,
    pub message: String,
    pub level: LogLevel,
}

/// A value the operator has pinned, and when the pin lifts.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveOverride {
    pub value: f64,
    /// Simulation seconds since the app started, or `None` to hold until cleared.
    pub expires_at_secs: Option<f64>,
}

/// Real-time snapshot of a tank process.
#[derive(Debug, Clone)]
pub struct TankProcessState {
    pub level_tag: String,
    pub inlet_valve_tag: String,
    pub drain_pump_tag: String,
    pub current_level: f64,
    pub max_height: f64,
    pub surface_area: f64,
    pub inflow_rate: f64,
    pub outflow_rate: f64,
    pub gravity_drain_coeff: f64,
}

/// Real-time snapshot of a thermal process.
#[derive(Debug, Clone)]
pub struct ThermalProcessState {
    pub temp_tag: String,
    pub heater_tag: String,
    pub current_temp: f64,
    pub ambient_temp: f64,
    pub heater_input: f64,
}

/// Real-time snapshot of a closed-loop PID controller.
#[derive(Debug, Clone)]
pub struct PidLoopState {
    pub setpoint_tag: String,
    pub process_var_tag: String,
    pub output_tag: String,
    pub setpoint: f64,
    pub process_variable: f64,
    pub output: f64,
    pub error: f64,
    pub config: PidConfig,
}

/// Diagnostic summary for a network or storage subsystem.
#[derive(Debug, Clone)]
pub struct NetworkServiceDiag {
    pub name: String,
    pub endpoint: String,
    pub is_enabled: bool,
    pub status_text: String,
    pub details: String,
}
