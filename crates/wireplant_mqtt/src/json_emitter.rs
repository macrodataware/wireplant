use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Standard plain JSON telemetry payload structure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonTelemetryPayload {
    pub timestamp_ms: u128,
    pub facility: String,
    pub tick_number: u64,
    pub metrics: HashMap<String, f64>,
}

impl JsonTelemetryPayload {
    pub fn new(facility: String, tick_number: u64, metrics: HashMap<String, f64>) -> Self {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        Self {
            timestamp_ms,
            facility,
            tick_number,
            metrics,
        }
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_json_payload_survives_a_round_trip_through_bytes() {
        let mut metrics = HashMap::new();
        metrics.insert("sensor_temp".to_string(), 42.5);
        metrics.insert("sensor_pressure".to_string(), 101.3);

        let payload = JsonTelemetryPayload::new("Facility_North".to_string(), 10, metrics.clone());

        assert_eq!(payload.facility, "Facility_North");
        assert_eq!(payload.tick_number, 10);
        assert_eq!(payload.metrics, metrics);
        assert!(payload.timestamp_ms > 0);

        let json_bytes = payload
            .to_json_bytes()
            .expect("Failed to serialize to JSON");
        let deserialized: JsonTelemetryPayload =
            serde_json::from_slice(&json_bytes).expect("Failed to deserialize JSON");

        assert_eq!(deserialized.facility, payload.facility);
        assert_eq!(deserialized.tick_number, payload.tick_number);
        assert_eq!(deserialized.timestamp_ms, payload.timestamp_ms);
        assert_eq!(deserialized.metrics.get("sensor_temp"), Some(&42.5));
        assert_eq!(deserialized.metrics.get("sensor_pressure"), Some(&101.3));
    }
}
