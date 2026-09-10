//! Fuzzing for the 32-bit conversions.
//!
//! A round trip alone is not an oracle: a codec that swaps the words on the way
//! out and again on the way back passes it while putting the wrong bytes on the
//! wire. So the main property reorders the four raw bytes by hand and checks the
//! encoder against that.

use proptest::prelude::*;
use wireplant_core::endian::{
    float32_to_registers, int32_to_registers, registers_to_float32, registers_to_int32,
};
use wireplant_core::types::Endianness;

/// The four bytes of a 32-bit value in the order the wire carries them, worked
/// out from the `Endianness` documentation rather than from the encoder.
fn wire_bytes(bits: u32, endianness: Endianness) -> [u8; 4] {
    let abcd = bits.to_be_bytes();
    match endianness {
        Endianness::BigEndian => abcd,
        Endianness::WordSwappedBig => [abcd[2], abcd[3], abcd[0], abcd[1]],
        Endianness::LittleEndian => [abcd[3], abcd[2], abcd[1], abcd[0]],
        Endianness::WordSwappedLittle => [abcd[1], abcd[0], abcd[3], abcd[2]],
    }
}

fn expected_pair(bits: u32, endianness: Endianness) -> (u16, u16) {
    let bytes = wire_bytes(bits, endianness);
    (
        u16::from_be_bytes([bytes[0], bytes[1]]),
        u16::from_be_bytes([bytes[2], bytes[3]]),
    )
}

fn any_endianness() -> impl Strategy<Value = Endianness> {
    prop_oneof![
        Just(Endianness::BigEndian),
        Just(Endianness::WordSwappedBig),
        Just(Endianness::LittleEndian),
        Just(Endianness::WordSwappedLittle),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    /// Every f32 goes to the word pair the documented byte order demands.
    #[test]
    fn float32_encodes_to_the_documented_wire_order(
        bits in any::<u32>(),
        endian in any_endianness(),
    ) {
        let value = f32::from_bits(bits);
        prop_assume!(!value.is_nan());
        prop_assert_eq!(float32_to_registers(value, endian), expected_pair(bits, endian));
    }

    #[test]
    fn int32_encodes_to_the_documented_wire_order(bits in any::<u32>(), endian in any_endianness()) {
        prop_assert_eq!(int32_to_registers(bits as i32, endian), expected_pair(bits, endian));
    }

    /// ...and decodes back to the same number.
    #[test]
    fn float32_survives_a_round_trip(
        val in any::<f32>().prop_filter("NaN has no single bit pattern", |x| !x.is_nan()),
        endian in any_endianness(),
    ) {
        let (r0, r1) = float32_to_registers(val, endian);
        prop_assert_eq!(registers_to_float32(r0, r1, endian), val);
    }

    #[test]
    fn int32_survives_a_round_trip(val in any::<i32>(), endian in any_endianness()) {
        let (r0, r1) = int32_to_registers(val, endian);
        prop_assert_eq!(registers_to_int32(r0, r1, endian), val);
    }
}

/// The layouts the documentation commits to, spelled out. These are the vectors
/// a vendor manual would give, so they do not move when the code does.
#[test]
fn the_documented_layouts_are_the_ones_the_codec_produces() {
    let cases = [
        (Endianness::BigEndian, (0x430E, 0xC000)),
        (Endianness::WordSwappedBig, (0xC000, 0x430E)),
        (Endianness::LittleEndian, (0x00C0, 0x0E43)),
        (Endianness::WordSwappedLittle, (0x0E43, 0x00C0)),
    ];

    for (endianness, expected) in cases {
        assert_eq!(
            float32_to_registers(142.75, endianness),
            expected,
            "wrong pair for {endianness:?}"
        );
        assert_eq!(
            registers_to_float32(expected.0, expected.1, endianness),
            142.75
        );
    }
}
