//! The one error type the codec and the Process Image produce, and the rule for
//! what each layer does with one.
//!
//! # Where errors go
//!
//! - **Protocol edge** (`wireplant_modbus`): an out-of-range address or an
//!   unsupported function becomes a Modbus exception code on the wire. Nothing
//!   is logged per request; a client polling badly would flood the log.
//! - **REST** (`wireplant_api`): a tag the scenario does not declare is a 404
//!   and a tag the Process Image refuses is a 500, both with a JSON body naming
//!   the tag. No arm substitutes a default for a failure.
//! - **Engine** (`wireplant_engine`): a tick that cannot read or write one tag
//!   logs that tag and keeps simulating the rest. A failed write is never
//!   reported as a success.
//! - **TUI** (`wireplant_tui`): the operator sees the failure in the event log
//!   and the value does not change, because a write that did not happen must not
//!   look like one that did.
//!
//! The rule behind all four: an error is either turned into something the caller
//! can act on, or it is surfaced. It is never swallowed into a plausible value.

use crate::types::MemoryArea;
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum CoreError {
    #[error("Memory area {0:?} is read-only for external write operations")]
    ReadOnlyArea(MemoryArea),
    #[error("Address {address} out of bounds for area {area:?} (capacity {capacity})")]
    OutOfBounds {
        area: MemoryArea,
        address: usize,
        capacity: usize,
    },
    #[error("Area {0:?} does not support bit operations")]
    InvalidAreaForBit(MemoryArea),
    #[error("Area {0:?} does not support register operations")]
    InvalidAreaForRegister(MemoryArea),
    #[error("{data_type:?} cannot live in area {area:?}")]
    DataTypeNotInArea {
        area: MemoryArea,
        data_type: crate::types::TagDataType,
    },
}
