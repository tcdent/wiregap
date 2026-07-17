#![allow(unsafe_code)]

//! The reversible, non-DMA PCI ops (ixy `pci.c` shape) — unbind, bus-master enable, `resource0` mmap.
//! **Every op takes `&Allowed`** and derives its target BDF from the token (`t.bdf()`), so it is
//! structurally impossible to operate on a device the gate did not allow. None of these arms a device
//! DMA — `resource0` mmap maps the device's OWN registers (MMIO), not host RAM.

use crate::gate::Allowed;
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileExt;

fn dev_path(t: &Allowed, leaf: &str) -> String {
    format!("/sys/bus/pci/devices/{}/{}", t.bdf().sysfs(), leaf)
}

/// Whether a 32-bit access at byte `offset` is wholly inside a region of `len` bytes: `offset + 4 <= len`,
/// overflow-safe AND rejecting `len < 4`. Pure — host-tested; the BAR0 register accessors assert it.
pub fn dword_in_bounds(offset: usize, len: usize) -> bool {
    offset.checked_add(4).is_some_and(|end| end <= len)
}

/// The kernel driver currently bound to the (allowed) device, if any — recorded for reversibility.
pub fn current_driver(t: &Allowed) -> Option<String> {
    fs::read_link(dev_path(t, "driver"))
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
}

/// Unbind the (allowed) device from its kernel driver. Reversible via [`rebind`].
pub fn unbind(t: &Allowed) -> io::Result<()> {
    fs::write(dev_path(t, "driver/unbind"), t.bdf().sysfs().as_bytes())
}

/// Rebind the (allowed) device to `driver` — the reversibility of [`unbind`].
pub fn rebind(t: &Allowed, driver: &str) -> io::Result<()> {
    fs::write(format!("/sys/bus/pci/drivers/{driver}/bind"), t.bdf().sysfs().as_bytes())
}

/// Enable Bus-Master (config-space COMMAND bit 2) via sysfs `config`. Read-modify-write; returns the
/// COMMAND read back. Reversible (clear bit 2).
pub fn enable_bus_master(t: &Allowed) -> io::Result<u16> {
    let f = fs::OpenOptions::new().read(true).write(true).open(dev_path(t, "config"))?;
    let mut buf = [0u8; 2];
    f.read_exact_at(&mut buf, 0x04)?;
    let command = u16::from_le_bytes(buf) | (1 << 2);
    f.write_all_at(&command.to_le_bytes(), 0x04)?;
    f.read_exact_at(&mut buf, 0x04)?;
    Ok(u16::from_le_bytes(buf))
}

/// A mmap of the device's BAR0 register window — CPU loads/stores to the mapped device registers (MMIO).
/// This is the device's OWN address space, NOT host RAM; reading/writing registers here is not a DMA.
pub struct Bar0 {
    ptr: *mut u8,
    len: usize,
}

impl Bar0 {
    /// Read a 32-bit device register at byte `offset` (an MMIO read of the BAR — not a host-RAM DMA).
    pub fn read32(&self, offset: usize) -> u32 {
        // Overflow-safe AND rejects len<4: `offset+4 <= len` with no wraparound.
        assert!(dword_in_bounds(offset, self.len), "register offset out of BAR0");
        // SAFETY: `offset+4 <= len` (checked), so `ptr+offset` is a naturally-aligned dword inside the
        // mmap'd BAR0 region; a volatile read of a device register has no host-memory effect.
        unsafe { std::ptr::read_volatile(self.ptr.add(offset) as *const u32) }
    }

    /// Write a 32-bit device register at byte `offset` (an MMIO write of the BAR — device register
    /// config, NOT a host-RAM DMA).
    pub fn write32(&self, offset: usize, value: u32) {
        assert!(dword_in_bounds(offset, self.len), "register offset out of BAR0");
        // SAFETY: `offset+4 <= len` (checked), so `ptr+offset` is a naturally-aligned dword inside the
        // mmap'd BAR0 region; a volatile write to a device register affects only the device (MMIO), not
        // host RAM.
        unsafe { std::ptr::write_volatile(self.ptr.add(offset) as *mut u32, value) }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// A raw `*mut u32` at register byte `offset` inside the BAR — for the **one** consumer that needs a
    /// stored MMIO pointer rather than a read/write call: the driver's volatile ring, whose `refill`/`post`
    /// write the RX/TX tail (`RDT`/`TDT`) doorbell directly. Bounds-checked like `read32`/`write32`. The
    /// returned pointer is valid only while this `Bar0` is mapped (its lifetime is the caller's obligation).
    pub fn reg_ptr(&self, offset: usize) -> *mut u32 {
        assert!(dword_in_bounds(offset, self.len), "register offset out of BAR0");
        // SAFETY: `offset+4 <= len` (checked) ⇒ `ptr+offset` is a naturally-aligned dword inside the
        // mmap'd BAR0. We only compute the address here; the volatile access happens at the ring's write.
        unsafe { self.ptr.add(offset) as *mut u32 }
    }
}

impl Drop for Bar0 {
    fn drop(&mut self) {
        // SAFETY: `ptr`/`len` came from a successful `mmap` in `map_resource0` and are unmapped once.
        unsafe {
            libc::munmap(self.ptr as *mut libc::c_void, self.len);
        }
    }
}

/// `mmap` the (allowed) device's BAR0 (`resource0`). Requires the device unbound from its kernel driver.
pub fn map_resource0(t: &Allowed) -> io::Result<Bar0> {
    let f = fs::OpenOptions::new().read(true).write(true).open(dev_path(t, "resource0"))?;
    let len = f.metadata()?.len() as usize;
    // SAFETY: standard file mmap — `len` is the resource size, `fd` a valid open resource0; on failure we
    // detect MAP_FAILED and return the errno. The returned pointer is owned by `Bar0` (unmapped on drop).
    let ptr = unsafe {
        libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, f.as_raw_fd(), 0)
    };
    if ptr == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    Ok(Bar0 { ptr: ptr as *mut u8, len })
}
