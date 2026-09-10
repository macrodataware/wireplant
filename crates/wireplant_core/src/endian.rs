//! Splitting 32-bit values across Modbus register pairs.
//!
//! Modbus registers are 16 bits, so an `f32` or `i32` needs two of them. The
//! awkward part is that "which two, in what order" is vendor-specific — see
//! [`Endianness`] for the four layouts and the byte order each one produces.
//! A pair is always returned as `(first, second)` in the order it should be
//! written to consecutive addresses.

use crate::types::Endianness;

/// Splits an IEEE-754 `f32` into the register pair to write at `addr` and `addr + 1`.
///
/// The returned tuple is in wire transmission order: element 0 maps to base `addr`,
/// and element 1 maps to `addr + 1`.
///
/// # Examples
///
/// In standard Modicon layout (`WordSwappedBig`), `142.75` (`0x430E_C000`) places
/// the least significant 16-bit word (`0xC000`) at register 0:
///
/// ```
/// use wireplant_core::endian::{float32_to_registers, registers_to_float32};
/// use wireplant_core::types::Endianness;
///
/// let (r0, r1) = float32_to_registers(142.75, Endianness::WordSwappedBig);
/// assert_eq!((r0, r1), (0xC000, 0x430E));
/// assert_eq!(registers_to_float32(r0, r1, Endianness::WordSwappedBig), 142.75);
/// ```
pub fn float32_to_registers(val: f32, endianness: Endianness) -> (u16, u16) {
    let bits = val.to_bits();
    let word_hi = ((bits >> 16) & 0xFFFF) as u16;
    let word_lo = (bits & 0xFFFF) as u16;

    match endianness {
        Endianness::BigEndian => (word_hi, word_lo),
        Endianness::WordSwappedBig => (word_lo, word_hi),
        Endianness::LittleEndian => {
            let swap_hi = word_hi.swap_bytes();
            let swap_lo = word_lo.swap_bytes();
            (swap_lo, swap_hi)
        }
        Endianness::WordSwappedLittle => {
            let swap_hi = word_hi.swap_bytes();
            let swap_lo = word_lo.swap_bytes();
            (swap_hi, swap_lo)
        }
    }
}

/// Rebuilds an IEEE-754 `f32` from register pair `(reg0, reg1)`.
pub fn registers_to_float32(reg0: u16, reg1: u16, endianness: Endianness) -> f32 {
    let (word_hi, word_lo) = match endianness {
        Endianness::BigEndian => (reg0, reg1),
        Endianness::WordSwappedBig => (reg1, reg0),
        Endianness::LittleEndian => (reg1.swap_bytes(), reg0.swap_bytes()),
        Endianness::WordSwappedLittle => (reg0.swap_bytes(), reg1.swap_bytes()),
    };

    let bits = ((word_hi as u32) << 16) | (word_lo as u32);
    f32::from_bits(bits)
}

/// Splits an `i32` into the register pair to write at `addr` and `addr + 1`.
///
/// Same layout rules as [`float32_to_registers`]; the sign is preserved by
/// reinterpreting through `u32` rather than doing arithmetic.
pub fn int32_to_registers(val: i32, endianness: Endianness) -> (u16, u16) {
    let bits = val as u32;
    let word_hi = ((bits >> 16) & 0xFFFF) as u16;
    let word_lo = (bits & 0xFFFF) as u16;

    match endianness {
        Endianness::BigEndian => (word_hi, word_lo),
        Endianness::WordSwappedBig => (word_lo, word_hi),
        Endianness::LittleEndian => (word_lo.swap_bytes(), word_hi.swap_bytes()),
        Endianness::WordSwappedLittle => (word_hi.swap_bytes(), word_lo.swap_bytes()),
    }
}

/// Rebuilds an `i32` from the register pair at `addr` and `addr + 1`.
pub fn registers_to_int32(reg0: u16, reg1: u16, endianness: Endianness) -> i32 {
    let (word_hi, word_lo) = match endianness {
        Endianness::BigEndian => (reg0, reg1),
        Endianness::WordSwappedBig => (reg1, reg0),
        Endianness::LittleEndian => (reg1.swap_bytes(), reg0.swap_bytes()),
        Endianness::WordSwappedLittle => (reg0.swap_bytes(), reg1.swap_bytes()),
    };

    let bits = ((word_hi as u32) << 16) | (word_lo as u32);
    bits as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::PlantMemory;
    use crate::types::MemoryArea;

    /// Pins the byte order each variant actually produces, not just that the
    /// pair survives a round-trip — a broken ordering round-trips fine against
    /// itself, which is how a mislabelled variant hides.
    #[test]
    fn wire_byte_order_matches_the_documented_layout() {
        let val = 142.75f32; // 0x430E_C000
        let expected = [
            (Endianness::BigEndian, "430EC000"),
            (Endianness::WordSwappedBig, "C000430E"),
            (Endianness::LittleEndian, "00C00E43"),
            (Endianness::WordSwappedLittle, "0E4300C0"),
        ];

        for (endian, want) in expected {
            let (r0, r1) = float32_to_registers(val, endian);
            let got: String = [r0, r1]
                .iter()
                .flat_map(|w| w.to_be_bytes())
                .map(|b| format!("{b:02X}"))
                .collect();
            assert_eq!(got, want, "wrong wire order for {endian:?}");
        }
    }

    #[test]
    fn float_roundtrips_under_every_word_order() {
        let cases = [
            (142.75f32, Endianness::BigEndian),
            (-55.125, Endianness::WordSwappedBig),
            (1234.5678, Endianness::LittleEndian),
            (-987.6543, Endianness::WordSwappedLittle),
        ];

        for (input, endian) in cases {
            let (r0, r1) = float32_to_registers(input, endian);
            let output = registers_to_float32(r0, r1, endian);
            assert_eq!(input, output, "round-trip failed for {endian:?}");
        }
    }

    #[test]
    fn int32_roundtrips_under_every_word_order() {
        let test_cases = [0i32, 1, -1, 12345678, -87654321, i32::MAX, i32::MIN];
        let endians = [
            Endianness::BigEndian,
            Endianness::WordSwappedBig,
            Endianness::LittleEndian,
            Endianness::WordSwappedLittle,
        ];

        for &val in &test_cases {
            for &endian in &endians {
                let (r0, r1) = int32_to_registers(val, endian);
                let decoded = registers_to_int32(r0, r1, endian);
                assert_eq!(val, decoded, "Failed for val {val} with endian {endian:?}");
            }
        }
    }

    /// Reads the pair back through the Process Image rather than calling the
    /// decoder directly — the src/dst registers are where a slice bug would show.
    #[test]
    fn register_pairs_survive_the_process_image() {
        let mut memory = PlantMemory::new(10, 10, 10, 100);
        memory
            .write_registers_slice(MemoryArea::HoldingRegister, 5, &[42, 43])
            .unwrap();

        let slice = memory
            .read_registers_slice(MemoryArea::HoldingRegister, 5, 2)
            .unwrap();
        assert_eq!(slice, &[42, 43]);
    }
}
