use thiserror::Error;
use wireplant_core::errors::CoreError;

/// Error types produced by interactive TUI operator overrides and memory writes.
#[derive(Debug, Error)]
pub enum TuiMemoryError {
    #[error("Tag '{0}' is not defined in this scenario")]
    TagNotFound(String),

    #[error("Tag '{tag_id}' is in read-only area '{area}'")]
    ReadOnlyArea { tag_id: String, area: &'static str },

    #[error("Process Image busy with a simulation tick")]
    ProcessImageBusy,

    #[error("Failed to encode tag '{tag_id}': {source}")]
    Encoding {
        tag_id: String,
        #[source]
        source: CoreError,
    },
}
