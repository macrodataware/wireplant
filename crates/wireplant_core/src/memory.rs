use std::sync::atomic::Ordering;

use crate::errors::CoreError;
use crate::types::MemoryArea;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Process Image memory representing the four primary Modbus tables.
///
/// Four flat, indexed vectors, because that is how a PLC's memory map behaves:
/// a tag id is resolved to an area and an address once, at load, and every
/// protocol path afterwards is a bounds-checked index.
#[derive(Debug, Clone, Default)]
pub struct PlantMemory {
    pub coils: Vec<bool>,
    pub discrete_inputs: Vec<bool>,
    pub input_registers: Vec<u16>,
    pub holding_registers: Vec<u16>,
}

impl PlantMemory {
    /// Allocates the four tables zero-filled at the given sizes.
    pub fn new(coil_count: usize, di_count: usize, ir_count: usize, hr_count: usize) -> Self {
        Self {
            coils: vec![false; coil_count],
            discrete_inputs: vec![false; di_count],
            input_registers: vec![0; ir_count],
            holding_registers: vec![0; hr_count],
        }
    }

    /// Reads one bit from Coils or Discrete Inputs.
    ///
    /// Asking a register area for a bit is a programming error rather than a
    /// wire error, so it gets its own variant instead of `OutOfBounds`.
    pub fn read_bit(&self, area: MemoryArea, address: usize) -> Result<bool, CoreError> {
        let bits = self.read_bits_slice(area, address, 1)?;
        Ok(bits[0])
    }

    /// Writes one bit to Coils or Discrete Inputs.
    ///
    /// Writing a discrete *input* is legal here even though Modbus treats that
    /// area as read-only — the simulation needs to drive sensor bits from the
    /// process models. The read-only rule is enforced at the protocol edge.
    pub fn write_bit(
        &mut self,
        area: MemoryArea,
        address: usize,
        value: bool,
    ) -> Result<(), CoreError> {
        self.write_bits_slice(area, address, &[value])
    }

    /// Borrow a contiguous run of bits without copying it.
    pub fn read_bits_slice(
        &self,
        area: MemoryArea,
        start: usize,
        count: usize,
    ) -> Result<&[bool], CoreError> {
        let table = self.bit_table(area)?;
        let span = WordSpan::checked(area, table.len(), start, count)?;
        Ok(&table[span.range()])
    }

    /// Write `bits` over the table starting at `address`, all or nothing.
    pub fn write_bits_slice(
        &mut self,
        area: MemoryArea,
        address: usize,
        bits: &[bool],
    ) -> Result<(), CoreError> {
        let capacity = self.bit_table(area)?.len();
        let span = WordSpan::checked(area, capacity, address, bits.len())?;
        let table = self.bit_table_mut(area)?;
        table[span.range()].copy_from_slice(bits);
        Ok(())
    }

    fn bit_table(&self, area: MemoryArea) -> Result<&[bool], CoreError> {
        match area {
            MemoryArea::Coil => Ok(&self.coils),
            MemoryArea::DiscreteInput => Ok(&self.discrete_inputs),
            _ => Err(CoreError::InvalidAreaForBit(area)),
        }
    }

    fn bit_table_mut(&mut self, area: MemoryArea) -> Result<&mut [bool], CoreError> {
        match area {
            MemoryArea::Coil => Ok(&mut self.coils),
            MemoryArea::DiscreteInput => Ok(&mut self.discrete_inputs),
            _ => Err(CoreError::InvalidAreaForBit(area)),
        }
    }

    /// Borrow a contiguous run of registers without copying it.
    ///
    /// This is the fast path the Modbus read handlers use for bulk reads. The
    /// sum is checked even though the `u16` addressing the protocol allows
    /// cannot overflow, because this is a `pub` entry point taking a `usize`.
    pub fn read_registers_slice(
        &self,
        area: MemoryArea,
        start: usize,
        count: usize,
    ) -> Result<&[u16], CoreError> {
        let table = self.register_table(area)?;
        let span = WordSpan::checked(area, table.len(), start, count)?;
        Ok(&table[span.range()])
    }

    /// Write `words` over the registers starting at `address`.
    ///
    /// All-or-nothing: the whole run is bounds-checked before the first word is
    /// written, so a short table cannot leave a half-written value behind for a
    /// client to poll.
    pub fn write_registers_slice(
        &mut self,
        area: MemoryArea,
        address: usize,
        words: &[u16],
    ) -> Result<(), CoreError> {
        let capacity = self.register_table(area)?.len();
        let span = WordSpan::checked(area, capacity, address, words.len())?;
        let table = self.register_table_mut(area)?;
        table[span.range()].copy_from_slice(words);
        Ok(())
    }

    /// The register table for `area`, or an error if it is a bit area.
    fn register_table(&self, area: MemoryArea) -> Result<&[u16], CoreError> {
        match area {
            MemoryArea::InputRegister => Ok(&self.input_registers),
            MemoryArea::HoldingRegister => Ok(&self.holding_registers),
            _ => Err(CoreError::InvalidAreaForRegister(area)),
        }
    }

    fn register_table_mut(&mut self, area: MemoryArea) -> Result<&mut [u16], CoreError> {
        match area {
            MemoryArea::InputRegister => Ok(&mut self.input_registers),
            MemoryArea::HoldingRegister => Ok(&mut self.holding_registers),
            _ => Err(CoreError::InvalidAreaForRegister(area)),
        }
    }
}

/// A bounds-checked run of registers, ready to index with.
///
/// Built once and then used for both the check and the slice, so there is no
/// way to verify one range and then touch another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordSpan {
    start: usize,
    end: usize,
}

impl WordSpan {
    pub fn checked(
        area: MemoryArea,
        capacity: usize,
        start: usize,
        count: usize,
    ) -> Result<Self, CoreError> {
        let end = start.checked_add(count).ok_or(CoreError::OutOfBounds {
            area,
            address: start,
            capacity,
        })?;
        if end > capacity {
            Err(CoreError::OutOfBounds {
                area,
                address: end,
                capacity,
            })
        } else {
            Ok(Self { start, end })
        }
    }

    pub fn range(self) -> std::ops::Range<usize> {
        self.start..self.end
    }
}

/// Handle to the Process Image shared by every protocol server and the engine.
///
/// An `RwLock` rather than a channel because the access pattern is lopsided:
/// SCADA clients poll constantly and concurrently, while only the engine tick
/// writes. The Modbus dispatcher takes it with `try_read`/`try_write` and
/// answers `ServerDeviceBusy` instead of queueing, so a slow client can never
/// stall the simulation loop.
pub type SharedProcessImage = Arc<RwLock<PlantMemory>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_process_image_is_zero_filled_at_the_requested_sizes() {
        let mem = PlantMemory::new(10, 20, 30, 40);
        assert_eq!(mem.coils.len(), 10);
        assert_eq!(mem.discrete_inputs.len(), 20);
        assert_eq!(mem.input_registers.len(), 30);
        assert_eq!(mem.holding_registers.len(), 40);
        assert!(!mem.read_bit(MemoryArea::Coil, 0).unwrap());
        assert_eq!(
            mem.read_registers_slice(MemoryArea::HoldingRegister, 0, 1)
                .unwrap(),
            &[0]
        );
    }

    #[test]
    fn bits_read_back_what_was_written() {
        let mut mem = PlantMemory::new(8, 8, 0, 0);
        mem.write_bit(MemoryArea::Coil, 3, true).unwrap();
        assert!(mem.read_bit(MemoryArea::Coil, 3).unwrap());
        assert!(!mem.read_bit(MemoryArea::Coil, 2).unwrap());

        mem.write_bit(MemoryArea::DiscreteInput, 5, true).unwrap();
        assert!(mem.read_bit(MemoryArea::DiscreteInput, 5).unwrap());

        // Out of bounds
        assert!(matches!(
            mem.read_bit(MemoryArea::Coil, 8),
            Err(CoreError::OutOfBounds { .. })
        ));
        assert!(matches!(
            mem.write_bit(MemoryArea::DiscreteInput, 8, true),
            Err(CoreError::OutOfBounds { .. })
        ));

        // Invalid area
        assert!(matches!(
            mem.read_bit(MemoryArea::InputRegister, 0),
            Err(CoreError::InvalidAreaForBit(_))
        ));
        assert!(matches!(
            mem.write_bit(MemoryArea::HoldingRegister, 0, true),
            Err(CoreError::InvalidAreaForBit(_))
        ));
    }

    #[test]
    fn registers_read_back_what_was_written() {
        let mut mem = PlantMemory::new(0, 0, 16, 16);
        mem.write_registers_slice(MemoryArea::InputRegister, 2, &[1234])
            .unwrap();
        assert_eq!(
            mem.read_registers_slice(MemoryArea::InputRegister, 2, 1)
                .unwrap(),
            &[1234]
        );

        mem.write_registers_slice(MemoryArea::HoldingRegister, 10, &[5678, 9])
            .unwrap();
        assert_eq!(
            mem.read_registers_slice(MemoryArea::HoldingRegister, 10, 2)
                .unwrap(),
            &[5678, 9]
        );

        // Out of bounds, and the failed write left nothing behind.
        assert!(matches!(
            mem.read_registers_slice(MemoryArea::InputRegister, 16, 1),
            Err(CoreError::OutOfBounds { .. })
        ));
        assert!(matches!(
            mem.write_registers_slice(MemoryArea::HoldingRegister, 15, &[1, 2]),
            Err(CoreError::OutOfBounds { .. })
        ));
        assert_eq!(
            mem.read_registers_slice(MemoryArea::HoldingRegister, 15, 1)
                .unwrap(),
            &[0]
        );

        // Invalid area
        assert!(matches!(
            mem.read_registers_slice(MemoryArea::Coil, 0, 1),
            Err(CoreError::InvalidAreaForRegister(_))
        ));
        assert!(matches!(
            mem.write_registers_slice(MemoryArea::DiscreteInput, 0, &[1]),
            Err(CoreError::InvalidAreaForRegister(_))
        ));
    }

    /// A run that cannot fit is refused whole, which is what lets the Modbus
    /// handler claim an FC15/FC16 that failed wrote nothing.
    #[test]
    fn a_bit_run_that_does_not_fit_writes_nothing() {
        let mut mem = PlantMemory::new(8, 8, 0, 0);
        mem.write_bits_slice(MemoryArea::Coil, 6, &[true, true, true])
            .unwrap_err();

        assert_eq!(
            mem.read_bits_slice(MemoryArea::Coil, 6, 2).unwrap(),
            &[false, false]
        );
    }

    /// The success half of this is covered where the pair is decoded; what
    /// matters here is that a run past the end of the table is refused.
    #[test]
    fn a_slice_read_cannot_run_past_the_end_of_its_table() {
        let mem = PlantMemory::new(0, 0, 10, 10);

        let err = mem.read_registers_slice(MemoryArea::HoldingRegister, 8, 3);
        assert!(matches!(err, Err(CoreError::OutOfBounds { .. })));

        let err_area = mem.read_registers_slice(MemoryArea::Coil, 0, 2);
        assert!(matches!(
            err_area,
            Err(CoreError::InvalidAreaForRegister(_))
        ));
    }
}

/// How many protocol sessions are open right now.
///
/// A shared counter rather than a per-server field: the Modbus listener is the
/// only writer, but the REST state endpoint reads it, and both sides already
/// depend on this crate. Whoever owns the servers allocates one and clones it
/// into each.
#[derive(Debug, Clone, Default)]
pub struct SessionCounter(Arc<std::sync::atomic::AtomicUsize>);

impl SessionCounter {
    pub fn active(&self) -> usize {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Claims a slot, or says no if `limit` are already taken.
    ///
    /// Uses `fetch_update` to atomically check the limit and increment the counter
    /// in a single transaction, preventing connection-burst race conditions.
    pub fn try_open(&self, limit: usize) -> Option<SessionGuard> {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |current| {
                if current < limit {
                    Some(current + 1)
                } else {
                    None
                }
            })
            .ok()
            .map(|_| SessionGuard {
                counter: self.clone(),
            })
    }

    fn release(&self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// One open session. Dropping it frees the slot it claimed.
#[derive(Debug)]
pub struct SessionGuard {
    counter: SessionCounter,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.counter.release();
    }
}
