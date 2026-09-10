use clap::{Parser, Subcommand};

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliCommand {
    /// Validates a scenario configuration without starting any listeners or simulation loops.
    Validate {
        /// Path to scenario TOML file (positional). If omitted, validates the embedded default.
        #[arg(value_name = "FILE")]
        path: Option<String>,

        /// Path to scenario TOML file (via flag).
        #[arg(short, long, value_name = "FILE", conflicts_with = "path")]
        scenario: Option<String>,
    },

    /// Exports recorded telemetry from the SQLite historian to CSV or JSON.
    ExportHistory {
        /// SQLite database file path.
        #[arg(short, long, default_value = "wireplant_history.db")]
        db: String,

        /// Tag ID filter. If omitted, all tags are exported.
        #[arg(short, long)]
        tag: Option<String>,

        /// Export format: "csv" or "json".
        #[arg(short, long, default_value = "csv")]
        format: String,

        /// Output file path. If omitted, streams to stdout.
        #[arg(short, long)]
        output: Option<String>,

        /// Start timestamp in milliseconds.
        #[arg(long)]
        from: Option<i64>,

        /// End timestamp in milliseconds.
        #[arg(long)]
        to: Option<i64>,

        /// Maximum records to output.
        #[arg(long)]
        limit: Option<usize>,
    },
}

/// WirePlant: Industrial OT Protocol & Telemetry Mock CLI
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub(crate) struct CliArgs {
    #[command(subcommand)]
    pub command: Option<CliCommand>,

    /// Path to a declarative scenario TOML file. If omitted, uses embedded default.
    #[arg(short, long)]
    pub scenario: Option<String>,

    /// Validate scenario configuration and exit immediately without starting runtime servers.
    #[arg(long)]
    pub validate: bool,

    /// Override HTTP API authorization token / key.
    #[arg(long, env = "WIREPLANT_API_KEY")]
    pub api_key: Option<String>,

    /// Override Modbus TCP bind address (e.g. 0.0.0.0:5020).
    #[arg(short = 'm', long)]
    pub modbus_bind: Option<String>,

    /// Override Modbus RTU serial port device path (e.g. /dev/ttyUSB0).
    #[arg(long)]
    pub modbus_serial: Option<String>,

    /// Override Modbus RTU serial baud rate (e.g. 9600, 19200, 115200).
    #[arg(long)]
    pub modbus_baud: Option<u32>,

    /// Override OPC-UA server bind address (e.g. 0.0.0.0:4840).
    #[arg(long)]
    pub opcua_bind: Option<String>,

    /// Override Inspection HTTP API bind address (e.g. 0.0.0.0:8080).
    #[arg(long)]
    pub http_bind: Option<String>,

    /// Override MQTT broker address (e.g. 127.0.0.1:1883).
    #[arg(long)]
    pub mqtt_broker: Option<String>,

    /// Enable or disable MQTT telemetry publishing.
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub mqtt_enabled: Option<bool>,

    /// Enable or disable SQLite embedded historian.
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub historian_enabled: Option<bool>,

    /// Override SQLite historian database file path.
    #[arg(long)]
    pub historian_db: Option<String>,

    /// Launch interactive Ratatui terminal dashboard.
    ///
    /// Mutually exclusive with `--headless`: a terminal dashboard and the
    /// structured-log path are two different programs to run.
    #[arg(long, conflicts_with = "headless")]
    pub tui: bool,

    /// Force headless operation without TUI, emitting structured logs.
    #[arg(long)]
    pub headless: bool,
}

impl CliArgs {
    /// Whether this invocation should open the dashboard.
    ///
    /// One predicate, because the log-subscriber setup in `main` and the spawn
    /// logic in the orchestrator both have to agree about what "TUI mode" means.
    pub(crate) fn wants_tui(&self) -> bool {
        self.tui && !self.headless
    }

    /// Returns the scenario target to validate if validation mode was requested via
    /// subcommand (`wireplant validate [FILE]` or `wireplant validate -s/--scenario FILE`)
    /// or flag (`wireplant --validate [-s FILE]`).
    pub(crate) fn validation_target(&self) -> Option<Option<&str>> {
        if let Some(CliCommand::Validate { path, scenario }) = &self.command {
            Some(scenario.as_deref().or(path.as_deref()))
        } else if self.validate {
            Some(self.scenario.as_deref())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_optional_flag_defaults_to_absent() {
        let args = CliArgs::parse_from(["wireplant"]);

        assert_eq!(args.scenario, None);
        assert_eq!(args.modbus_bind, None);
        assert_eq!(args.opcua_bind, None);
        assert_eq!(args.http_bind, None);
        assert_eq!(args.mqtt_broker, None);
        assert_eq!(args.mqtt_enabled, None);
        assert!(!args.tui);
        assert!(!args.headless);
        assert!(!args.wants_tui());
    }

    #[test]
    fn long_and_short_flags_land_in_the_same_fields() {
        let args = CliArgs::parse_from([
            "wireplant",
            "--scenario",
            "test_scenario.toml",
            "-m",
            "127.0.0.1:5021",
            "--opcua-bind",
            "127.0.0.1:4841",
            "--http-bind",
            "127.0.0.1:8081",
            "--mqtt-broker",
            "127.0.0.1:1884",
            "--mqtt-enabled",
            "--tui",
        ]);

        assert_eq!(args.scenario.as_deref(), Some("test_scenario.toml"));
        assert_eq!(args.modbus_bind.as_deref(), Some("127.0.0.1:5021"));
        assert_eq!(args.opcua_bind.as_deref(), Some("127.0.0.1:4841"));
        assert_eq!(args.http_bind.as_deref(), Some("127.0.0.1:8081"));
        assert_eq!(args.mqtt_broker.as_deref(), Some("127.0.0.1:1884"));
        assert_eq!(args.mqtt_enabled, Some(true));
        assert!(args.wants_tui());

        let short = CliArgs::parse_from(["wireplant", "-s", "scenario.toml", "-m", "0.0.0.0:1502"]);
        assert_eq!(short.scenario.as_deref(), Some("scenario.toml"));
        assert_eq!(short.modbus_bind.as_deref(), Some("0.0.0.0:1502"));
    }

    /// The bare `--mqtt-enabled` means true and an explicit `false` means false;
    /// that asymmetry is the reason the flag is an `Option<bool>` at all.
    #[test]
    fn mqtt_enabled_reads_a_bare_flag_and_an_explicit_value() {
        let bare = CliArgs::parse_from(["wireplant", "--mqtt-enabled"]);
        assert_eq!(bare.mqtt_enabled, Some(true));

        let explicit = CliArgs::parse_from(["wireplant", "--mqtt-enabled", "false"]);
        assert_eq!(explicit.mqtt_enabled, Some(false));
    }

    /// Asking for the dashboard and for structured logs at the same time has no
    /// sensible meaning, so clap refuses it instead of silently picking one.
    #[test]
    fn tui_and_headless_together_are_rejected() {
        let result = CliArgs::try_parse_from(["wireplant", "--tui", "--headless"]);

        assert!(result.is_err(), "--tui --headless should not parse");
    }

    #[test]
    fn headless_alone_suppresses_the_dashboard() {
        let args = CliArgs::parse_from(["wireplant", "--headless"]);
        assert!(!args.wants_tui());
    }

    #[test]
    fn validate_subcommand_captures_target_scenario() {
        let with_path =
            CliArgs::parse_from(["wireplant", "validate", "scenarios/petrochemical.toml"]);
        assert_eq!(
            with_path.validation_target(),
            Some(Some("scenarios/petrochemical.toml"))
        );

        let without_path = CliArgs::parse_from(["wireplant", "validate"]);
        assert_eq!(without_path.validation_target(), Some(None));

        let with_short_flag = CliArgs::parse_from([
            "wireplant",
            "validate",
            "-s",
            "scenarios/petrochemical.toml",
        ]);
        assert_eq!(
            with_short_flag.validation_target(),
            Some(Some("scenarios/petrochemical.toml"))
        );

        let with_long_flag = CliArgs::parse_from([
            "wireplant",
            "validate",
            "--scenario",
            "scenarios/petrochemical.toml",
        ]);
        assert_eq!(
            with_long_flag.validation_target(),
            Some(Some("scenarios/petrochemical.toml"))
        );
    }

    #[test]
    fn validate_subcommand_rejects_conflicting_positional_and_flag() {
        let result = CliArgs::try_parse_from([
            "wireplant",
            "validate",
            "scenarios/petrochemical.toml",
            "-s",
            "scenarios/other.toml",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn validate_flag_captures_target_scenario() {
        let with_flag =
            CliArgs::parse_from(["wireplant", "--validate", "-s", "scenarios/custom.toml"]);
        assert_eq!(
            with_flag.validation_target(),
            Some(Some("scenarios/custom.toml"))
        );

        let flag_default = CliArgs::parse_from(["wireplant", "--validate"]);
        assert_eq!(flag_default.validation_target(), Some(None));
    }

    #[test]
    fn api_key_flag_is_parsed() {
        let args = CliArgs::parse_from(["wireplant", "--api-key", "token123"]);
        assert_eq!(args.api_key.as_deref(), Some("token123"));
    }
}
