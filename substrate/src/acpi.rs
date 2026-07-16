#![allow(unsafe_code)]

//! Minimal ACPI walk — RSDP → XSDT/RSDT → **MCFG** → the ECAM base. Only what Slice 2 consumes: the
//! MCFG so ECAM can reach this NIC's config space. (Sprint 5 walks the same way for the DMAR.) The
//! tables are firmware-provided physical memory, identity-mapped and valid after `ExitBootServices`, so
//! access is raw reads — the confined `unsafe` (deny at the crate root, `allow` here), each read a
//! bounded, `read_unaligned` field access with a SAFETY note. No allocation.

/// The ACPI root pointer, from the UEFI config table (kept live since Slice 1).
#[derive(Clone, Copy)]
pub struct Acpi {
    rsdp: u64,
}

/// A parsed MCFG ECAM allocation for one PCI segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EcamAlloc {
    pub base: u64,
    pub segment: u16,
    pub start_bus: u8,
    pub end_bus: u8,
}

// SAFETY (shared by the readers below): each `addr` is a field inside a firmware ACPI table located by
// following the RSDP/XSDT chain. Those tables live in ACPI memory that persists past ExitBootServices
// and is identity-mapped, so the physical address is a valid readable pointer. `read_unaligned` tolerates
// the tables' packed layout. Reads have no side effects and touch no Rust-owned memory.
fn read_u16(addr: u64) -> u16 {
    unsafe { core::ptr::read_unaligned(addr as *const u16) }
}
fn read_u32(addr: u64) -> u32 {
    unsafe { core::ptr::read_unaligned(addr as *const u32) }
}
fn read_u64(addr: u64) -> u64 {
    unsafe { core::ptr::read_unaligned(addr as *const u64) }
}
fn read_sig(addr: u64) -> [u8; 4] {
    unsafe { core::ptr::read_unaligned(addr as *const [u8; 4]) }
}

/// System Description Table header size (bytes) — signature..creator_revision.
const SDT_HEADER_LEN: u64 = 36;

impl Acpi {
    pub const fn new(rsdp: u64) -> Self {
        Self { rsdp }
    }

    /// RSDP revision (byte 15): 0 = ACPI 1.0 (RSDT), ≥2 = ACPI 2.0+ (XSDT).
    fn revision(&self) -> u8 {
        // SAFETY: as above — the RSDP itself, a valid firmware structure.
        unsafe { core::ptr::read_unaligned((self.rsdp + 15) as *const u8) }
    }

    /// The address of the root system table: XSDT (ACPI 2.0+) or RSDT (1.0).
    fn root_table(&self) -> (u64, bool) {
        if self.revision() >= 2 {
            (read_u64(self.rsdp + 24), true) // XSDT, 64-bit entries
        } else {
            (read_u32(self.rsdp + 16) as u64, false) // RSDT, 32-bit entries
        }
    }

    /// Find a table by signature by scanning the root table's entry list. Returns the table's address.
    fn find_table(&self, signature: &[u8; 4]) -> Option<u64> {
        let (root, is_xsdt) = self.root_table();
        if root == 0 {
            return None;
        }
        let total_len = read_u32(root + 4) as u64;
        if total_len < SDT_HEADER_LEN {
            return None;
        }
        let entry_size: u64 = if is_xsdt { 8 } else { 4 };
        let count = (total_len - SDT_HEADER_LEN) / entry_size;
        for i in 0..count {
            let entry_ptr = root + SDT_HEADER_LEN + i * entry_size;
            let table = if is_xsdt { read_u64(entry_ptr) } else { read_u32(entry_ptr) as u64 };
            if table != 0 && read_sig(table) == *signature {
                return Some(table);
            }
        }
        None
    }

    /// The ECAM allocation for `segment` from the MCFG table (`base + reserved(8) + entries[16]`).
    pub fn ecam_alloc(&self, segment: u16) -> Option<EcamAlloc> {
        let mcfg = self.find_table(b"MCFG")?;
        let total_len = read_u32(mcfg + 4) as u64;
        // Entries start after the 36-byte header + 8 reserved bytes; each entry is 16 bytes.
        let first = mcfg + SDT_HEADER_LEN + 8;
        if total_len < first - mcfg {
            return None;
        }
        let count = (total_len - (first - mcfg)) / 16;
        for i in 0..count {
            let e = first + i * 16;
            let seg = (read_u32(e + 8) & 0xFFFF) as u16; // segment is the u16 right after base(u64)
            if seg == segment {
                return Some(EcamAlloc {
                    base: read_u64(e),
                    segment: seg,
                    start_bus: (read_u32(e + 8) >> 16) as u8,
                    end_bus: (read_u32(e + 12) & 0xFF) as u8,
                });
            }
        }
        None
    }

    /// The first DRHD's IOMMU register base for `segment`, from the DMAR table — the VT-d analog of
    /// `ecam_alloc` (find the table, scan the remapping structures). `None` if there is no IOMMU.
    /// DMAR: 36-byte SDT header, then host-addr-width(1) + flags(1) + reserved(10); remapping structures
    /// start at offset 48. Each structure: type(u16) + length(u16); DRHD (type 0) holds segment(u16 @ +6)
    /// + register-base(u64 @ +8).
    pub fn dmar_drhd_reg_base(&self, segment: u16) -> Option<u64> {
        let dmar = self.find_table(b"DMAR")?;
        let total_len = read_u32(dmar + 4) as u64;
        let mut off = 48u64;
        while off + 4 <= total_len {
            let stype = read_u16(dmar + off);
            let slen = read_u16(dmar + off + 2) as u64;
            if slen == 0 {
                break; // malformed — stop rather than loop
            }
            if stype == 0 && read_u16(dmar + off + 6) == segment {
                return Some(read_u64(dmar + off + 8));
            }
            off += slen;
        }
        None
    }
}
