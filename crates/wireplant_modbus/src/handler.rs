use std::future;
use std::sync::Arc;
use tokio_modbus::prelude::{
    ConformityLevel, DeviceIdObject, ExceptionCode, ReadDeviceIdentificationResponse, Request,
    Response, SlaveRequest,
};
use tokio_modbus::server::Service;
use tracing::warn;
use wireplant_core::memory::{PlantMemory, SessionGuard, SharedProcessImage};
use wireplant_core::types::MemoryArea;

// Quantity ceilings from the Modbus Application Protocol V1.1b request PDUs.
// Note they are two different pairs, not one shared limit: FC01/FC02 allow up
// to 2000 bits, FC03/FC04 only 125 registers, because the response payload is
// capped at 253 bytes and two bytes per register eat it twice as fast.
pub const MAX_READ_COILS: u16 = 2000;
pub const MAX_READ_REGISTERS: u16 = 125;
pub const MAX_WRITE_COILS: usize = 1968;
/// Maximum write registers for FC16 (Write Multiple Registers): (253 - 6) / 2 = 123.
pub const MAX_WRITE_REGISTERS_FC16: usize = 123;
/// Maximum write registers for FC23 (Read/Write Multiple Registers): (253 - 10) / 2 = 121.
pub const MAX_WRITE_REGISTERS_FC23: usize = 121;
/// Default write registers ceiling (matches FC16 limit of 123).
pub const MAX_WRITE_REGISTERS: usize = MAX_WRITE_REGISTERS_FC16;

/// Serves Modbus requests out of the Process Image the engine is animating.
///
/// Reads take a shared lock, writes take an exclusive one, and both use the
/// non-blocking `try_*` form: if the engine holds the lock for a tick, the
/// client gets a `ServerDeviceBusy` exception rather than a queued request. A
/// SCADA poller that retries will not notice; a client that holds a lock open
/// would otherwise be able to stall the simulation.
#[derive(Clone, Debug)]
pub struct ModbusHandler {
    shared_memory: SharedProcessImage,
    /// Unit ids the server answers to. Empty means "any", which is what a
    /// scenario that never mentions unit ids expects from a simulator.
    allowed_unit_ids: Vec<u8>,
    /// The connection slot this handler belongs to.
    ///
    /// Held here rather than in the accept closure because the handler lives
    /// exactly as long as the connection does: the slot is freed when the
    /// connection ends, with no separate bookkeeping to get out of step.
    _slot: Arc<Option<SessionGuard>>,
}

impl ModbusHandler {
    pub fn new(
        shared_memory: SharedProcessImage,
        allowed_unit_ids: Vec<u8>,
        slot: Option<SessionGuard>,
    ) -> Self {
        Self {
            shared_memory,
            allowed_unit_ids,
            _slot: Arc::new(slot),
        }
    }

    fn accepts_unit(&self, slave: u8) -> bool {
        self.allowed_unit_ids.is_empty() || self.allowed_unit_ids.contains(&slave)
    }
}

/// Checks a request quantity against the ceiling for its function code.
///
/// Zero is rejected as well as the upper bound: the spec has no meaningful
/// "read nothing" request, and answering it with an empty success hides a
/// malformed client.
fn check_quantity(count: usize, max: usize) -> Result<(), ExceptionCode> {
    if count == 0 || count > max {
        Err(ExceptionCode::IllegalDataValue)
    } else {
        Ok(())
    }
}

/// Reads `count` consecutive bits out of one of the discrete areas.
///
/// The allocation is capped at what the area can actually satisfy. `count` here
/// is a client-supplied field that the codec does not bound, so reserving from
/// it directly would let a malformed request ask for a huge buffer before the
/// first address check rejects it.
fn read_bits(
    mem: &PlantMemory,
    area: MemoryArea,
    addr: u16,
    count: u16,
) -> Result<Vec<bool>, ExceptionCode> {
    let bits = mem
        .read_bits_slice(area, addr as usize, count as usize)
        .map_err(|_| ExceptionCode::IllegalDataAddress)?;
    Ok(bits.to_vec())
}

/// Reads `count` consecutive words out of one of the register areas.
///
/// Same capped-allocation reasoning as [`read_bits`]; the slice read means one
/// bounds check for the whole run instead of one per word.
fn read_words(
    mem: &PlantMemory,
    area: MemoryArea,
    addr: u16,
    count: u16,
) -> Result<Vec<u16>, ExceptionCode> {
    let words = mem
        .read_registers_slice(area, addr as usize, count as usize)
        .map_err(|_| ExceptionCode::IllegalDataAddress)?;
    Ok(words.to_vec())
}

impl Service for ModbusHandler {
    type Request = SlaveRequest<'static>;
    type Response = Response;
    type Exception = ExceptionCode;
    type Future = future::Ready<Result<Self::Response, Self::Exception>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        // The result is computed inline and wrapped once, so the dispatch is a
        // Ready future with no Box and no await point.
        if !self.accepts_unit(req.slave) {
            return future::ready(Err(ExceptionCode::IllegalDataAddress));
        }
        future::ready(self.dispatch(req.request))
    }
}

impl ModbusHandler {
    fn dispatch(&self, req: Request<'static>) -> Result<Response, ExceptionCode> {
        match &req {
            // Reads share one path: all eight handlers differ only in which
            // table they touch and whether the unit is a bit or a word.
            Request::ReadCoils(addr, cnt) => {
                check_quantity(*cnt as usize, MAX_READ_COILS as usize)?;
                let mem = self
                    .shared_memory
                    .try_read()
                    .map_err(|_| ExceptionCode::ServerDeviceBusy)?;
                Ok(Response::ReadCoils(read_bits(
                    &mem,
                    MemoryArea::Coil,
                    *addr,
                    *cnt,
                )?))
            }
            Request::ReadDiscreteInputs(addr, cnt) => {
                check_quantity(*cnt as usize, MAX_READ_COILS as usize)?;
                let mem = self
                    .shared_memory
                    .try_read()
                    .map_err(|_| ExceptionCode::ServerDeviceBusy)?;
                Ok(Response::ReadDiscreteInputs(read_bits(
                    &mem,
                    MemoryArea::DiscreteInput,
                    *addr,
                    *cnt,
                )?))
            }
            Request::ReadHoldingRegisters(addr, cnt) => {
                check_quantity(*cnt as usize, MAX_READ_REGISTERS as usize)?;
                let mem = self
                    .shared_memory
                    .try_read()
                    .map_err(|_| ExceptionCode::ServerDeviceBusy)?;
                Ok(Response::ReadHoldingRegisters(read_words(
                    &mem,
                    MemoryArea::HoldingRegister,
                    *addr,
                    *cnt,
                )?))
            }
            Request::ReadInputRegisters(addr, cnt) => {
                check_quantity(*cnt as usize, MAX_READ_REGISTERS as usize)?;
                let mem = self
                    .shared_memory
                    .try_read()
                    .map_err(|_| ExceptionCode::ServerDeviceBusy)?;
                Ok(Response::ReadInputRegisters(read_words(
                    &mem,
                    MemoryArea::InputRegister,
                    *addr,
                    *cnt,
                )?))
            }

            // Writes need the exclusive lock. The multi-writes check the whole
            // run before touching it, so a bad final address leaves the table
            // as it was rather than half-applied.
            Request::WriteSingleCoil(addr, val) => {
                let mut mem = self
                    .shared_memory
                    .try_write()
                    .map_err(|_| ExceptionCode::ServerDeviceBusy)?;
                mem.write_bit(MemoryArea::Coil, *addr as usize, *val)
                    .map_err(|_| ExceptionCode::IllegalDataAddress)?;
                Ok(Response::WriteSingleCoil(*addr, *val))
            }
            Request::WriteSingleRegister(addr, val) => {
                let mut mem = self
                    .shared_memory
                    .try_write()
                    .map_err(|_| ExceptionCode::ServerDeviceBusy)?;
                mem.write_registers_slice(MemoryArea::HoldingRegister, *addr as usize, &[*val])
                    .map_err(|_| ExceptionCode::IllegalDataAddress)?;
                Ok(Response::WriteSingleRegister(*addr, *val))
            }
            Request::WriteMultipleCoils(addr, coils) => {
                check_quantity(coils.len(), MAX_WRITE_COILS)?;
                let mut mem = self
                    .shared_memory
                    .try_write()
                    .map_err(|_| ExceptionCode::ServerDeviceBusy)?;
                mem.write_bits_slice(MemoryArea::Coil, *addr as usize, coils)
                    .map_err(|_| ExceptionCode::IllegalDataAddress)?;
                Ok(Response::WriteMultipleCoils(*addr, coils.len() as u16))
            }
            Request::WriteMultipleRegisters(addr, words) => {
                check_quantity(words.len(), MAX_WRITE_REGISTERS)?;
                let mut mem = self
                    .shared_memory
                    .try_write()
                    .map_err(|_| ExceptionCode::ServerDeviceBusy)?;
                mem.write_registers_slice(MemoryArea::HoldingRegister, *addr as usize, words)
                    .map_err(|_| ExceptionCode::IllegalDataAddress)?;
                Ok(Response::WriteMultipleRegisters(*addr, words.len() as u16))
            }

            // FC22 (0x16): Mask Write Register
            // Calculates: (Current AND And_Mask) OR (Or_Mask AND NOT And_Mask)
            Request::MaskWriteRegister(addr, and_mask, or_mask) => {
                let mut mem = self
                    .shared_memory
                    .try_write()
                    .map_err(|_| ExceptionCode::ServerDeviceBusy)?;
                let current = mem
                    .read_registers_slice(MemoryArea::HoldingRegister, *addr as usize, 1)
                    .map_err(|_| ExceptionCode::IllegalDataAddress)?[0];
                let updated = (current & *and_mask) | (*or_mask & !*and_mask);
                mem.write_registers_slice(MemoryArea::HoldingRegister, *addr as usize, &[updated])
                    .map_err(|_| ExceptionCode::IllegalDataAddress)?;
                Ok(Response::MaskWriteRegister(*addr, *and_mask, *or_mask))
            }

            // FC23 (0x17): Read/Write Multiple Registers
            // Writes are applied before the read in the same transaction.
            Request::ReadWriteMultipleRegisters(read_addr, read_qty, write_addr, write_words) => {
                check_quantity(*read_qty as usize, MAX_READ_REGISTERS as usize)?;
                check_quantity(write_words.len(), MAX_WRITE_REGISTERS_FC23)?;

                let mut mem = self
                    .shared_memory
                    .try_write()
                    .map_err(|_| ExceptionCode::ServerDeviceBusy)?;

                mem.write_registers_slice(
                    MemoryArea::HoldingRegister,
                    *write_addr as usize,
                    write_words,
                )
                .map_err(|_| ExceptionCode::IllegalDataAddress)?;

                let words = mem
                    .read_registers_slice(
                        MemoryArea::HoldingRegister,
                        *read_addr as usize,
                        *read_qty as usize,
                    )
                    .map_err(|_| ExceptionCode::IllegalDataAddress)?
                    .to_vec();

                Ok(Response::ReadWriteMultipleRegisters(words))
            }

            // FC43 / 0x2B: Encapsulated Interface Transport (Read Device Identification)
            Request::ReadDeviceIdentification(read_code, _object_id) => {
                let objects = vec![
                    DeviceIdObject {
                        id: 0x00,
                        value: tokio_modbus::bytes::Bytes::from_static(b"WirePlant"),
                    },
                    DeviceIdObject {
                        id: 0x01,
                        value: tokio_modbus::bytes::Bytes::from_static(b"WP-SIM"),
                    },
                    DeviceIdObject {
                        id: 0x02,
                        value: tokio_modbus::bytes::Bytes::from_static(b"0.1.0"),
                    },
                ];

                Ok(Response::ReadDeviceIdentification(
                    ReadDeviceIdentificationResponse {
                        read_code: *read_code,
                        conformity_level: ConformityLevel::BasicIdentificationStreamOnly,
                        more_follows: false,
                        next_object_id: 0x00,
                        device_id_objects: objects,
                    },
                ))
            }

            // FC08 (0x08): Diagnostics
            // Subfunction 0x0000 echoes query data back to verify serial/TCP links.
            Request::Custom(0x08, data) => {
                if data.len() < 4 {
                    return Err(ExceptionCode::IllegalDataValue);
                }
                let sub_function = u16::from_be_bytes([data[0], data[1]]);
                match sub_function {
                    // Return Query Data (Loopback)
                    0x0000 | 0x0001 | 0x0004 => Ok(Response::Custom(
                        0x08,
                        tokio_modbus::bytes::Bytes::copy_from_slice(data),
                    )),
                    // Clear Counters and Diagnostic Register
                    0x000A => {
                        let mut resp = [0u8; 4];
                        resp[0..2].copy_from_slice(&data[0..2]);
                        Ok(Response::Custom(
                            0x08,
                            tokio_modbus::bytes::Bytes::copy_from_slice(&resp),
                        ))
                    }
                    // Return Bus Message Count
                    0x000B => {
                        let mut resp = [0u8; 4];
                        resp[0..2].copy_from_slice(&data[0..2]);
                        resp[2..4].copy_from_slice(&1u16.to_be_bytes());
                        Ok(Response::Custom(
                            0x08,
                            tokio_modbus::bytes::Bytes::copy_from_slice(&resp),
                        ))
                    }
                    _ => Err(ExceptionCode::IllegalDataValue),
                }
            }

            _ => {
                warn!("Unsupported Modbus function code received: {:?}", req);
                Err(ExceptionCode::IllegalFunction)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use wireplant_core::memory::PlantMemory;

    fn create_test_handler(
        coils: usize,
        dis: usize,
        irs: usize,
        hrs: usize,
    ) -> (ModbusHandler, SharedProcessImage) {
        let memory = Arc::new(RwLock::new(PlantMemory::new(coils, dis, irs, hrs)));
        let handler = ModbusHandler::new(memory.clone(), vec![1], None);
        (handler, memory)
    }

    /// Every test drives unit 1; the address a request is aimed at is not what
    /// most of them are about.
    fn from_unit1(req: Request<'static>) -> SlaveRequest<'static> {
        SlaveRequest {
            slave: 1,
            request: req,
        }
    }

    #[tokio::test]
    async fn fc01_reads_coils_and_enforces_its_quantity_ceiling() {
        let (handler, memory) = create_test_handler(100, 100, 100, 100);
        {
            let mut mem = memory.write().await;
            mem.coils[0] = true;
            mem.coils[1] = false;
            mem.coils[2] = true;
        }

        // Count = 0 -> IllegalDataValue
        let res = handler.call(from_unit1(Request::ReadCoils(0, 0))).await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataValue));

        // Count > 2000 -> IllegalDataValue
        let res = handler
            .call(from_unit1(Request::ReadCoils(0, MAX_READ_COILS + 1)))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataValue));

        // Valid read
        let res = handler.call(from_unit1(Request::ReadCoils(0, 3))).await;
        assert_eq!(res, Ok(Response::ReadCoils(vec![true, false, true])));

        // Out of bounds -> IllegalDataAddress
        let res = handler.call(from_unit1(Request::ReadCoils(99, 5))).await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataAddress));
    }

    #[tokio::test]
    async fn fc02_reads_discrete_inputs_and_enforces_its_ceiling() {
        let (handler, memory) = create_test_handler(100, 100, 100, 100);

        // Prepopulate discrete input
        {
            let mut mem = memory.write().await;
            mem.discrete_inputs[0] = false;
            mem.discrete_inputs[1] = true;
        }

        // Count = 0 -> IllegalDataValue
        let res = handler
            .call(from_unit1(Request::ReadDiscreteInputs(0, 0)))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataValue));

        // Count > 2000 -> IllegalDataValue
        let res = handler
            .call(from_unit1(Request::ReadDiscreteInputs(
                0,
                MAX_READ_COILS + 1,
            )))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataValue));

        // Valid read
        let res = handler
            .call(from_unit1(Request::ReadDiscreteInputs(0, 2)))
            .await;
        assert_eq!(res, Ok(Response::ReadDiscreteInputs(vec![false, true])));

        // Out of bounds -> IllegalDataAddress
        let res = handler
            .call(from_unit1(Request::ReadDiscreteInputs(99, 5)))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataAddress));
    }

    /// Sizes chosen so each area has its own capacity; the out-of-bounds case
    /// only proves something when the area is smaller than the address asked for.
    fn read_capacity_handler() -> (ModbusHandler, SharedProcessImage) {
        create_test_handler(100, 100, 200, 500)
    }

    fn bit_read_case(area: MemoryArea, addr: u16, count: u16) -> (SlaveRequest<'static>, Response) {
        let (request, response) = match area {
            MemoryArea::Coil => (
                Request::ReadCoils(addr, count),
                Response::ReadCoils(vec![true, false, true]),
            ),
            MemoryArea::DiscreteInput => (
                Request::ReadDiscreteInputs(addr, count),
                Response::ReadDiscreteInputs(vec![false, true]),
            ),
            other => unreachable!("not a bit area: {other:?}"),
        };
        (from_unit1(request), response)
    }

    #[tokio::test]
    async fn bit_reads_honour_the_fc01_fc02_quantity_ceiling() {
        let (handler, memory) = read_capacity_handler();
        {
            let mut mem = memory.write().await;
            mem.coils[0] = true;
            mem.coils[1] = false;
            mem.coils[2] = true;
            mem.discrete_inputs[0] = false;
            mem.discrete_inputs[1] = true;
        }

        // The ceiling is the same for both bit function codes, so assert it on
        // each rather than trusting that one implies the other.
        for area in [MemoryArea::Coil, MemoryArea::DiscreteInput] {
            let (zero, _) = bit_read_case(area, 0, 0);
            assert_eq!(
                handler.call(zero).await,
                Err(ExceptionCode::IllegalDataValue),
                "{area:?}: zero-quantity read should be rejected"
            );

            let (over, _) = bit_read_case(area, 0, MAX_READ_COILS + 1);
            assert_eq!(
                handler.call(over).await,
                Err(ExceptionCode::IllegalDataValue),
                "{area:?}: quantity above MAX_READ_COILS should be rejected"
            );
        }

        let (valid, expected) = bit_read_case(MemoryArea::Coil, 0, 3);
        assert_eq!(handler.call(valid).await, Ok(expected));

        let (valid, expected) = bit_read_case(MemoryArea::DiscreteInput, 0, 2);
        assert_eq!(handler.call(valid).await, Ok(expected));
    }

    #[tokio::test]
    async fn fc03_reads_holding_registers_and_restores_their_values() {
        let (handler, memory) = read_capacity_handler();
        {
            let mut mem = memory.write().await;
            mem.holding_registers[0] = 0x1234;
            mem.holding_registers[1] = 0x5678;
        }

        // Zero and just-over-the-ceiling are both IllegalDataValue...
        for count in [0, MAX_READ_REGISTERS + 1] {
            let res = handler
                .call(from_unit1(Request::ReadHoldingRegisters(0, count)))
                .await;
            assert_eq!(
                res,
                Err(ExceptionCode::IllegalDataValue),
                "quantity {count} should be rejected"
            );
        }

        // ...and exactly the ceiling is legal, which is the boundary that matters.
        let res = handler
            .call(from_unit1(Request::ReadHoldingRegisters(
                0,
                MAX_READ_REGISTERS,
            )))
            .await;
        assert!(res.is_ok(), "quantity at the ceiling should succeed");

        let res = handler
            .call(from_unit1(Request::ReadHoldingRegisters(0, 2)))
            .await;
        assert_eq!(
            res,
            Ok(Response::ReadHoldingRegisters(vec![0x1234, 0x5678]))
        );

        // 499 + 5 runs off the end of a 500-register table.
        let res = handler
            .call(from_unit1(Request::ReadHoldingRegisters(499, 5)))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataAddress));
    }

    #[tokio::test]
    async fn input_registers_read_back_what_was_placed_there() {
        let (handler, memory) = read_capacity_handler();
        {
            let mut mem = memory.write().await;
            mem.input_registers[10] = 42;
            mem.input_registers[11] = 99;
        }

        let res = handler
            .call(from_unit1(Request::ReadInputRegisters(0, 0)))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataValue));

        let res = handler
            .call(from_unit1(Request::ReadInputRegisters(
                0,
                MAX_READ_REGISTERS + 1,
            )))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataValue));

        let res = handler
            .call(from_unit1(Request::ReadInputRegisters(10, 2)))
            .await;
        assert_eq!(res, Ok(Response::ReadInputRegisters(vec![42, 99])));

        // Input registers here are 200 long, unlike the holding table.
        let res = handler
            .call(from_unit1(Request::ReadInputRegisters(199, 5)))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataAddress));
    }

    #[tokio::test]
    async fn single_writes_report_success_and_touch_memory() {
        let (handler, memory) = create_test_handler(50, 50, 50, 50);

        for val in [true, false] {
            let res = handler
                .call(from_unit1(Request::WriteSingleCoil(5, val)))
                .await;
            assert_eq!(res, Ok(Response::WriteSingleCoil(5, val)));
            let mem = memory.read().await;
            assert_eq!(
                mem.coils[5], val,
                "coil 5 should hold {val} after writing it"
            );
        }

        let res = handler
            .call(from_unit1(Request::WriteSingleRegister(7, 12345)))
            .await;
        assert_eq!(res, Ok(Response::WriteSingleRegister(7, 12345)));
        {
            let mem = memory.read().await;
            assert_eq!(mem.holding_registers[7], 12345);
        }

        // One address past the end of the table, for both widths.
        let res = handler
            .call(from_unit1(Request::WriteSingleCoil(99, true)))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataAddress));
        let res = handler
            .call(from_unit1(Request::WriteSingleRegister(99, 12345)))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataAddress));
    }

    #[tokio::test]
    async fn multi_writes_enforce_their_distinct_ceilings() {
        // FC15 and FC16 have different limits (1968 bits vs 123 words), so the
        // tables are sized differently to keep each out-of-range case meaningful.
        let (coil_handler, coil_memory) = create_test_handler(2000, 50, 50, 50);

        let res = coil_handler
            .call(from_unit1(Request::WriteMultipleCoils(
                0,
                Cow::Borrowed(&[]),
            )))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataValue));

        let large_coils = vec![true; MAX_WRITE_COILS + 1];
        let res = coil_handler
            .call(from_unit1(Request::WriteMultipleCoils(
                0,
                Cow::Owned(large_coils),
            )))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataValue));

        let coils_to_write = vec![true, false, true, true];
        let res = coil_handler
            .call(from_unit1(Request::WriteMultipleCoils(
                2,
                Cow::Owned(coils_to_write),
            )))
            .await;
        assert_eq!(res, Ok(Response::WriteMultipleCoils(2, 4)));
        {
            let mem = coil_memory.read().await;
            assert_eq!(
                &mem.coils[2..6],
                &[true, false, true, true],
                "written coils should land in order from the start address"
            );
        }

        let res = coil_handler
            .call(from_unit1(Request::WriteMultipleCoils(
                1999,
                Cow::Borrowed(&[true, false]),
            )))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataAddress));

        let (reg_handler, reg_memory) = create_test_handler(50, 50, 50, 200);

        let res = reg_handler
            .call(from_unit1(Request::WriteMultipleRegisters(
                0,
                Cow::Borrowed(&[]),
            )))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataValue));

        let large_words = vec![123; MAX_WRITE_REGISTERS + 1];
        let res = reg_handler
            .call(from_unit1(Request::WriteMultipleRegisters(
                0,
                Cow::Owned(large_words),
            )))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataValue));

        let words = vec![111, 222, 333];
        let res = reg_handler
            .call(from_unit1(Request::WriteMultipleRegisters(
                10,
                Cow::Owned(words),
            )))
            .await;
        assert_eq!(res, Ok(Response::WriteMultipleRegisters(10, 3)));
        {
            let mem = reg_memory.read().await;
            assert_eq!(&mem.holding_registers[10..13], &[111, 222, 333]);
        }

        let res = reg_handler
            .call(from_unit1(Request::WriteMultipleRegisters(
                199,
                Cow::Borrowed(&[1, 2, 3]),
            )))
            .await;
        assert_eq!(res, Err(ExceptionCode::IllegalDataAddress));
    }

    #[tokio::test]
    async fn an_unsupported_function_code_answers_illegal_function() {
        let (handler, _memory) = create_test_handler(50, 50, 50, 50);
        let res = handler.call(from_unit1(Request::ReportServerId)).await;
        assert_eq!(res, Err(ExceptionCode::IllegalFunction));
    }

    #[tokio::test]
    async fn mask_write_register_applies_and_or_masks_correctly() {
        let (handler, memory) = create_test_handler(50, 50, 50, 50);
        // Set initial register value at address 4 to 0x1234
        {
            let mut mem = memory.write().await;
            mem.holding_registers[4] = 0x1234;
        }

        // Formula: (0x1234 & 0x00FF) | (0xFF00 & !0x00FF) == 0x0034 | 0xFF00 == 0xFF34
        let res = handler
            .call(from_unit1(Request::MaskWriteRegister(4, 0x00FF, 0xFF00)))
            .await;
        assert_eq!(res, Ok(Response::MaskWriteRegister(4, 0x00FF, 0xFF00)));

        {
            let mem = memory.read().await;
            assert_eq!(mem.holding_registers[4], 0xFF34);
        }
    }

    #[tokio::test]
    async fn read_write_multiple_registers_writes_before_reading() {
        let (handler, memory) = create_test_handler(50, 50, 50, 50);
        {
            let mut mem = memory.write().await;
            mem.holding_registers[10] = 0xAAAA;
            mem.holding_registers[11] = 0xBBBB;
        }

        // Write to address 10 and 11, then read them back in the same call
        let write_words = vec![0x1111, 0x2222];
        let res = handler
            .call(from_unit1(Request::ReadWriteMultipleRegisters(
                10,
                2,
                10,
                Cow::Owned(write_words),
            )))
            .await;

        assert_eq!(
            res,
            Ok(Response::ReadWriteMultipleRegisters(vec![0x1111, 0x2222]))
        );
        {
            let mem = memory.read().await;
            assert_eq!(&mem.holding_registers[10..12], &[0x1111, 0x2222]);
        }
    }

    #[tokio::test]
    async fn read_device_identification_returns_basic_objects() {
        let (handler, _memory) = create_test_handler(50, 50, 50, 50);
        let res = handler
            .call(from_unit1(Request::ReadDeviceIdentification(
                tokio_modbus::prelude::ReadCode::Basic,
                0x00,
            )))
            .await;

        match res {
            Ok(Response::ReadDeviceIdentification(resp)) => {
                assert_eq!(resp.device_id_objects.len(), 3);
                assert_eq!(&resp.device_id_objects[0].value[..], b"WirePlant");
                assert_eq!(&resp.device_id_objects[1].value[..], b"WP-SIM");
                assert_eq!(&resp.device_id_objects[2].value[..], b"0.1.0");
            }
            other => panic!(
                "Expected ReadDeviceIdentification response, got {:?}",
                other
            ),
        }
    }

    #[tokio::test]
    async fn diagnostics_loopback_echoes_query_data() {
        let (handler, _memory) = create_test_handler(50, 50, 50, 50);
        let echo_payload = vec![0x00, 0x00, 0xCA, 0xFE];
        let res = handler
            .call(from_unit1(Request::Custom(
                0x08,
                Cow::Owned(echo_payload.clone()),
            )))
            .await;

        match res {
            Ok(Response::Custom(0x08, bytes)) => {
                assert_eq!(&bytes[..], &echo_payload[..]);
            }
            other => panic!("Expected Custom(0x08) response, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn a_unit_id_outside_the_allow_list_is_refused() {
        let memory = Arc::new(RwLock::new(PlantMemory::new(10, 10, 10, 10)));
        let handler = ModbusHandler::new(memory, vec![1, 2], None);

        for slave in [1, 2] {
            let request = SlaveRequest {
                slave,
                request: Request::ReadHoldingRegisters(0, 1),
            };
            assert!(
                handler.call(request).await.is_ok(),
                "unit {slave} is on the allow-list"
            );
        }

        let request = SlaveRequest {
            slave: 7,
            request: Request::ReadHoldingRegisters(0, 1),
        };
        assert_eq!(
            handler.call(request).await,
            Err(ExceptionCode::IllegalDataAddress)
        );
    }

    #[tokio::test]
    async fn an_empty_allow_list_answers_every_unit_id() {
        let memory = Arc::new(RwLock::new(PlantMemory::new(10, 10, 10, 10)));
        let handler = ModbusHandler::new(memory, Vec::new(), None);

        let request = SlaveRequest {
            slave: 247,
            request: Request::ReadInputRegisters(0, 1),
        };
        assert!(handler.call(request).await.is_ok());
    }

    #[tokio::test]
    async fn a_held_write_lock_makes_requests_answer_busy() {
        let (handler, memory) = create_test_handler(50, 50, 50, 50);

        // Acquire exclusive write lock on memory and hold it
        let _write_guard = memory.write().await;

        // Any read or write request should immediately return ServerDeviceBusy
        let res_read = handler.call(from_unit1(Request::ReadCoils(0, 1))).await;
        assert_eq!(res_read, Err(ExceptionCode::ServerDeviceBusy));

        let res_write = handler
            .call(from_unit1(Request::WriteSingleCoil(0, true)))
            .await;
        assert_eq!(res_write, Err(ExceptionCode::ServerDeviceBusy));
    }

    #[tokio::test]
    async fn fc23_enforces_its_121_register_write_ceiling() {
        let (handler, _memory) = create_test_handler(200, 200, 200, 200);

        // 121 words is the legal maximum according to the Modbus specification (PDU length limit)
        let legal_write = vec![0x1234; 121];
        let req_legal = from_unit1(Request::ReadWriteMultipleRegisters(
            0,
            1,
            0,
            legal_write.into(),
        ));
        assert!(handler.call(req_legal).await.is_ok());

        // 122 words exceeds the protocol limit and must return IllegalDataValue
        let illegal_write = vec![0x1234; 122];
        let req_illegal = from_unit1(Request::ReadWriteMultipleRegisters(
            0,
            1,
            0,
            illegal_write.into(),
        ));
        assert_eq!(
            handler.call(req_illegal).await,
            Err(ExceptionCode::IllegalDataValue)
        );
    }
}
