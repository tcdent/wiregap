#![allow(unsafe_code)]

//! ECAM (PCIe enhanced config access) for **exactly this NIC's BDF** — not a bus enumerator. The MCFG
//! (found via ACPI) gives the ECAM base; config space is then MMIO. This slice reads the
//! firmware-assigned BAR0 (read, never allocate), enables Bus-Master + Memory-Space in COMMAND, and
//! walks the capability list for MSI-X — the three things binding this 82599 needs from config space.
//!
//! The address arithmetic ([`config_address`]) is pure and host-tested; the volatile MMIO is the
//! confined `unsafe` (deny at the crate root, `allow` here), each access a single aligned read/write of
//! config space with a SAFETY note.

/// Standard PCI config-space offsets.
const OFF_VENDOR: u16 = 0x00;
const OFF_DEVICE: u16 = 0x02;
const OFF_COMMAND: u16 = 0x04;
const OFF_STATUS: u16 = 0x06;
const OFF_BAR0: u16 = 0x10;
const OFF_CAP_PTR: u16 = 0x34;

/// COMMAND bits this slice sets.
pub const CMD_MEMORY_SPACE: u16 = 1 << 1;
pub const CMD_BUS_MASTER: u16 = 1 << 2;

/// STATUS bit 4 = capability list present.
const STATUS_CAP_LIST: u16 = 1 << 4;

/// Capability IDs.
const CAP_ID_MSIX: u8 = 0x11;

/// A single PCIe function addressed through ECAM. Sized to one device; no scanning.
#[derive(Clone, Copy, Debug)]
pub struct Device {
    ecam_base: u64,
    bus: u8,
    dev: u8,
    func: u8,
}

/// The ECAM MMIO address of a config register: `base + (bus<<20 | dev<<15 | func<<12 | offset)`.
/// Pure — host-tested; the one place BDF→address is computed.
pub const fn config_address(ecam_base: u64, bus: u8, dev: u8, func: u8, offset: u16) -> u64 {
    ecam_base
        + (((bus as u64) << 20) | ((dev as u64) << 15) | ((func as u64) << 12) | ((offset as u64) & 0xFFF))
}

/// A memory BAR base, with its type observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bar {
    pub base: u64,
    pub is_64bit: bool,
    pub prefetchable: bool,
}

/// Decode a memory BAR from its low dword (and, for a 64-bit BAR, its high dword). Pure — host-tested,
/// so BOTH the 32-bit path (e1000e, the QEMU stand-in) and the 64-bit-prefetchable path (the real 82599's
/// BAR0, unreachable in QEMU) are covered now, not deferred to Slice 6. `high` is ignored for a 32-bit BAR.
pub const fn decode_bar(low: u32, high: u32) -> Bar {
    let is_mem = low & 0x1 == 0;
    let is_64bit = is_mem && (low & 0x6) == 0x4;
    let prefetchable = is_mem && (low & 0x8) != 0;
    let mut base = (low & 0xFFFF_FFF0) as u64;
    if is_64bit {
        base |= (high as u64) << 32;
    }
    Bar { base, is_64bit, prefetchable }
}

/// The MSI-X capability, as read from the cap list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MsixCap {
    /// Config offset of the capability.
    pub cap_offset: u16,
    /// Number of table entries (message-control table-size field + 1).
    pub table_size: u16,
    /// BAR index (BIR) holding the MSI-X table.
    pub table_bir: u8,
    /// Byte offset of the table within that BAR.
    pub table_offset: u32,
}

impl Device {
    pub const fn new(ecam_base: u64, bus: u8, dev: u8, func: u8) -> Self {
        Self { ecam_base, bus, dev, func }
    }

    fn addr(&self, offset: u16) -> u64 {
        config_address(self.ecam_base, self.bus, self.dev, self.func, offset)
    }

    fn read32(&self, offset: u16) -> u32 {
        // SAFETY: `addr` is a naturally-aligned dword inside this BDF's 4 KiB ECAM window (the window
        // is reported by MCFG and mapped identity by firmware). A volatile config read has no side
        // effect beyond the read and touches no Rust-owned memory.
        unsafe { core::ptr::read_volatile(self.addr(offset & !0x3) as *const u32) }
    }

    fn read16(&self, offset: u16) -> u16 {
        let dword = self.read32(offset & !0x3);
        let shift = ((offset & 0x2) * 8) as u32;
        (dword >> shift) as u16
    }

    fn write16(&self, offset: u16, value: u16) {
        // Read-modify-write the containing dword so adjacent config bytes are preserved.
        let aligned = offset & !0x3;
        let shift = ((offset & 0x2) * 8) as u32;
        let mut dword = self.read32(aligned);
        dword &= !(0xFFFFu32 << shift);
        dword |= (value as u32) << shift;
        // SAFETY: same aligned config dword as `read32`; a volatile write to config space affects only
        // the device's configuration, no Rust-owned memory.
        unsafe { core::ptr::write_volatile(self.addr(aligned) as *mut u32, dword) }
    }

    /// (vendor, device) IDs — used to confirm we are addressing the NIC we placed, not to enumerate.
    pub fn vendor_device(&self) -> (u16, u16) {
        (self.read16(OFF_VENDOR), self.read16(OFF_DEVICE))
    }

    /// Read the firmware-assigned BAR0. Does not size or reassign it (read-only). The bit-decode is
    /// [`decode_bar`] (pure); this only supplies the config dwords (and reads the high dword only for a
    /// 64-bit BAR, so a 32-bit BAR0's neighbour BAR1 is left untouched).
    pub fn bar0(&self) -> Bar {
        let low = self.read32(OFF_BAR0);
        let is_64bit = (low & 0x1 == 0) && (low & 0x6) == 0x4;
        let high = if is_64bit { self.read32(OFF_BAR0 + 4) } else { 0 };
        decode_bar(low, high)
    }

    /// Enable Memory-Space + Bus-Master in COMMAND (Bus-Master is required for the device to DMA), then
    /// read COMMAND back and return the observed value.
    pub fn enable_bus_master_and_memory(&self) -> u16 {
        let command = self.read16(OFF_COMMAND);
        self.write16(OFF_COMMAND, command | CMD_MEMORY_SPACE | CMD_BUS_MASTER);
        self.read16(OFF_COMMAND)
    }

    /// Walk the capability list for MSI-X. Bounded (≤ 48 hops) so a malformed/looping list can never
    /// spin. Returns `None` if the device has no cap list or no MSI-X capability.
    pub fn find_msix(&self) -> Option<MsixCap> {
        if self.read16(OFF_STATUS) & STATUS_CAP_LIST == 0 {
            return None;
        }
        let mut cap = self.read16(OFF_CAP_PTR) & 0xFC;
        for _ in 0..48 {
            if cap == 0 {
                return None;
            }
            let header = self.read16(cap);
            let cap_id = (header & 0xFF) as u8;
            let next = (header >> 8) & 0xFC;
            if cap_id == CAP_ID_MSIX {
                let msg_ctrl = self.read16(cap + 2);
                let table = self.read32(cap + 4);
                return Some(MsixCap {
                    cap_offset: cap,
                    table_size: (msg_ctrl & 0x7FF) + 1,
                    table_bir: (table & 0x7) as u8,
                    table_offset: table & !0x7,
                });
            }
            cap = next;
        }
        None
    }
}
