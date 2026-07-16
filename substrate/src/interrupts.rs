//! This NIC's interrupt plumbing — a **minimal** IDT with exactly the vectors this NIC needs (its
//! MSI-X vector + the LAPIC-timer vector), plus a double-fault catch so a stray fault reports instead of
//! triple-faulting silently. Not a general interrupt framework.
//!
//! The vector *policy* ([`VectorPlan`]) is pure and host-tested. The IDT + the interrupt entry stubs are
//! the bare-metal part. Handlers are entered through hand-written **naked** stubs (stable Rust) that save
//! the caller-saved registers, call a normal `extern "C"` handler, restore, and `iretq` — this avoids the
//! nightly-only `x86-interrupt` ABI, keeping the crate on the project's stable toolchain. The `unsafe`
//! (the asm stubs + loading the IDT) is confined here with SAFETY notes.

/// The vectors this NIC uses. Vectors 0..31 are CPU exceptions, so all must be ≥ 32 and distinct.
#[derive(Clone, Copy, Debug)]
pub struct VectorPlan {
    /// The NIC's MSI-X interrupt (its RX/TX-completion wakeup; a self-IPI stands in this slice).
    pub nic_msix: u8,
    /// The LAPIC one-shot timer (retransmit deadlines).
    pub timer: u8,
    /// The APIC spurious vector.
    pub spurious: u8,
}

impl VectorPlan {
    /// This NIC's vectors. Kept away from the exception range and from each other.
    pub const NIC: VectorPlan = VectorPlan { nic_msix: 0x40, timer: 0x41, spurious: 0xFF };

    /// Valid iff every vector avoids the CPU-exception range (≥ 32) and no two collide.
    pub const fn is_valid(&self) -> bool {
        self.nic_msix >= 32
            && self.timer >= 32
            && self.spurious >= 32
            && self.nic_msix != self.timer
            && self.nic_msix != self.spurious
            && self.timer != self.spurious
    }
}

#[cfg(target_os = "uefi")]
pub use imp::{init, MSIX_HITS, MSIX_PENDING, TIMER_TICKS};

#[cfg(target_os = "uefi")]
mod imp {
    #![allow(unsafe_code)]

    use super::VectorPlan;
    use crate::lapic;
    use core::arch::naked_asm;
    use core::cell::UnsafeCell;
    use core::fmt::Write;
    use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use x86_64::structures::idt::InterruptDescriptorTable;
    use x86_64::VirtAddr;

    /// Times the LAPIC timer fired (gate-3 observable: incremented by the timer handler).
    pub static TIMER_TICKS: AtomicU32 = AtomicU32::new(0);
    /// Times the NIC-MSI-X vector was taken (via the self-IPI stand-in).
    pub static MSIX_HITS: AtomicU32 = AtomicU32::new(0);
    /// Lost-wakeup latch: set by the MSI-X handler, consumed (swapped false) by the reactor's `block`
    /// so a wakeup arriving just before a park is not lost (the bare-metal analog of the eventfd latch).
    pub static MSIX_PENDING: AtomicBool = AtomicBool::new(false);

    // ---- normal `extern "C"` handlers (reached from the naked stubs) --------------------------

    extern "C" fn on_timer() {
        TIMER_TICKS.fetch_add(1, Ordering::SeqCst);
        lapic::eoi();
    }

    extern "C" fn on_msix() {
        MSIX_HITS.fetch_add(1, Ordering::SeqCst);
        MSIX_PENDING.store(true, Ordering::SeqCst);
        lapic::eoi();
    }

    extern "C" fn on_double_fault() -> ! {
        let mut com = crate::serial::Serial::com1();
        let _ = writeln!(com, "EXCEPTION : double fault (halting)");
        loop {
            x86_64::instructions::hlt();
        }
    }

    // ---- naked interrupt stubs ----------------------------------------------------------------
    //
    // On interrupt entry the CPU has pushed the interrupt frame (SS,RSP,RFLAGS,CS,RIP) 16-byte aligned;
    // the 9 caller-saved pushes below then leave RSP 16-aligned for the `call`. The `extern "C"` handler
    // preserves RBX/RBP/R12–R15 for us; we save/restore the caller-saved set around it, then `iretq`.

    macro_rules! interrupt_stub {
        ($stub:ident => $handler:ident) => {
            #[unsafe(naked)]
            extern "C" fn $stub() {
                naked_asm!(
                    "push rax", "push rcx", "push rdx", "push rsi", "push rdi",
                    "push r8", "push r9", "push r10", "push r11",
                    "call {h}",
                    "pop r11", "pop r10", "pop r9", "pop r8",
                    "pop rdi", "pop rsi", "pop rdx", "pop rcx", "pop rax",
                    "iretq",
                    h = sym $handler,
                );
            }
        };
    }

    interrupt_stub!(timer_stub => on_timer);
    interrupt_stub!(msix_stub => on_msix);

    // The double-fault stub diverges (never returns), so it needs no register save/restore.
    #[unsafe(naked)]
    extern "C" fn double_fault_stub() {
        naked_asm!("call {h}", h = sym on_double_fault);
    }

    // ---- the IDT ------------------------------------------------------------------------------

    struct IdtCell(UnsafeCell<InterruptDescriptorTable>);
    // SAFETY: the IDT is written once during single-threaded `init` before interrupts are enabled, then
    // only read by the CPU. No concurrent Rust access.
    unsafe impl Sync for IdtCell {}

    static IDT: IdtCell = IdtCell(UnsafeCell::new(InterruptDescriptorTable::new()));

    /// Install the minimal IDT (this NIC's two vectors + the double-fault catch) and load it.
    pub fn init(plan: &VectorPlan) {
        // SAFETY: single-threaded, pre-`sti`; the stub symbols are valid interrupt entry points, and
        // `set_handler_addr` fills the code selector from the current (long-mode) CS. We take one
        // `&mut` to the static IDT here and do not alias it.
        unsafe {
            let idt = &mut *IDT.0.get();
            idt[plan.timer].set_handler_addr(VirtAddr::new(timer_stub as *const () as u64));
            idt[plan.nic_msix].set_handler_addr(VirtAddr::new(msix_stub as *const () as u64));
            idt.double_fault.set_handler_addr(VirtAddr::new(double_fault_stub as *const () as u64));
        }
        // SAFETY: the IDT lives in a `'static`, so the CPU may reference it for the program's lifetime.
        unsafe { (*IDT.0.get()).load() };
    }
}
