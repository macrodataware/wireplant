use rusqlite::Connection;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

pub(crate) struct ExportOptions {
    pub db_path: String,
    pub tag_id: Option<String>,
    pub format: String,
    pub output_file: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: Option<usize>,
}

/// Streams recorded telemetry out of the SQLite historian without booting the simulator.
pub(crate) fn run_export(
    opts: ExportOptions,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = Path::new(&opts.db_path);
    if !path.exists() {
        return Err(format!("Database file not found: '{}'", opts.db_path).into());
    }

    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    let query = "SELECT timestamp_ms, tag_id, value FROM telemetry \
                 WHERE (:tag IS NULL OR tag_id = :tag) \
                   AND (:from IS NULL OR timestamp_ms >= :from) \
                   AND (:to IS NULL OR timestamp_ms <= :to) \
                 ORDER BY timestamp_ms ASC \
                 LIMIT :limit";

    let mut stmt = conn.prepare(query)?;

    let mut writer: Box<dyn Write> = match &opts.output_file {
        Some(target) => Box::new(BufWriter::new(File::create(target)?)),
        None => Box::new(BufWriter::new(io::stdout())),
    };

    let is_json = opts.format.eq_ignore_ascii_case("json");
    if is_json {
        writeln!(writer, "[")?;
    } else {
        writeln!(writer, "timestamp_ms,tag_id,value")?;
    }

    let limit_val = opts.limit.unwrap_or(usize::MAX) as i64;
    let mut rows = stmt.query(rusqlite::named_params! {
        ":tag": opts.tag_id.as_deref(),
        ":from": opts.from_ms,
        ":to": opts.to_ms,
        ":limit": limit_val,
    })?;

    let mut count = 0usize;
    let mut first = true;

    while let Some(row) = rows.next()? {
        let ts: i64 = row.get(0)?;
        let tag: String = row.get(1)?;
        let val: f64 = row.get(2)?;

        if is_json {
            if !first {
                writeln!(writer, ",")?;
            }
            write!(
                writer,
                "  {{\"timestamp_ms\":{ts},\"tag_id\":\"{tag}\",\"value\":{val}}}"
            )?;
            first = false;
        } else {
            writeln!(writer, "{ts},{tag},{val}")?;
        }
        count += 1;
    }

    if is_json {
        writeln!(writer, "\n]")?;
    }

    writer.flush()?;
    eprintln!(
        "Exported {} records to {}",
        count,
        opts.output_file.as_deref().unwrap_or("stdout")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_missing_file_reports_clean_error() {
        let opts = ExportOptions {
            db_path: "/nonexistent/test_historian_db.db".to_string(),
            tag_id: None,
            format: "csv".to_string(),
            output_file: None,
            from_ms: None,
            to_ms: None,
            limit: None,
        };

        let res = run_export(opts);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("not found"));
    }
}
