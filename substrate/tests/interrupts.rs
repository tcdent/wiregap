//! Host `cargo test` of the Slice-3 pure logic (harness surface (a)): the interrupt vector policy. The
//! IDT/LAPIC/timer machinery is bare-metal and exercised in QEMU; the *policy* (vectors avoid the
//! exception range and don't collide) is arithmetic and proven here.

use substrate::interrupts::VectorPlan;

#[test]
fn nic_plan_is_valid() {
    assert!(VectorPlan::NIC.is_valid());
    // The concrete assignment, pinned.
    assert_eq!(VectorPlan::NIC.nic_msix, 0x40);
    assert_eq!(VectorPlan::NIC.timer, 0x41);
    assert_eq!(VectorPlan::NIC.spurious, 0xFF);
}

#[test]
fn a_vector_in_the_exception_range_is_invalid() {
    // 0..31 are CPU exceptions; a data vector there is a bug.
    let bad = VectorPlan { nic_msix: 0x10, timer: 0x41, spurious: 0xFF };
    assert!(!bad.is_valid(), "vector < 32 must be rejected");
}

#[test]
fn colliding_vectors_are_invalid() {
    let msix_eq_timer = VectorPlan { nic_msix: 0x40, timer: 0x40, spurious: 0xFF };
    assert!(!msix_eq_timer.is_valid(), "msix == timer must be rejected");
    let timer_eq_spurious = VectorPlan { nic_msix: 0x40, timer: 0xFF, spurious: 0xFF };
    assert!(!timer_eq_spurious.is_valid(), "timer == spurious must be rejected");
}
