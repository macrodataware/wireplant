//! Foundation crate for WirePlant: the Process Image, the value codec, and the
//! shared enums every other crate names.

pub mod codec;
pub mod endian;
pub mod errors;
pub mod memory;
pub mod net;
pub mod overrides;
pub mod types;

pub use codec::TagAddress;
pub use errors::CoreError;
pub use memory::{PlantMemory, SessionCounter, SessionGuard, SharedProcessImage, WordSpan};
pub use net::{parse_host_port, AddrError};
pub use overrides::{create_shared_overrides, OverrideRegistry, SharedOverrides, TagOverride};
pub use types::{Endianness, MemoryArea, TagDataType};
