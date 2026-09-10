//! Embedded time-series historian leveraging SQLite in Write-Ahead Logging (WAL) mode.
//!
//! Provides batch-buffered ingestion from `EngineTickEvent`, non-blocking concurrent queries
//! for SCADA/REST clients, and automated retention pruning.

use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{info, warn};

use crate::ticker::EngineTickEvent;

/// Error types produced by the embedded SQLite historian actor.
#[derive(Debug, thiserror::Error)]
pub enum HistorianError {
    #[error("Historian background worker channel disconnected")]
    ChannelClosed,
    #[error("Historian database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Configuration for the embedded SQLite historian.
#[derive(Debug, Clone)]
pub struct HistorianConfig {
    pub db_path: PathBuf,
    pub flush_interval: Duration,
    pub batch_size: usize,
    pub retention_hours: u32,
}

impl Default for HistorianConfig {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from("wireplant_history.db"),
            flush_interval: Duration::from_millis(1000),
            batch_size: 500,
            retention_hours: 48,
        }
    }
}

/// Tag definition for metadata cache.
#[derive(Debug, Clone)]
pub struct HistorianTagMeta {
    pub id: String,
    pub description: String,
    pub units: String,
    pub data_type: String,
}

/// Historical alarm record returned by database queries:
/// `(timestamp_ms, alarm_id, state_raw, process_value, message)`.
pub type AlarmLogRecord = (i64, String, u16, Option<f64>, Option<String>);

/// Commands dispatched to the background SQLite worker thread.
enum HistorianCommand {
    BatchInsert {
        samples: Vec<(i64, String, f64)>,
        ack: Option<oneshot::Sender<Result<(), rusqlite::Error>>>,
    },
    QueryTag {
        tag_id: String,
        start_ms: i64,
        end_ms: i64,
        limit: usize,
        reply: oneshot::Sender<Result<Vec<(i64, f64)>, rusqlite::Error>>,
    },
    LogAlarm {
        alarm_id: String,
        state: u16,
        value: Option<f64>,
        message: Option<String>,
        ack: Option<oneshot::Sender<Result<(), rusqlite::Error>>>,
    },
    PruneBefore {
        cutoff_ms: i64,
        reply: oneshot::Sender<Result<usize, rusqlite::Error>>,
    },
    QueryAlarms {
        alarm_id: Option<String>,
        start_ms: i64,
        end_ms: i64,
        limit: usize,
        reply: oneshot::Sender<Result<Vec<AlarmLogRecord>, rusqlite::Error>>,
    },
}

/// Embedded industrial historian using SQLite with the Actor Pattern.
///
/// Database file transactions are isolated on a dedicated background worker thread,
/// communicating with Tokio async tasks through message passing.
#[derive(Clone, Debug)]
pub struct Historian {
    pub config: HistorianConfig,
    cmd_tx: mpsc::Sender<HistorianCommand>,
}

impl Historian {
    /// Opens or creates the SQLite database, establishes performance pragmas, and launches the worker thread.
    pub fn new(
        config: HistorianConfig,
        tags: &[HistorianTagMeta],
    ) -> Result<Self, rusqlite::Error> {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<HistorianCommand>(2048);
        let tags_owned = tags.to_vec();
        let db_path = config.db_path.clone();

        let mut conn = if db_path.to_string_lossy() == ":memory:" {
            Connection::open_in_memory()?
        } else {
            if let Some(parent) = db_path.parent() {
                if !parent.as_os_str().is_empty() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }
            Connection::open(&db_path)?
        };

        // Configure performance pragmas for high ingestion and concurrent readers
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;
             PRAGMA cache_size = -16000;",
        )?;

        // Initialize schema
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tags (
                tag_id TEXT PRIMARY KEY,
                description TEXT,
                units TEXT,
                data_type TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS telemetry (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp_ms INTEGER NOT NULL,
                tag_id TEXT NOT NULL,
                value REAL NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_telemetry_tag_time 
            ON telemetry(tag_id, timestamp_ms, id);

            CREATE TABLE IF NOT EXISTS alarm_log (
                timestamp_ms INTEGER NOT NULL,
                alarm_id TEXT NOT NULL,
                state INTEGER NOT NULL,
                value REAL,
                message TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_alarm_log_time 
            ON alarm_log(timestamp_ms);",
        )?;

        // Populate or refresh tag metadata
        {
            let tx = conn.transaction()?;
            {
                let mut stmt = tx.prepare(
                    "INSERT OR REPLACE INTO tags (tag_id, description, units, data_type) 
                     VALUES (?1, ?2, ?3, ?4)",
                )?;
                for tag in &tags_owned {
                    stmt.execute(params![tag.id, tag.description, tag.units, tag.data_type])?;
                }
            }
            tx.commit()?;
        }

        // Spawn dedicated background OS thread
        std::thread::Builder::new()
            .name("wireplant-historian-worker".to_string())
            .spawn(move || {
                while let Some(cmd) = cmd_rx.blocking_recv() {
                    match cmd {
                        HistorianCommand::BatchInsert { samples, ack } => {
                            let res = Self::handle_batch_insert(&mut conn, &samples);
                            if let Some(reply) = ack {
                                let _ = reply.send(res);
                            }
                        }
                        HistorianCommand::QueryTag {
                            tag_id,
                            start_ms,
                            end_ms,
                            limit,
                            reply,
                        } => {
                            let res =
                                Self::handle_query_tag(&conn, &tag_id, start_ms, end_ms, limit);
                            let _ = reply.send(res);
                        }
                        HistorianCommand::LogAlarm {
                            alarm_id,
                            state,
                            value,
                            message,
                            ack,
                        } => {
                            let res = Self::handle_log_alarm(
                                &conn,
                                &alarm_id,
                                state,
                                value,
                                message.as_deref(),
                            );
                            if let Some(reply) = ack {
                                let _ = reply.send(res);
                            }
                        }
                        HistorianCommand::PruneBefore { cutoff_ms, reply } => {
                            let res = (|| -> Result<usize, rusqlite::Error> {
                                let deleted_telemetry = conn.execute(
                                    "DELETE FROM telemetry WHERE timestamp_ms < ?1",
                                    params![cutoff_ms],
                                )?;
                                let deleted_alarms = conn.execute(
                                    "DELETE FROM alarm_log WHERE timestamp_ms < ?1",
                                    params![cutoff_ms],
                                )?;
                                Ok(deleted_telemetry + deleted_alarms)
                            })();
                            let _ = reply.send(res);
                        }
                        HistorianCommand::QueryAlarms {
                            alarm_id,
                            start_ms,
                            end_ms,
                            limit,
                            reply,
                        } => {
                            let res = Self::handle_query_alarms(
                                &conn,
                                alarm_id.as_deref(),
                                start_ms,
                                end_ms,
                                limit,
                            );
                            let _ = reply.send(res);
                        }
                    }
                }
            })
            .expect("Failed to spawn historian worker thread");

        Ok(Self { config, cmd_tx })
    }

    fn handle_batch_insert(
        conn: &mut Connection,
        samples: &[(i64, String, f64)],
    ) -> Result<(), rusqlite::Error> {
        if samples.is_empty() {
            return Ok(());
        }
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO telemetry (timestamp_ms, tag_id, value) VALUES (?1, ?2, ?3)",
            )?;
            for (ts, tag, val) in samples {
                stmt.execute(params![ts, tag, val])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn handle_query_tag(
        conn: &Connection,
        tag_id: &str,
        start_ms: i64,
        end_ms: i64,
        limit: usize,
    ) -> Result<Vec<(i64, f64)>, rusqlite::Error> {
        let mut stmt = conn.prepare(
            "SELECT timestamp_ms, value 
             FROM telemetry 
             WHERE tag_id = ?1 AND timestamp_ms BETWEEN ?2 AND ?3 
             ORDER BY timestamp_ms ASC, id ASC 
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![tag_id, start_ms, end_ms, limit as i64], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        let mut results = Vec::new();
        for r in rows {
            results.push(r?);
        }
        Ok(results)
    }

    fn handle_log_alarm(
        conn: &Connection,
        alarm_id: &str,
        state: u16,
        value: Option<f64>,
        message: Option<&str>,
    ) -> Result<(), rusqlite::Error> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis() as i64;
        conn.execute(
            "INSERT INTO alarm_log (timestamp_ms, alarm_id, state, value, message) 
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![now_ms, alarm_id, state, value, message],
        )?;
        Ok(())
    }

    fn handle_query_alarms(
        conn: &Connection,
        alarm_id: Option<&str>,
        start_ms: i64,
        end_ms: i64,
        limit: usize,
    ) -> Result<Vec<AlarmLogRecord>, rusqlite::Error> {
        let mut results = Vec::new();
        if let Some(id) = alarm_id {
            let mut stmt = conn.prepare(
                "SELECT timestamp_ms, alarm_id, state, value, message
                 FROM alarm_log
                 WHERE alarm_id = ?1 AND timestamp_ms BETWEEN ?2 AND ?3
                 ORDER BY timestamp_ms ASC
                 LIMIT ?4",
            )?;
            let rows = stmt.query_map(params![id, start_ms, end_ms, limit as i64], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?;
            for r in rows {
                results.push(r?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT timestamp_ms, alarm_id, state, value, message
                 FROM alarm_log
                 WHERE timestamp_ms BETWEEN ?1 AND ?2
                 ORDER BY timestamp_ms ASC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![start_ms, end_ms, limit as i64], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?;
            for r in rows {
                results.push(r?);
            }
        }
        Ok(results)
    }

    /// Background worker consuming simulation ticks and batch-inserting telemetry into SQLite.
    pub async fn run_ingestion_loop(
        self: Arc<Self>,
        mut tick_rx: watch::Receiver<EngineTickEvent>,
        mut cancel_rx: tokio::sync::broadcast::Receiver<()>,
    ) {
        let mut buffer: Vec<(i64, String, f64)> = Vec::with_capacity(self.config.batch_size);
        let mut interval = tokio::time::interval(self.config.flush_interval);

        loop {
            tokio::select! {
                _ = cancel_rx.recv() => {
                    info!("Historian received shutdown signal, flushing buffer");
                    let _ = self.flush_buffer(&mut buffer).await;
                    break;
                }
                changed = tick_rx.changed() => {
                    if changed.is_err() {
                        let _ = self.flush_buffer(&mut buffer).await;
                        break;
                    }
                    let event = tick_rx.borrow().clone();
                    let now_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or(Duration::ZERO)
                        .as_millis() as i64;

                    let mut tag_entries: Vec<(String, f64)> = event.tag_values.into_iter().collect();
                    tag_entries.sort_by(|a, b| a.0.cmp(&b.0));

                    for (tag_id, val) in tag_entries {
                        buffer.push((now_ms, tag_id, val));
                    }

                    if buffer.len() >= self.config.batch_size {
                        let _ = self.flush_buffer(&mut buffer).await;
                    }
                }
                _ = interval.tick() => {
                    if !buffer.is_empty() {
                        let _ = self.flush_buffer(&mut buffer).await;
                    }
                }
            }
        }
    }

    /// Flushes buffered telemetry samples to the background worker.
    pub async fn flush_buffer(
        &self,
        buffer: &mut Vec<(i64, String, f64)>,
    ) -> Result<(), HistorianError> {
        if buffer.is_empty() {
            return Ok(());
        }
        let samples = std::mem::take(buffer);
        let (ack_tx, ack_rx) = oneshot::channel();
        self.cmd_tx
            .send(HistorianCommand::BatchInsert {
                samples,
                ack: Some(ack_tx),
            })
            .await
            .map_err(|_| HistorianError::ChannelClosed)?;

        ack_rx
            .await
            .map_err(|_| HistorianError::ChannelClosed)?
            .map_err(HistorianError::Sqlite)
    }

    /// Background task for pruning data older than the retention ceiling.
    pub async fn run_retention_pruner(
        self: Arc<Self>,
        mut cancel_rx: tokio::sync::broadcast::Receiver<()>,
    ) {
        let mut interval = tokio::time::interval(Duration::from_secs(600));

        loop {
            tokio::select! {
                _ = cancel_rx.recv() => break,
                _ = interval.tick() => {
                    let cutoff_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or(Duration::ZERO)
                        .as_millis() as i64
                        - (self.config.retention_hours as i64 * 3600 * 1000);

                    if let Err(e) = self.prune_before(cutoff_ms).await {
                        warn!("Historian retention pruner error: {e}");
                    }
                }
            }
        }
    }

    /// Log an alarm lifecycle event to SQLite.
    pub async fn log_alarm_event(
        &self,
        alarm_id: &str,
        state: u16,
        value: Option<f64>,
        message: Option<&str>,
    ) -> Result<(), HistorianError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.cmd_tx
            .send(HistorianCommand::LogAlarm {
                alarm_id: alarm_id.to_string(),
                state,
                value,
                message: message.map(|s| s.to_string()),
                ack: Some(ack_tx),
            })
            .await
            .map_err(|_| HistorianError::ChannelClosed)?;

        ack_rx
            .await
            .map_err(|_| HistorianError::ChannelClosed)?
            .map_err(HistorianError::Sqlite)
    }

    /// Query historical time series for a tag within a millisecond timestamp window.
    pub async fn query_tag(
        &self,
        tag_id: &str,
        start_ms: i64,
        end_ms: i64,
        limit: usize,
    ) -> Result<Vec<(i64, f64)>, HistorianError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(HistorianCommand::QueryTag {
                tag_id: tag_id.to_string(),
                start_ms,
                end_ms,
                limit,
                reply: reply_tx,
            })
            .await
            .map_err(|_| HistorianError::ChannelClosed)?;

        reply_rx
            .await
            .map_err(|_| HistorianError::ChannelClosed)?
            .map_err(HistorianError::Sqlite)
    }

    /// Direct batch insert helper.
    pub async fn insert_samples(&self, samples: &[(i64, &str, f64)]) -> Result<(), HistorianError> {
        let owned = samples
            .iter()
            .map(|(ts, tag, val)| (*ts, tag.to_string(), *val))
            .collect();
        let (ack_tx, ack_rx) = oneshot::channel();
        self.cmd_tx
            .send(HistorianCommand::BatchInsert {
                samples: owned,
                ack: Some(ack_tx),
            })
            .await
            .map_err(|_| HistorianError::ChannelClosed)?;

        ack_rx
            .await
            .map_err(|_| HistorianError::ChannelClosed)?
            .map_err(HistorianError::Sqlite)
    }

    /// Deletes records older than `cutoff_ms` directly (used for testing and maintenance).
    pub async fn prune_before(&self, cutoff_ms: i64) -> Result<usize, HistorianError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(HistorianCommand::PruneBefore {
                cutoff_ms,
                reply: reply_tx,
            })
            .await
            .map_err(|_| HistorianError::ChannelClosed)?;

        reply_rx
            .await
            .map_err(|_| HistorianError::ChannelClosed)?
            .map_err(HistorianError::Sqlite)
    }

    /// Query alarm logs within a millisecond timestamp window, optionally filtered by alarm ID.
    pub async fn query_alarms(
        &self,
        alarm_id: Option<&str>,
        start_ms: i64,
        end_ms: i64,
        limit: usize,
    ) -> Result<Vec<(i64, String, u16, Option<f64>, Option<String>)>, HistorianError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(HistorianCommand::QueryAlarms {
                alarm_id: alarm_id.map(str::to_string),
                start_ms,
                end_ms,
                limit,
                reply: reply_tx,
            })
            .await
            .map_err(|_| HistorianError::ChannelClosed)?;

        reply_rx
            .await
            .map_err(|_| HistorianError::ChannelClosed)?
            .map_err(HistorianError::Sqlite)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn historian_in_memory_crud_and_query() {
        let config = HistorianConfig {
            db_path: PathBuf::from(":memory:"),
            flush_interval: Duration::from_millis(50),
            batch_size: 10,
            retention_hours: 24,
        };

        let tags = vec![HistorianTagMeta {
            id: "tank_level".into(),
            description: "Tank Water Level".into(),
            units: "meters".into(),
            data_type: "Float32".into(),
        }];

        let historian = Arc::new(Historian::new(config, &tags).unwrap());

        // Insert historical data
        let samples = vec![
            (1000, "tank_level", 4.5),
            (2000, "tank_level", 4.8),
            (3000, "tank_level", 5.1),
            (4000, "tank_level", 5.4),
        ];
        historian.insert_samples(&samples).await.unwrap();

        // Query range 1500..3500
        let points = historian
            .query_tag("tank_level", 1500, 3500, 10)
            .await
            .unwrap();
        assert_eq!(points.len(), 2);
        assert_eq!(points[0], (2000, 4.8));
        assert_eq!(points[1], (3000, 5.1));

        // Test pruning
        let deleted = historian.prune_before(2500).await.unwrap();
        assert_eq!(deleted, 2);

        // Remaining points
        let remaining = historian
            .query_tag("tank_level", 0, 5000, 10)
            .await
            .unwrap();
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0], (3000, 5.1));
    }
}
