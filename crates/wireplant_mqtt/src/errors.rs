use thiserror::Error;
use wireplant_core::errors::CoreError;

/// Error types produced by the MQTT telemetry publisher and command subscriber.
#[derive(Debug, Error)]
pub enum MqttError {
    #[error("Malformed JSON command payload: {0}")]
    MalformedPayload(#[from] serde_json::Error),

    #[error("Target tag '{0}' not found in scenario configuration")]
    TagNotFound(String),

    #[error("Cannot write to tag '{tag_id}': memory area '{area}' is a read-only hardware input")]
    ReadOnlyArea { tag_id: String, area: &'static str },

    #[error("Process image encoding failure for tag '{tag_id}': {source}")]
    EncodingError {
        tag_id: String,
        #[source]
        source: CoreError,
    },
}
