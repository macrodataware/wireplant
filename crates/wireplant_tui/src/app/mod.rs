pub mod alarms;
pub mod diagnostics;
pub mod navigation;
pub mod overrides;
pub mod types;

#[cfg(test)]
mod tests;

pub use types::*;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind};
use futures::StreamExt;
use ratatui::widgets::TableState;
use ratatui::DefaultTerminal;
use tokio::sync::{mpsc, watch};

use wireplant_config::schema::ScenarioConfig;
use wireplant_core::codec::BOOL_THRESHOLD;
use wireplant_core::memory::{SessionCounter, SharedProcessImage};
use wireplant_core::overrides::SharedOverrides;
use wireplant_core::types::MemoryArea;
use wireplant_engine::alarm::{ActiveAlarm, AlarmManager};
use wireplant_engine::historian::Historian;
use wireplant_engine::ticker::EngineTickEvent;

use crate::ui::draw_ui;

/// Interactive TUI application state for WirePlant.
pub struct TuiApp {
    pub scenario: ScenarioConfig,
    pub shared_memory: SharedProcessImage,
    pub start_time: Instant,
    pub tick_rx: watch::Receiver<EngineTickEvent>,
    pub alarms: AlarmManager,
    pub sessions: SessionCounter,
    pub historian: Option<Arc<Historian>>,
    pub overrides: SharedOverrides,

    pub sparkline_data: HashMap<String, VecDeque<f64>>,
    pub is_running: bool,
    pub active_tab: ActiveTab,
    pub mode: AppMode,
    pub log_entries: VecDeque<LogEntry>,
    pub log_rx: Option<mpsc::UnboundedReceiver<LogEntry>>,
    pub active_overrides: HashMap<String, ActiveOverride>,

    // Cached alarm metrics updated on ticks
    pub cached_alarms: Vec<ActiveAlarm>,
    pub active_alarms_count: usize,
    pub unacknowledged_alarms_count: usize,

    // Navigation and stateful table states
    pub selected_tag_index: usize,
    pub tag_table_state: TableState,
    pub alarm_table_state: TableState,
    pub memory_table_state: TableState,
    pub log_table_state: TableState,

    // Filters and memory options
    pub tag_search_query: String,
    pub tag_area_filter: TagAreaFilter,
    pub memory_table_selected: MemoryTableSelection,
    pub memory_address_offset: usize,
    pub log_filter: LogFilterLevel,
    pub log_auto_scroll: bool,
}

impl std::fmt::Debug for TuiApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TuiApp")
            .field("is_running", &self.is_running)
            .field("active_tab", &self.active_tab)
            .field("mode", &self.mode)
            .finish()
    }
}

impl TuiApp {
    /// Creates the TUI state machine bound to shared process memory, alarm management,
    /// and telemetry channels.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scenario: ScenarioConfig,
        shared_memory: SharedProcessImage,
        tick_rx: watch::Receiver<EngineTickEvent>,
        alarms: AlarmManager,
        sessions: SessionCounter,
        historian: Option<Arc<Historian>>,
        overrides: SharedOverrides,
        log_rx: Option<mpsc::UnboundedReceiver<LogEntry>>,
    ) -> Self {
        let mut sparkline_data = HashMap::new();
        for tag in &scenario.tags {
            sparkline_data.insert(tag.id.clone(), VecDeque::with_capacity(SPARKLINE_CAPACITY));
        }

        let mut tag_table_state = TableState::default();
        tag_table_state.select(Some(0));

        let mut alarm_table_state = TableState::default();
        alarm_table_state.select(Some(0));

        let mut memory_table_state = TableState::default();
        memory_table_state.select(Some(0));

        let mut log_table_state = TableState::default();
        log_table_state.select(Some(0));

        let mut app = Self {
            scenario,
            shared_memory,
            start_time: Instant::now(),
            tick_rx,
            alarms,
            sessions,
            historian,
            overrides,
            sparkline_data,
            is_running: true,
            active_tab: ActiveTab::ProcessTags,
            mode: AppMode::Normal,
            log_entries: VecDeque::with_capacity(LOG_CAPACITY),
            log_rx,
            active_overrides: HashMap::new(),
            cached_alarms: Vec::new(),
            active_alarms_count: 0,
            unacknowledged_alarms_count: 0,
            selected_tag_index: 0,
            tag_table_state,
            alarm_table_state,
            memory_table_state,
            log_table_state,
            tag_search_query: String::new(),
            tag_area_filter: TagAreaFilter::All,
            memory_table_selected: MemoryTableSelection::HoldingRegisters,
            memory_address_offset: 0,
            log_filter: LogFilterLevel::All,
            log_auto_scroll: true,
        };

        app.sync_alarms_from_inner();
        app
    }

    /// Appends a new message to the event log ring buffer.
    pub fn add_log(&mut self, level: LogLevel, message: impl Into<String>) {
        if self.log_entries.len() >= LOG_CAPACITY {
            self.log_entries.pop_front();
        }
        self.log_entries.push_back(LogEntry {
            timestamp_secs: self.start_time.elapsed().as_secs(),
            message: message.into(),
            level,
        });
    }

    /// Pulls incoming tracing events from the channel into the log ring buffer.
    pub fn poll_incoming_logs(&mut self) {
        if let Some(rx) = &mut self.log_rx {
            while let Ok(entry) = rx.try_recv() {
                if self.log_entries.len() >= LOG_CAPACITY {
                    self.log_entries.pop_front();
                }
                self.log_entries.push_back(entry);
            }
        }
    }

    /// Pulls latest telemetry values from the simulation engine receiver into
    /// the sparkline ring buffers, and lifts overrides whose time is up.
    pub fn on_tick(&mut self, elapsed_secs: f64) {
        self.expire_overrides(elapsed_secs);
        self.poll_incoming_logs();

        let has_changed = self.tick_rx.has_changed().unwrap_or(false);
        let is_empty = self.sparkline_data.values().all(|b| b.is_empty());
        if !has_changed && !is_empty {
            return;
        }

        let event = self.tick_rx.borrow_and_update().clone();
        for (tag_id, value) in &event.tag_values {
            let shown = self
                .active_overrides
                .get(tag_id)
                .map_or(*value, |o| o.value);

            if let Some(buf) = self.sparkline_data.get_mut(tag_id) {
                push_capped(buf, shown);
            }
        }
    }

    /// Redraws and handles input until the user quits.
    pub async fn run_loop(
        mut self,
        mut terminal: DefaultTerminal,
        shutdown_tx: tokio::sync::broadcast::Sender<()>,
    ) -> Result<(), std::io::Error> {
        let mut event_stream = EventStream::new();
        let mut tick_timer = tokio::time::interval(std::time::Duration::from_millis(50));
        let mut input_error_logged = false;

        while self.is_running {
            let elapsed_secs = self.start_time.elapsed().as_secs_f64();
            self.on_tick(elapsed_secs);
            self.refresh_alarms().await;

            terminal.draw(|f| draw_ui(f, &mut self))?;

            tokio::select! {
                _ = tick_timer.tick() => {}
                maybe_event = event_stream.next() => {
                    match maybe_event {
                        Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                            self.handle_key(key.code).await;
                        }
                        Some(Err(e)) => {
                            if !input_error_logged {
                                self.add_log(LogLevel::Error, format!("Terminal input failed: {e}"));
                                input_error_logged = true;
                            }
                            self.is_running = false;
                        }
                        None => self.is_running = false,
                        Some(Ok(_)) => {}
                    }
                }
            }
        }

        if let Err(e) = shutdown_tx.send(()) {
            self.add_log(
                LogLevel::Error,
                format!("Shutdown signal had no listeners: {e}"),
            );
        }
        Ok(())
    }

    pub async fn handle_key(&mut self, code: KeyCode) {
        match &mut self.mode {
            AppMode::Normal => self.handle_normal_key(code).await,
            AppMode::ValueInputModal {
                input_buffer,
                duration_buffer,
                ..
            } => match code {
                KeyCode::Char(c) if c.is_ascii_digit() || c == '.' || c == '-' => {
                    input_buffer.push(c);
                }
                KeyCode::Backspace => {
                    input_buffer.pop();
                }
                KeyCode::Tab => {
                    // Quick duration toggle or tab
                    if duration_buffer.is_empty() {
                        duration_buffer.push_str("10");
                    } else {
                        duration_buffer.clear();
                    }
                }
                KeyCode::Enter => self.commit_override_modal(),
                KeyCode::Esc => self.mode = AppMode::Normal,
                _ => {}
            },
            AppMode::ShelveAlarmModal {
                duration_buffer, ..
            } => match code {
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    if duration_buffer.len() < 9 {
                        duration_buffer.push(c);
                    }
                }
                KeyCode::Backspace => {
                    duration_buffer.pop();
                }
                KeyCode::Enter => {
                    self.commit_shelve_modal().await;
                }
                KeyCode::Esc => self.mode = AppMode::Normal,
                _ => {}
            },
            AppMode::TagSearchModal { query_buffer } => match code {
                KeyCode::Char(c) => {
                    query_buffer.push(c);
                }
                KeyCode::Backspace => {
                    query_buffer.pop();
                }
                KeyCode::Enter => {
                    self.tag_search_query = query_buffer.clone();
                    self.selected_tag_index = 0;
                    self.tag_table_state.select(Some(0));
                    self.mode = AppMode::Normal;
                }
                KeyCode::Esc => {
                    self.mode = AppMode::Normal;
                }
                _ => {}
            },
            AppMode::HelpOverlay => match code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Enter => {
                    self.mode = AppMode::Normal;
                }
                _ => {}
            },
            AppMode::FaultProfileMenu { selected_index } => {
                let profiles = self.scenario.fault_profiles.len();
                match code {
                    KeyCode::Up => *selected_index = selected_index.saturating_sub(1),
                    KeyCode::Down if profiles > 0 && *selected_index + 1 < profiles => {
                        *selected_index += 1;
                    }
                    KeyCode::Enter => {
                        let index = *selected_index;
                        self.mode = AppMode::Normal;
                        self.trigger_fault_profile(index);
                    }
                    KeyCode::Esc => self.mode = AppMode::Normal,
                    _ => {}
                }
            }
        }
    }

    async fn handle_normal_key(&mut self, code: KeyCode) {
        // Tab-switching shortcuts 1..=4 are scoped if on MemoryMap so 1..=4 can select memory tables
        let is_mem_map = self.active_tab == ActiveTab::MemoryMap;
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.is_running = false,
            KeyCode::Char('?') => self.mode = AppMode::HelpOverlay,
            KeyCode::Tab => self.tab_next(),
            KeyCode::BackTab => self.tab_prev(),
            KeyCode::Char('1') if !is_mem_map => self.set_tab(ActiveTab::ProcessTags),
            KeyCode::Char('2') if !is_mem_map => self.set_tab(ActiveTab::Alarms),
            KeyCode::Char('3') if !is_mem_map => self.set_tab(ActiveTab::ProcessControl),
            KeyCode::Char('4') if !is_mem_map => self.set_tab(ActiveTab::NetworkSessions),
            KeyCode::Char('5') => self.set_tab(ActiveTab::MemoryMap),
            KeyCode::Char('6') => self.set_tab(ActiveTab::FaultStudio),
            KeyCode::Char('7') => self.set_tab(ActiveTab::EventLog),

            // Tab-specific keybindings
            _ => match self.active_tab {
                ActiveTab::ProcessTags => match code {
                    KeyCode::Up => self.select_up(),
                    KeyCode::Down => self.select_down(),
                    KeyCode::PageUp => self.select_page_up(),
                    KeyCode::PageDown => self.select_page_down(),
                    KeyCode::Enter | KeyCode::Char('f') | KeyCode::Char('F') => {
                        self.open_override_modal();
                    }
                    KeyCode::Char(' ') => self.toggle_discrete(),
                    KeyCode::Char('c') => self.clear_override_selected(),
                    KeyCode::Char('C') => self.clear_all_overrides(),
                    KeyCode::Char('m') | KeyCode::Char('M') => {
                        self.tag_area_filter = self.tag_area_filter.next();
                        self.selected_tag_index = 0;
                        self.tag_table_state.select(Some(0));
                    }
                    KeyCode::Char('/') => {
                        self.mode = AppMode::TagSearchModal {
                            query_buffer: self.tag_search_query.clone(),
                        };
                    }
                    KeyCode::Esc => {
                        if !self.tag_search_query.is_empty() {
                            self.tag_search_query.clear();
                            self.selected_tag_index = 0;
                            self.tag_table_state.select(Some(0));
                        } else {
                            self.is_running = false;
                        }
                    }
                    KeyCode::Char('z') | KeyCode::Char('Z') => self.freeze_selected_tag(),
                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        self.mode = AppMode::FaultProfileMenu { selected_index: 0 };
                    }
                    _ => {}
                },
                ActiveTab::Alarms => match code {
                    KeyCode::Up => self.alarm_select_up(),
                    KeyCode::Down => self.alarm_select_down(),
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        self.acknowledge_selected_alarm().await;
                    }
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        self.reset_selected_alarm().await;
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        self.open_shelve_modal();
                    }
                    KeyCode::Char('u') | KeyCode::Char('U') => {
                        self.unshelve_selected_alarm().await;
                    }
                    KeyCode::Esc => self.is_running = false,
                    _ => {}
                },
                ActiveTab::ProcessControl => match code {
                    KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Enter => {
                        // Open setpoint override modal for first PID or tag
                        let (_, _, pids) = self.get_process_states();
                        if let Some(pid) = pids.first() {
                            let sp_tag = pid.setpoint_tag.clone();
                            if let Some(idx) = self.scenario.tag_index(&sp_tag) {
                                self.selected_tag_index = idx;
                                self.tag_table_state.select(Some(idx));
                                self.open_override_modal();
                            }
                        }
                    }
                    KeyCode::Esc => self.is_running = false,
                    _ => {}
                },
                ActiveTab::NetworkSessions => {
                    if code == KeyCode::Esc {
                        self.is_running = false;
                    }
                }
                ActiveTab::MemoryMap => match code {
                    KeyCode::Char('1') => {
                        self.memory_table_selected = MemoryTableSelection::Coils;
                        self.memory_address_offset = 0;
                    }
                    KeyCode::Char('2') => {
                        self.memory_table_selected = MemoryTableSelection::DiscreteInputs;
                        self.memory_address_offset = 0;
                    }
                    KeyCode::Char('3') => {
                        self.memory_table_selected = MemoryTableSelection::InputRegisters;
                        self.memory_address_offset = 0;
                    }
                    KeyCode::Char('4') => {
                        self.memory_table_selected = MemoryTableSelection::HoldingRegisters;
                        self.memory_address_offset = 0;
                    }
                    KeyCode::Left | KeyCode::Char('h') => {
                        self.memory_table_selected = self.memory_table_selected.prev();
                        self.memory_address_offset = 0;
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        self.memory_table_selected = self.memory_table_selected.next();
                        self.memory_address_offset = 0;
                    }
                    KeyCode::Up => {
                        self.memory_address_offset = self.memory_address_offset.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        self.memory_address_offset = self.memory_address_offset.saturating_add(1);
                    }
                    KeyCode::PageUp => {
                        self.memory_address_offset = self.memory_address_offset.saturating_sub(16);
                    }
                    KeyCode::PageDown => {
                        self.memory_address_offset = self.memory_address_offset.saturating_add(16);
                    }
                    KeyCode::Esc => self.is_running = false,
                    _ => {}
                },
                ActiveTab::FaultStudio => match code {
                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        self.mode = AppMode::FaultProfileMenu { selected_index: 0 };
                    }
                    KeyCode::Char('c') => self.clear_override_selected(),
                    KeyCode::Char('C') => self.clear_all_overrides(),
                    KeyCode::Esc => self.is_running = false,
                    _ => {}
                },
                ActiveTab::EventLog => match code {
                    KeyCode::Char('f') | KeyCode::Char('F') => {
                        self.log_filter = self.log_filter.next();
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        self.log_auto_scroll = !self.log_auto_scroll;
                    }
                    KeyCode::Up => {
                        self.log_auto_scroll = false;
                        let cur = self.log_table_state.selected().unwrap_or(0);
                        self.log_table_state.select(Some(cur.saturating_sub(1)));
                    }
                    KeyCode::Down => {
                        let cur = self.log_table_state.selected().unwrap_or(0);
                        let total = self.log_entries.len();
                        if total > 0 && cur + 1 < total {
                            self.log_table_state.select(Some(cur + 1));
                        }
                    }
                    KeyCode::PageUp => {
                        self.log_auto_scroll = false;
                        let cur = self.log_table_state.selected().unwrap_or(0);
                        self.log_table_state.select(Some(cur.saturating_sub(10)));
                    }
                    KeyCode::PageDown => {
                        self.log_auto_scroll = false;
                        let cur = self.log_table_state.selected().unwrap_or(0);
                        let total = self.log_entries.len();
                        if total > 0 {
                            self.log_table_state.select(Some((cur + 10).min(total - 1)));
                        }
                    }
                    KeyCode::Home => {
                        self.log_auto_scroll = false;
                        self.log_table_state.select(Some(0));
                    }
                    KeyCode::End => {
                        self.log_auto_scroll = true;
                        if !self.log_entries.is_empty() {
                            self.log_table_state
                                .select(Some(self.log_entries.len() - 1));
                        }
                    }
                    KeyCode::Esc => self.is_running = false,
                    _ => {}
                },
            },
        }
    }
}

pub(crate) fn is_discrete_area(area: MemoryArea) -> bool {
    matches!(area, MemoryArea::Coil | MemoryArea::DiscreteInput)
}

pub(crate) fn format_value(current: Option<f64>, is_discrete: bool) -> String {
    match current {
        Some(value) if is_discrete => {
            if value >= BOOL_THRESHOLD {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        Some(value) => format!("{value:.2}"),
        None => String::new(),
    }
}

pub(crate) fn push_capped(buf: &mut VecDeque<f64>, value: f64) {
    if buf.len() >= SPARKLINE_CAPACITY {
        buf.pop_front();
    }
    buf.push_back(value);
}
