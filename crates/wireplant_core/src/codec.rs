//! The one place a tag's value crosses between memory and a number.
//!
//! Every path that reads or writes a tag — Modbus dispatch, the REST routes, the
//! engine tick, the TUI overrides — goes through [`decode_tag`] and
//! [`encode_tag`]. Decoding is total for the type/area pairs the schema allows;
//! encoding refuses a combination that cannot fit instead of writing a
//! plausible-looking zero.
//!
//! A 32-bit tag is split across two consecutive registers and a `Float64` across
//! four, in the layout [`Endianness`] names. Whoever writes the low word has to
//! be whoever writes the high word, which is why these live together rather than
//! in three copies that can drift.

use crate::endian::{
    float32_to_registers, int32_to_registers, registers_to_float32, registers_to_int32,
};
use crate::errors::CoreError;
use crate::memory::PlantMemory;
use crate::types::{Endianness, MemoryArea, TagDataType};

/// At or above this, a floating-point tag reads as true.
///
/// The single threshold in the workspace: every `>= 0.5` that used to be spelled
/// out at a call site reads this instead.
pub const BOOL_THRESHOLD: f64 = 0.5;

/// What the codec needs to know about a tag, however the caller is holding it.
///
/// Implemented for the scenario's `TagConfig` and for the engine's `TagBinding`,
/// so neither has to unpack itself into a five-argument call.
pub trait TagAddress {
    fn area(&self) -> MemoryArea;
    fn address(&self) -> usize;
    fn data_type(&self) -> TagDataType;
    fn endianness(&self) -> Endianness;
}

/// Reads one tag out of the Process Image.
pub fn decode_tag<T: TagAddress + ?Sized>(memory: &PlantMemory, tag: &T) -> Result<f64, CoreError> {
    decode(
        memory,
        tag.area(),
        tag.address(),
        tag.data_type(),
        tag.endianness(),
    )
}

/// Writes one tag into the Process Image, all words or none.
pub fn encode_tag<T: TagAddress + ?Sized>(
    memory: &mut PlantMemory,
    tag: &T,
    value: f64,
) -> Result<(), CoreError> {
    encode(
        memory,
        tag.area(),
        tag.address(),
        tag.data_type(),
        tag.endianness(),
        value,
    )
}

/// Reads a typed tag straight out of one of the four tables.
pub fn decode(
    memory: &PlantMemory,
    area: MemoryArea,
    address: usize,
    data_type: TagDataType,
    endianness: Endianness,
) -> Result<f64, CoreError> {
    match (area, data_type) {
        (MemoryArea::Coil | MemoryArea::DiscreteInput, TagDataType::Boolean) => {
            Ok(f64::from(u8::from(memory.read_bit(area, address)?)))
        }
        (MemoryArea::InputRegister | MemoryArea::HoldingRegister, dtype) => {
            let words = memory.read_registers_slice(area, address, dtype.register_span())?;
            Ok(decode_words(dtype, words, endianness))
        }
        (area, data_type) => Err(CoreError::DataTypeNotInArea { area, data_type }),
    }
}

/// Writes a typed tag into one of the four tables.
pub fn encode(
    memory: &mut PlantMemory,
    area: MemoryArea,
    address: usize,
    data_type: TagDataType,
    endianness: Endianness,
    value: f64,
) -> Result<(), CoreError> {
    match (area, data_type) {
        (MemoryArea::Coil | MemoryArea::DiscreteInput, TagDataType::Boolean) => {
            memory.write_bit(area, address, value >= BOOL_THRESHOLD)
        }
        (MemoryArea::InputRegister | MemoryArea::HoldingRegister, dtype) => {
            memory.write_registers_slice(area, address, &encode_words(dtype, value, endianness))
        }
        (area, data_type) => Err(CoreError::DataTypeNotInArea { area, data_type }),
    }
}

fn decode_words(data_type: TagDataType, words: &[u16], endianness: Endianness) -> f64 {
    match data_type {
        TagDataType::Boolean => f64::from(u8::from(words[0] != 0)),
        TagDataType::Int16 => f64::from(words[0] as i16),
        TagDataType::UInt16 => f64::from(words[0]),
        TagDataType::Int32 => f64::from(registers_to_int32(words[0], words[1], endianness)),
        TagDataType::UInt32 => f64::from(registers_to_int32(words[0], words[1], endianness) as u32),
        TagDataType::Float32 => f64::from(registers_to_float32(words[0], words[1], endianness)),
        TagDataType::Float64 => f64::from_bits(decode_u64(words, endianness)),
    }
}

fn encode_words(data_type: TagDataType, value: f64, endianness: Endianness) -> Vec<u16> {
    match data_type {
        // A coil held as a number uses the same threshold as everything else, so
        // one value reads back identically over Modbus, REST and OPC-UA.
        TagDataType::Boolean => vec![u16::from(value >= BOOL_THRESHOLD)],
        TagDataType::Int16 => vec![(value as i16) as u16],
        TagDataType::UInt16 => vec![value as u16],
        TagDataType::Int32 => {
            let (r0, r1) = int32_to_registers(value as i32, endianness);
            vec![r0, r1]
        }
        TagDataType::UInt32 => {
            let (r0, r1) = int32_to_registers((value as u32) as i32, endianness);
            vec![r0, r1]
        }
        TagDataType::Float32 => {
            let (r0, r1) = float32_to_registers(value as f32, endianness);
            vec![r0, r1]
        }
        TagDataType::Float64 => encode_u64(value.to_bits(), endianness).to_vec(),
    }
}

/// `Float64` follows the same word-order rule as the 32-bit types: `BigEndian`
/// and `LittleEndian` are byte-reverse mirror images in the 32-bit case and the
/// same word sequence here, while the two "swapped" variants put the low half
/// first.
///
/// Modbus defines no 64-bit layout to copy, so treat this as the convention the
/// simulator uses rather than a spec. No bundled scenario exercises it.
fn decode_u64(words: &[u16], endianness: Endianness) -> u64 {
    let (w0, w1, w2, w3) = match endianness {
        Endianness::BigEndian => (words[0], words[1], words[2], words[3]),
        Endianness::LittleEndian => (
            words[3].swap_bytes(),
            words[2].swap_bytes(),
            words[1].swap_bytes(),
            words[0].swap_bytes(),
        ),
        Endianness::WordSwappedBig => (words[2], words[3], words[0], words[1]),
        Endianness::WordSwappedLittle => (
            words[2].swap_bytes(),
            words[3].swap_bytes(),
            words[0].swap_bytes(),
            words[1].swap_bytes(),
        ),
    };

    (u64::from(w0) << 48) | (u64::from(w1) << 32) | (u64::from(w2) << 16) | u64::from(w3)
}

fn encode_u64(bits: u64, endianness: Endianness) -> [u16; 4] {
    let w0 = ((bits >> 48) & 0xFFFF) as u16;
    let w1 = ((bits >> 32) & 0xFFFF) as u16;
    let w2 = ((bits >> 16) & 0xFFFF) as u16;
    let w3 = (bits & 0xFFFF) as u16;

    match endianness {
        Endianness::BigEndian => [w0, w1, w2, w3],
        Endianness::LittleEndian => [
            w3.swap_bytes(),
            w2.swap_bytes(),
            w1.swap_bytes(),
            w0.swap_bytes(),
        ],
        Endianness::WordSwappedBig => [w2, w3, w0, w1],
        Endianness::WordSwappedLittle => [
            w2.swap_bytes(),
            w3.swap_bytes(),
            w0.swap_bytes(),
            w1.swap_bytes(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> PlantMemory {
        PlantMemory::new(8, 8, 32, 32)
    }

    #[test]
    fn a_float32_written_at_an_address_reads_back_at_the_same_one() {
        for endianness in [
            Endianness::BigEndian,
            Endianness::WordSwappedBig,
            Endianness::LittleEndian,
            Endianness::WordSwappedLittle,
        ] {
            let mut mem = memory();
            encode(
                &mut mem,
                MemoryArea::HoldingRegister,
                4,
                TagDataType::Float32,
                endianness,
                88.75,
            )
            .unwrap();

            let read_back = decode(
                &mem,
                MemoryArea::HoldingRegister,
                4,
                TagDataType::Float32,
                endianness,
            )
            .unwrap();
            assert_eq!(read_back, 88.75, "round trip failed for {endianness:?}");
        }
    }

    #[test]
    fn every_declared_type_fits_the_area_it_is_asked_to_live_in() {
        let mut mem = memory();
        for data_type in [
            TagDataType::Int16,
            TagDataType::UInt16,
            TagDataType::Int32,
            TagDataType::UInt32,
            TagDataType::Float32,
            TagDataType::Float64,
        ] {
            encode(
                &mut mem,
                MemoryArea::InputRegister,
                10,
                data_type,
                Endianness::BigEndian,
                7.0,
            )
            .unwrap_or_else(|e| panic!("{data_type:?} refused to encode: {e}"));

            let read_back = decode(
                &mem,
                MemoryArea::InputRegister,
                10,
                data_type,
                Endianness::BigEndian,
            )
            .unwrap();
            assert_eq!(
                read_back, 7.0,
                "{data_type:?} did not survive the round trip"
            );
        }
    }

    #[test]
    fn a_boolean_lives_in_bits_and_nothing_else_does() {
        let mut mem = memory();
        encode(
            &mut mem,
            MemoryArea::Coil,
            3,
            TagDataType::Boolean,
            Endianness::BigEndian,
            1.0,
        )
        .unwrap();
        assert!(mem.read_bit(MemoryArea::Coil, 3).unwrap());
        assert_eq!(
            decode(
                &mem,
                MemoryArea::Coil,
                3,
                TagDataType::Boolean,
                Endianness::BigEndian
            )
            .unwrap(),
            1.0
        );

        // A float in a coil is a schema mistake, not a value to invent.
        let err = encode(
            &mut mem,
            MemoryArea::Coil,
            3,
            TagDataType::Float32,
            Endianness::BigEndian,
            1.0,
        )
        .unwrap_err();
        assert!(matches!(err, CoreError::DataTypeNotInArea { .. }));
    }

    #[test]
    fn a_tag_too_close_to_the_end_is_refused_rather_than_half_written() {
        let mut mem = PlantMemory::new(0, 0, 4, 0);
        let err = encode(
            &mut mem,
            MemoryArea::InputRegister,
            3,
            TagDataType::Float32,
            Endianness::BigEndian,
            1.5,
        )
        .unwrap_err();
        assert!(matches!(err, CoreError::OutOfBounds { .. }));
        assert_eq!(
            mem.input_registers[3], 0,
            "nothing should have been written"
        );

        let err = decode(
            &mem,
            MemoryArea::InputRegister,
            3,
            TagDataType::Float64,
            Endianness::BigEndian,
        )
        .unwrap_err();
        assert!(matches!(err, CoreError::OutOfBounds { .. }));
    }

    #[test]
    fn the_boolean_threshold_is_the_only_one() {
        let mut mem = memory();
        encode(
            &mut mem,
            MemoryArea::Coil,
            0,
            TagDataType::Boolean,
            Endianness::BigEndian,
            BOOL_THRESHOLD,
        )
        .unwrap();
        assert!(mem.read_bit(MemoryArea::Coil, 0).unwrap());

        encode(
            &mut mem,
            MemoryArea::Coil,
            0,
            TagDataType::Boolean,
            Endianness::BigEndian,
            BOOL_THRESHOLD - f64::EPSILON,
        )
        .unwrap();
        assert!(!mem.read_bit(MemoryArea::Coil, 0).unwrap());
    }

    /// Float64 spans four words, so its layout is the easiest thing here to get
    /// wrong quietly. This pins the words rather than a round trip.
    #[test]
    fn float64_puts_its_words_where_the_layout_says() {
        // 1.0f64 is 0x3FF0_0000_0000_0000.
        let cases = [
            (Endianness::BigEndian, [0x3FF0, 0x0000, 0x0000, 0x0000]),
            (Endianness::LittleEndian, [0x0000, 0x0000, 0x0000, 0xF03F]),
            (Endianness::WordSwappedBig, [0x0000, 0x0000, 0x3FF0, 0x0000]),
            (
                Endianness::WordSwappedLittle,
                [0x0000, 0x0000, 0xF03F, 0x0000],
            ),
        ];

        for (endianness, expected) in cases {
            assert_eq!(
                encode_u64(1.0f64.to_bits(), endianness),
                expected,
                "wrong word order for {endianness:?}"
            );
            assert_eq!(decode_u64(&expected, endianness), 1.0f64.to_bits());
        }
    }

    #[test]
    fn float64_round_trip_all_endianness() {
        for endianness in [
            Endianness::BigEndian,
            Endianness::WordSwappedBig,
            Endianness::LittleEndian,
            Endianness::WordSwappedLittle,
        ] {
            let mut mem = memory();
            encode(
                &mut mem,
                MemoryArea::HoldingRegister,
                8,
                TagDataType::Float64,
                endianness,
                123456.7890123,
            )
            .unwrap();

            let read_back = decode(
                &mem,
                MemoryArea::HoldingRegister,
                8,
                TagDataType::Float64,
                endianness,
            )
            .unwrap();
            assert_eq!(
                read_back, 123456.7890123,
                "round trip failed for {endianness:?}"
            );
        }
    }
}
