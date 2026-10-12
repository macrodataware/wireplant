use serde::{Deserialize, Serialize};

/// Standard industrial PLC memory address partitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryArea {
    /// 0xxxx: 1-bit discrete read/write (e.g. motor relays, pump switches).
    Coil,
    /// 1xxxx: 1-bit discrete read-only (e.g. physical limit switches, sensor trips).
    DiscreteInput,
    /// 3xxxx: 16-bit analog read-only (e.g. pressure transducers, thermocouples).
    InputRegister,
    /// 4xxxx: 16-bit analog read/write (e.g. setpoint targets, speed limits).
    HoldingRegister,
}

impl MemoryArea {
    /// Whether external protocol clients (Modbus, OPC-UA, MQTT, HTTP) and operators
    /// are permitted to write to this memory area.
    ///
    /// Following standard industrial automation conventions:
    /// - Coils (0x) and Holding Registers (4x) represent controllable actuators and setpoints (read/write).
    /// - Discrete Inputs (1x) and Input Registers (3x) represent physical sensor readings (read-only).
    #[inline]
    pub const fn is_writable(self) -> bool {
        matches!(self, MemoryArea::Coil | MemoryArea::HoldingRegister)
    }

    /// Whether this memory area represents physical hardware sensor inputs.
    #[inline]
    pub const fn is_read_only(self) -> bool {
        !self.is_writable()
    }

    /// Canonical string identifier for configuration and error messages.
    pub const fn as_str(self) -> &'static str {
        match self {
            MemoryArea::Coil => "coil",
            MemoryArea::DiscreteInput => "discrete_input",
            MemoryArea::InputRegister => "input_register",
            MemoryArea::HoldingRegister => "holding_register",
        }
    }

    /// Human-facing display title for UI interfaces.
    pub const fn display_name(self) -> &'static str {
        match self {
            MemoryArea::Coil => "Coil",
            MemoryArea::DiscreteInput => "Discrete Input",
            MemoryArea::InputRegister => "Input Register",
            MemoryArea::HoldingRegister => "Holding Register",
        }
    }

    /// Returns true if this memory area addresses single-bit discrete values.
    pub const fn is_discrete(self) -> bool {
        matches!(self, MemoryArea::Coil | MemoryArea::DiscreteInput)
    }
}

impl std::fmt::Display for MemoryArea {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an unrecognized memory area string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Unknown memory area: '{0}' (expected coil, discrete_input, input_register, or holding_register)")]
pub struct ParseMemoryAreaError(pub String);

impl std::str::FromStr for MemoryArea {
    type Err = ParseMemoryAreaError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "coil" | "Coil" => Ok(MemoryArea::Coil),
            "discrete_input" | "DiscreteInput" => Ok(MemoryArea::DiscreteInput),
            "input_register" | "InputRegister" => Ok(MemoryArea::InputRegister),
            "holding_register" | "HoldingRegister" => Ok(MemoryArea::HoldingRegister),
            other => Err(ParseMemoryAreaError(other.to_string())),
        }
    }
}

/// How a 32-bit value is laid out across the two 16-bit registers that carry it.
///
/// Each name is the order of the four bytes A B C D as they appear on the wire,
/// where A is the most significant byte. Encoding `142.75` (`0x430E_C000`) under
/// each variant writes the register pair shown:
///
/// | Variant | Wire order | Registers |
/// |---|---|---|
/// | `BigEndian` | ABCD | `430E C000` |
/// | `WordSwappedBig` | CDAB | `C000 430E` |
/// | `LittleEndian` | DCBA | `00C0 0E43` |
/// | `WordSwappedLittle` | BADC | `0E43 00C0` |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Endianness {
    /// ABCD — high word first, high byte first. Network byte order.
    BigEndian,
    /// CDAB — low word first. What most Schneider Modicon drives expect, and
    /// usually what a comms manual means by "float inverse".
    #[default]
    WordSwappedBig,
    /// DCBA — both words and both bytes reversed.
    LittleEndian,
    /// BADC — word order left alone, but the bytes inside each word swapped.
    WordSwappedLittle,
}

impl Endianness {
    /// The wire order as bytes, for callers that talk in letters (OPC-UA).
    pub const fn wire_order(self) -> &'static str {
        match self {
            Endianness::BigEndian => "ABCD",
            Endianness::WordSwappedBig => "CDAB",
            Endianness::LittleEndian => "DCBA",
            Endianness::WordSwappedLittle => "BADC",
        }
    }
}

/// Primitive telemetry data representations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagDataType {
    Boolean,
    Int16,
    #[serde(rename = "uint16", alias = "u_int16")]
    UInt16,
    Int32,
    #[serde(rename = "uint32", alias = "u_int32")]
    UInt32,
    Float32,
    Float64,
}

impl TagDataType {
    /// How many 16-bit words this type occupies on the wire.
    ///
    /// The one place this arithmetic lives: address-collision validation, the
    /// codec and the Process Image sizing all read it from here, so a new type
    /// cannot be added to one of them and forgotten in the others.
    pub const fn register_span(self) -> usize {
        match self {
            TagDataType::Boolean | TagDataType::Int16 | TagDataType::UInt16 => 1,
            TagDataType::Int32 | TagDataType::UInt32 | TagDataType::Float32 => 2,
            TagDataType::Float64 => 4,
        }
    }

    /// Canonical lowercase snake_case representation matching Serde serialization.
    pub const fn as_str(self) -> &'static str {
        match self {
            TagDataType::Boolean => "boolean",
            TagDataType::Int16 => "int16",
            TagDataType::UInt16 => "uint16",
            TagDataType::Int32 => "int32",
            TagDataType::UInt32 => "uint32",
            TagDataType::Float32 => "float32",
            TagDataType::Float64 => "float64",
        }
    }
}

impl std::fmt::Display for TagDataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an unrecognized tag data type string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Unknown tag data type: '{0}' (expected boolean, int16, uint16, int32, uint32, float32, or float64)")]
pub struct ParseTagDataTypeError(pub String);

impl std::str::FromStr for TagDataType {
    type Err = ParseTagDataTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "boolean" | "Boolean" => Ok(TagDataType::Boolean),
            "int16" | "Int16" => Ok(TagDataType::Int16),
            "uint16" | "UInt16" | "u_int16" => Ok(TagDataType::UInt16),
            "int32" | "Int32" => Ok(TagDataType::Int32),
            "uint32" | "UInt32" | "u_int32" => Ok(TagDataType::UInt32),
            "float32" | "Float32" => Ok(TagDataType::Float32),
            "float64" | "Float64" => Ok(TagDataType::Float64),
            other => Err(ParseTagDataTypeError(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn parses_valid_and_invalid_memory_areas() {
        assert_eq!(
            MemoryArea::from_str("holding_register").unwrap(),
            MemoryArea::HoldingRegister
        );
        assert_eq!(MemoryArea::from_str("coil").unwrap(), MemoryArea::Coil);
        let err = MemoryArea::from_str("unknown_zone").unwrap_err();
        assert_eq!(err.to_string(), "Unknown memory area: 'unknown_zone' (expected coil, discrete_input, input_register, or holding_register)");
    }

    #[test]
    fn parses_valid_and_invalid_data_types() {
        assert_eq!(
            TagDataType::from_str("float32").unwrap(),
            TagDataType::Float32
        );
        assert_eq!(
            TagDataType::from_str("uint16").unwrap(),
            TagDataType::UInt16
        );
        let err = TagDataType::from_str("invalid_type").unwrap_err();
        assert_eq!(err.to_string(), "Unknown tag data type: 'invalid_type' (expected boolean, int16, uint16, int32, uint32, float32, or float64)");
    }
}
