//! # WirePlant Core (`wireplant_core`)
//!
//! Foundational data structures, memory layout, codecs, and endianness conversions
//! for the WirePlant industrial process simulation system.
//!
//! ## Architecture Overview
//!
//! In industrial automation, a Programmable Logic Controller (PLC) exposes its memory
//! through four standardized tables (the **Process Image**):
//!
//! | Table | Address Prefix | Data Type | Access |
//! | :--- | :--- | :--- | :--- |
//! | **Coils** | `0x` | Discrete Bit (1-bit) | Read/Write |
//! | **Discrete Inputs** | `1x` | Discrete Bit (1-bit) | Read-Only |
//! | **Input Registers** | `3x` | 16-bit Word | Read-Only |
//! | **Holding Registers** | `4x` | 16-bit Word | Read/Write |
//!
//! [`PlantMemory`] maintains these four tables as flat, indexed vectors.
//!
//! ## Example: Reading and Writing Registers
//!
//! ```rust
//! use wireplant_core::memory::PlantMemory;
//! use wireplant_core::types::MemoryArea;
//!
//! let mut memory = PlantMemory::new(10, 10, 10, 10);
//! memory.write_registers_slice(MemoryArea::HoldingRegister, 0, &[42, 100]).unwrap();
//!
//! let slice = memory.read_registers_slice(MemoryArea::HoldingRegister, 0, 2).unwrap();
//! assert_eq!(slice, &[42, 100]);
//! ```

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
