//! Mock [`TraceBackend`](super::TraceBackend) for unit-testing
//! investigators without touching the SDK.
//!
//! The mock is intentionally simple: a precomputed sequence of register
//! snapshots and writes for one thread, plus a fixed module list, a
//! fixed memory image, and an optional termination event. It is *not* a
//! TTD simulator; it just lets us drive the investigators end-to-end
//! against scenarios we hand-construct.

use std::collections::BTreeMap;

use super::*;

/// One register-write event in the mock timeline.
#[derive(Debug, Clone)]
pub struct MockWrite {
    pub position: Position,
    pub reg: RegId,
    pub value: u64,
    pub ip: u64,
}

#[derive(Debug, Default, Clone)]
pub struct MockTrace {
    pub thread: ThreadId,
    pub modules: Vec<ModuleInfo>,
    pub termination: Option<TerminationEvent>,
    /// Register snapshots keyed by position. The snapshot at the
    /// greatest position `<= p` is treated as the state at `p`.
    pub register_snapshots: BTreeMap<Position, Registers>,
    /// Memory bytes at fixed addresses; constant across positions for
    /// the mock. Maps base address -> bytes.
    pub memory: BTreeMap<u64, Vec<u8>>,
    /// Stack at termination (or at any queried position; mock ignores
    /// position for stack).
    pub stack_at_termination: Vec<RawFrame>,
    /// Ordered write timeline used for `last_write_register`.
    pub writes: Vec<MockWrite>,
}

impl MockTrace {
    pub fn new(thread: ThreadId) -> Self {
        Self {
            thread,
            ..Default::default()
        }
    }
}

impl TraceBackend for MockTrace {
    fn modules(&self) -> BackendResult<Vec<ModuleInfo>> {
        Ok(self.modules.clone())
    }

    fn termination(&self) -> BackendResult<Option<TerminationEvent>> {
        Ok(self.termination.clone())
    }

    fn registers(&self, thread: ThreadId, position: Position) -> BackendResult<Registers> {
        if thread != self.thread {
            return Err(BackendError::UnknownThread);
        }
        let snap = self
            .register_snapshots
            .range(..=position)
            .next_back()
            .map(|(_, r)| r.clone())
            .ok_or(BackendError::InvalidPosition)?;
        Ok(snap)
    }

    fn read_memory(
        &self,
        _position: Position,
        address: u64,
        len: usize,
    ) -> BackendResult<Vec<u8>> {
        // Find the range that contains [address, address+len).
        if let Some((&base, bytes)) = self.memory.range(..=address).next_back() {
            let end = base.saturating_add(bytes.len() as u64);
            if address >= base && address.saturating_add(len as u64) <= end {
                let off = (address - base) as usize;
                return Ok(bytes[off..off + len].to_vec());
            }
        }
        Err(BackendError::OutOfRange)
    }

    fn stack(&self, thread: ThreadId, _position: Position) -> BackendResult<Vec<RawFrame>> {
        if thread != self.thread {
            return Err(BackendError::UnknownThread);
        }
        Ok(self.stack_at_termination.clone())
    }

    fn last_write_register(
        &self,
        thread: ThreadId,
        reg: RegId,
        before: Position,
    ) -> BackendResult<Option<WriteRecord>> {
        if thread != self.thread {
            return Err(BackendError::UnknownThread);
        }
        let hit = self
            .writes
            .iter()
            .filter(|w| w.reg == reg && w.position < before)
            .max_by_key(|w| w.position);
        Ok(hit.map(|w| WriteRecord {
            position: w.position,
            thread,
            ip: w.ip,
            value: w.value,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(s: u64, t: u64) -> Position {
        Position { sequence: s, steps: t }
    }

    fn empty() -> MockTrace {
        MockTrace::new(ThreadId(1))
    }

    #[test]
    fn modules_starts_empty() {
        assert!(empty().modules().unwrap().is_empty());
    }

    #[test]
    fn termination_starts_none() {
        assert!(empty().termination().unwrap().is_none());
    }

    #[test]
    fn unknown_thread_for_registers() {
        let mock = empty();
        assert!(matches!(
            mock.registers(ThreadId(2), pos(0, 0)),
            Err(BackendError::UnknownThread)
        ));
    }

    #[test]
    fn registers_invalid_position_when_no_snapshots() {
        let mock = empty();
        assert!(matches!(
            mock.registers(ThreadId(1), pos(0, 0)),
            Err(BackendError::InvalidPosition)
        ));
    }

    #[test]
    fn registers_returns_latest_snapshot_at_or_before_position() {
        let mut mock = empty();
        let r0 = Registers { rax: 1, ..Default::default() };
        let r1 = Registers { rax: 2, ..Default::default() };
        mock.register_snapshots.insert(pos(10, 0), r0);
        mock.register_snapshots.insert(pos(20, 0), r1);

        assert_eq!(
            mock.registers(ThreadId(1), pos(15, 0)).unwrap().rax,
            1
        );
        assert_eq!(
            mock.registers(ThreadId(1), pos(20, 0)).unwrap().rax,
            2
        );
        assert_eq!(
            mock.registers(ThreadId(1), pos(99, 0)).unwrap().rax,
            2
        );
    }

    #[test]
    fn read_memory_returns_bytes_in_range() {
        let mut mock = empty();
        mock.memory.insert(0x1000, vec![0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(
            mock.read_memory(pos(0, 0), 0x1001, 2).unwrap(),
            vec![0xBB, 0xCC]
        );
    }

    #[test]
    fn read_memory_out_of_range() {
        let mut mock = empty();
        mock.memory.insert(0x1000, vec![0; 4]);
        assert!(matches!(
            mock.read_memory(pos(0, 0), 0x2000, 4),
            Err(BackendError::OutOfRange)
        ));
    }

    #[test]
    fn read_memory_crossing_range_end_fails() {
        let mut mock = empty();
        mock.memory.insert(0x1000, vec![0; 4]);
        assert!(matches!(
            mock.read_memory(pos(0, 0), 0x1003, 2),
            Err(BackendError::OutOfRange)
        ));
    }

    #[test]
    fn last_write_register_finds_most_recent_before_position() {
        let mut mock = empty();
        mock.writes.push(MockWrite {
            position: pos(10, 0),
            reg: RegId::Rax,
            value: 0,
            ip: 0x1000,
        });
        mock.writes.push(MockWrite {
            position: pos(20, 0),
            reg: RegId::Rax,
            value: 42,
            ip: 0x1010,
        });
        mock.writes.push(MockWrite {
            position: pos(30, 0),
            reg: RegId::Rbx,
            value: 99,
            ip: 0x1020,
        });

        let w = mock
            .last_write_register(ThreadId(1), RegId::Rax, pos(25, 0))
            .unwrap()
            .unwrap();
        assert_eq!(w.value, 42);
        assert_eq!(w.position, pos(20, 0));
    }

    #[test]
    fn last_write_register_strictly_before() {
        let mut mock = empty();
        mock.writes.push(MockWrite {
            position: pos(20, 0),
            reg: RegId::Rax,
            value: 7,
            ip: 0x1000,
        });
        // Asking for "before pos(20)" must exclude the write *at* pos(20).
        assert!(mock
            .last_write_register(ThreadId(1), RegId::Rax, pos(20, 0))
            .unwrap()
            .is_none());
    }

    #[test]
    fn last_write_register_none_when_no_writes() {
        assert!(empty()
            .last_write_register(ThreadId(1), RegId::Rax, pos(100, 0))
            .unwrap()
            .is_none());
    }

    #[test]
    fn last_write_register_unknown_thread() {
        assert!(matches!(
            empty().last_write_register(ThreadId(99), RegId::Rax, pos(100, 0)),
            Err(BackendError::UnknownThread)
        ));
    }

    #[test]
    fn position_ordering() {
        assert!(pos(1, 5) < pos(1, 6));
        assert!(pos(1, 9999) < pos(2, 0));
    }

    #[test]
    fn regid_get_covers_all_variants() {
        let r = Registers {
            rax: 1, rbx: 2, rcx: 3, rdx: 4,
            rsi: 5, rdi: 6, rbp: 7, rsp: 8,
            r8: 9, r9: 10, r10: 11, r11: 12,
            r12: 13, r13: 14, r14: 15, r15: 16,
            rip: 17, rflags: 18,
        };
        let all = [
            (RegId::Rax, 1), (RegId::Rbx, 2), (RegId::Rcx, 3), (RegId::Rdx, 4),
            (RegId::Rsi, 5), (RegId::Rdi, 6), (RegId::Rbp, 7), (RegId::Rsp, 8),
            (RegId::R8, 9), (RegId::R9, 10), (RegId::R10, 11), (RegId::R11, 12),
            (RegId::R12, 13), (RegId::R13, 14), (RegId::R14, 15), (RegId::R15, 16),
            (RegId::Rip, 17), (RegId::Rflags, 18),
        ];
        for (reg, expected) in all {
            assert_eq!(r.get(reg), expected, "{reg:?}");
        }
    }
}
