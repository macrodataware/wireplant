use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use std::fmt::Write;
use std::sync::Arc;
use wireplant_core::codec::decode_tag;
use wireplant_engine::alarm::IsaAlarmState;

use crate::routes::AppState;

/// Renders Prometheus / OpenMetrics text exposition format.
///
/// Snapshots memory values quickly under the read lock to prevent holding up
/// the simulation ticker or external protocol clients.
pub async fn prometheus_metrics_handler(State(state): State<Arc<AppState>>) -> Response {
    let mut out = String::with_capacity(4096);

    let uptime = state.start_time.elapsed().as_secs_f64();
    writeln!(
        out,
        "# HELP wireplant_uptime_seconds Total runtime in seconds"
    )
    .unwrap();
    writeln!(out, "# TYPE wireplant_uptime_seconds gauge").unwrap();
    writeln!(out, "wireplant_uptime_seconds {:.3}", uptime).unwrap();

    let sessions = state.sessions.active();
    writeln!(
        out,
        "\n# HELP wireplant_modbus_sessions Active Modbus client sessions"
    )
    .unwrap();
    writeln!(out, "# TYPE wireplant_modbus_sessions gauge").unwrap();
    writeln!(out, "wireplant_modbus_sessions {}", sessions).unwrap();

    let snapshot = state.alarms.get_snapshot().await;
    writeln!(
        out,
        "\n# HELP wireplant_alarms Configured alarms grouped by state and severity"
    )
    .unwrap();
    writeln!(out, "# TYPE wireplant_alarms gauge").unwrap();

    let mut state_counts = std::collections::BTreeMap::new();
    for alarm in &snapshot {
        let state_name = match alarm.state {
            IsaAlarmState::Normal => "normal",
            IsaAlarmState::Unacknowledged => "unacknowledged",
            IsaAlarmState::Acknowledged => "acknowledged",
            IsaAlarmState::ReturnToNormalUnack => "rtn_unack",
            IsaAlarmState::Latched => "latched",
            IsaAlarmState::Shelved => "shelved",
        };
        let key = (alarm.config.severity.label().to_lowercase(), state_name);
        *state_counts.entry(key).or_insert(0) += 1;
    }

    for ((severity, state_str), count) in state_counts {
        writeln!(
            out,
            "wireplant_alarms{{severity=\"{severity}\",state=\"{state_str}\"}} {count}"
        )
        .unwrap();
    }

    // Read tags in one pass so the read lock is held only for copying values.
    let mut tag_snapshots: Vec<(String, &'static str, String, f64)> = {
        let memory = state.shared_memory.read().await;
        state
            .scenario
            .tags
            .iter()
            .filter_map(|tag| {
                decode_tag(&memory, tag)
                    .ok()
                    .map(|val| (tag.id.clone(), tag.area.as_str(), tag.units.clone(), val))
            })
            .collect()
    };
    tag_snapshots.sort_by(|a, b| a.0.cmp(&b.0));

    writeln!(
        out,
        "\n# HELP wireplant_process_tag Current Process Image tag values"
    )
    .unwrap();
    writeln!(out, "# TYPE wireplant_process_tag gauge").unwrap();
    for (id, area, units, val) in tag_snapshots {
        let escaped_id = escape_prometheus_label(&id);
        let escaped_units = escape_prometheus_label(&units);
        let formatted_val = format_prometheus_float(val);
        writeln!(
            out,
            "wireplant_process_tag{{tag_id=\"{escaped_id}\",area=\"{area}\",units=\"{escaped_units}\"}} {formatted_val}"
        )
        .unwrap();
    }

    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        out,
    )
        .into_response()
}

fn format_prometheus_float(val: f64) -> String {
    if val.is_infinite() {
        if val.is_sign_positive() {
            "+Inf".to_string()
        } else {
            "-Inf".to_string()
        }
    } else if val.is_nan() {
        "NaN".to_string()
    } else {
        format!("{:.6}", val)
    }
}

fn escape_prometheus_label(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
