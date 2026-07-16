#![allow(unsafe_code)]

//! COM1 (`0x3F8`) 16550 UART — the post-handoff console. After `ExitBootServices` the UEFI text
//! services are gone, so output is raw port I/O to the UART. That port I/O is the crate's only Slice-1
//! `unsafe`, confined here; each block is a single `in`/`out` to a fixed, well-known ISA port with a
//! SAFETY note. This is boot/console boilerplate, **not** on the NIC data path and not security-bearing.

use core::fmt;
use x86_64::instructions::port::Port;

/// COM1 base I/O port. The register offsets below are the standard 16550 layout.
const COM1_BASE: u16 = 0x3F8;

/// A minimal write-only COM1 console: enough to emit the banner + assertions, nothing more.
pub struct Serial {
    data: Port<u8>,        // +0 : transmit/receive buffer (DLAB=0)
    int_enable: Port<u8>,  // +1 : interrupt enable      (DLAB=0) / divisor high (DLAB=1)
    fifo_ctrl: Port<u8>,   // +2 : FIFO control
    line_ctrl: Port<u8>,   // +3 : line control (DLAB bit)
    modem_ctrl: Port<u8>,  // +4 : modem control
    line_status: Port<u8>, // +5 : line status (bit 5 = transmit-holding-register empty)
}

impl Serial {
    /// Initialise COM1 for 115200 8N1, FIFO on. Idempotent — safe to call again (e.g. from the panic
    /// handler); it just reprograms the fixed UART.
    pub fn com1() -> Self {
        let mut s = Self {
            data: Port::new(COM1_BASE),
            int_enable: Port::new(COM1_BASE + 1),
            fifo_ctrl: Port::new(COM1_BASE + 2),
            line_ctrl: Port::new(COM1_BASE + 3),
            modem_ctrl: Port::new(COM1_BASE + 4),
            line_status: Port::new(COM1_BASE + 5),
        };
        // SAFETY: COM1 is a fixed ISA 16550 UART at 0x3F8..=0x3FF. These writes program its own
        // configuration registers (baud divisor, line/FIFO/modem control) — no memory is aliased and
        // port writes to a UART have no undefined behaviour. The sequence is the standard 16550 init.
        unsafe {
            s.int_enable.write(0x00); // disable UART interrupts (we poll)
            s.line_ctrl.write(0x80); // DLAB=1 to set the baud divisor
            s.data.write(0x01); // divisor low  = 1  -> 115200 baud
            s.int_enable.write(0x00); // divisor high = 0
            s.line_ctrl.write(0x03); // DLAB=0, 8 bits, no parity, 1 stop
            s.fifo_ctrl.write(0xC7); // enable FIFO, clear rx/tx, 14-byte threshold
            s.modem_ctrl.write(0x0B); // DTR + RTS + OUT2
        }
        s
    }

    fn write_byte(&mut self, byte: u8) {
        // SAFETY: reads the UART line-status register and writes the transmit register — its own
        // fixed ports. We spin until bit 5 (transmit-holding-register empty) before writing, so no
        // byte is dropped; no memory is touched.
        unsafe {
            while self.line_status.read() & 0x20 == 0 {
                core::hint::spin_loop();
            }
            self.data.write(byte);
        }
    }
}

impl fmt::Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &byte in s.as_bytes() {
            // Translate LF to CRLF so the log is clean on a raw terminal.
            if byte == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(byte);
        }
        Ok(())
    }
}
