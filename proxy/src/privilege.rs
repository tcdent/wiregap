//! Sprint 8 — the privilege-drop primitive (`architecture.md` §10, L15): spend privilege at bring-up,
//! drop it **before any box byte is processed**. The box-facing relay then runs with no privilege to
//! spend — least privilege by construction, a structural floor, not a runtime discipline.
//!
//! This is the crate's **second** confined-unsafe module (after [`crate::open_discipline`]). The
//! standing house rule holds: box-reachable code carries **confined in-house `unsafe`**, never a
//! third-party wrapper — reviewability is the guarantee, not a zero-unsafe-token count. The crate root
//! is `deny(unsafe_code)`; this module re-`allow`s it, confined and SAFETY-noted.
//!
//! **Certification is a live privileged probe (capture-model), not a unit test.** A dropped process
//! cannot be produced in an unprivileged test, and a passing compile proves nothing about a drop that
//! never ran. Per IB2, the post-drop state ([`observe`]) and the negative test (a privileged op fails
//! from the dropped process) must be **observed** on a real privileged→dropped run — the same
//! live-probe discipline `open_discipline` carries.

#![allow(unsafe_code)]

use std::io;

// The `capset`/`capget` ABI structs are not exposed by `libc` 0.2, so they are declared here — stable
// Linux kernel ABI (`<linux/capability.h>`), a few certifiable lines in-house rather than pulling a
// capability wrapper crate onto the trusted surface (the box-reachable-core house rule).

/// `<linux/capability.h>` `_LINUX_CAPABILITY_VERSION_3` — the 64-bit (2×u32) capability ABI.
const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;

/// `cap_user_header_t` (`struct __user_cap_header_struct`): the ABI version + target pid (0 = self).
#[repr(C)]
struct CapHeader {
    version: u32,
    pid: libc::c_int,
}

/// One `cap_user_data_t` word (`struct __user_cap_data_struct`); the v3 ABI passes an array of **two**
/// (low then high 32 bits of each 64-bit set).
#[repr(C)]
#[derive(Clone, Copy)]
struct CapData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

/// Map a raw syscall return to `io::Result`: `-1` is failure carrying `errno`. **Every**
/// privilege-affecting syscall is routed through this — an unchecked failure that silently leaves the
/// process privileged is the classic bug IB2 guards against.
fn checked(ret: libc::c_int, step: &str) -> io::Result<()> {
    if ret == -1 {
        let e = io::Error::last_os_error();
        Err(io::Error::new(
            e.kind(),
            format!("privilege-drop step `{step}` failed (errno {:?}): {e}", e.raw_os_error()),
        ))
    } else {
        Ok(())
    }
}

/// Drop **all** privilege and pin it dropped: the capability ambient + bounding sets, supplementary
/// groups, gid, uid, and the effective/permitted/inheritable capability sets, then `no-new-privs`.
///
/// **Order is load-bearing** and matches the well-established secure sequence:
/// 1. `PR_SET_KEEPCAPS = 0` — do not retain capabilities across the uid change (the default, made
///    explicit so the intent is auditable).
/// 2. Clear the **ambient** capability set (`PR_CAP_AMBIENT_CLEAR_ALL`) — else ambient caps survive
///    the uid drop and re-enter the permitted set on exec.
/// 3. Drop the entire capability **bounding** set — so no capability can be re-acquired via a
///    subsequent `execve` of a file-capability or setuid binary.
/// 4. `setgroups([])` — drop supplementary groups **while still privileged** (needs `CAP_SETGID`,
///    which the uid drop removes).
/// 5. `setresgid(gid,gid,gid)` — **gid before uid**; dropping the uid first would lose `CAP_SETGID`
///    and strand a privileged gid.
/// 6. `setresuid(uid,uid,uid)` — **all three** (real/effective/**saved**); a privileged saved-uid is
///    re-acquirable, so it is not a drop. After this the process is unprivileged.
/// 7. Clear the effective/permitted/inheritable capability sets (`capset`, a reduction — always
///    permitted).
/// 8. `PR_SET_NO_NEW_PRIVS = 1` — bar any future privilege gain via `execve` (also neuters setuid
///    binaries), the belt on top of the empty bounding set.
///
/// `uid`/`gid` are the **target unprivileged identity** to drop to — supplied by the caller (the
/// composition root), not chosen here.
pub fn drop_to(uid: u32, gid: u32) -> io::Result<()> {
    // 1. Do not keep caps across the uid change.
    // SAFETY: prctl with a constant option and integer arg; no memory is touched.
    checked(unsafe { libc::prctl(libc::PR_SET_KEEPCAPS, 0, 0, 0, 0) }, "PR_SET_KEEPCAPS")?;

    // 2. Clear the ambient capability set.
    // SAFETY: prctl PR_CAP_AMBIENT with the CLEAR_ALL sub-op takes only integer args.
    checked(
        unsafe { libc::prctl(libc::PR_CAP_AMBIENT, libc::PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0) },
        "PR_CAP_AMBIENT_CLEAR_ALL",
    )?;

    // 3. Drop the whole bounding set.
    drop_bounding_set()?;

    // 4. Supplementary groups first (needs CAP_SETGID, lost after the uid drop).
    // SAFETY: setgroups(0, NULL) is the documented "drop all supplementary groups" call.
    checked(unsafe { libc::setgroups(0, core::ptr::null()) }, "setgroups")?;

    // 5. gid (all three), before uid.
    // SAFETY: setresgid with three plain gids; no memory is touched.
    checked(unsafe { libc::setresgid(gid, gid, gid) }, "setresgid")?;

    // 6. uid (all three) — after this, unprivileged.
    // SAFETY: setresuid with three plain uids; no memory is touched.
    checked(unsafe { libc::setresuid(uid, uid, uid) }, "setresuid")?;

    // 7. Clear effective/permitted/inheritable capability sets.
    clear_capability_sets()?;

    // 8. No new privileges via execve.
    // SAFETY: prctl PR_SET_NO_NEW_PRIVS with the required-1 arg; no memory is touched.
    checked(unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) }, "PR_SET_NO_NEW_PRIVS")?;

    Ok(())
}

/// Drop every capability from the bounding set. Walks caps upward, using `PR_CAPBSET_READ` to find the
/// kernel's highest valid cap number (`-1`/`EINVAL` past the end), and drops each present one. Needs
/// `CAP_SETPCAP`, so it runs **before** the uid drop.
fn drop_bounding_set() -> io::Result<()> {
    let mut cap: libc::c_ulong = 0;
    loop {
        // SAFETY: prctl PR_CAPBSET_READ with an integer cap number; returns 1/0, or -1 past the max.
        let present = unsafe { libc::prctl(libc::PR_CAPBSET_READ, cap, 0, 0, 0) };
        if present == -1 {
            break; // walked past the last cap the kernel defines
        }
        if present == 1 {
            // SAFETY: prctl PR_CAPBSET_DROP with an integer cap number.
            checked(unsafe { libc::prctl(libc::PR_CAPBSET_DROP, cap, 0, 0, 0) }, "PR_CAPBSET_DROP")?;
        }
        cap += 1;
    }
    Ok(())
}

/// Clear the effective/permitted/inheritable capability sets to empty via `capset` (v3).
fn clear_capability_sets() -> io::Result<()> {
    let mut header = CapHeader {
        version: LINUX_CAPABILITY_VERSION_3,
        pid: 0, // the calling thread
    };
    // Two 32-bit words for the 64-bit v3 sets; all zero = drop everything (a reduction, permitted).
    let data = [
        CapData { effective: 0, permitted: 0, inheritable: 0 },
        CapData { effective: 0, permitted: 0, inheritable: 0 },
    ];
    // SAFETY: `header` is a valid v3 cap header; `data` is a 2-element v3 cap-data array (the size
    // v3 requires); capset reads both and clears every capability.
    let r = unsafe {
        libc::syscall(libc::SYS_capset, &mut header as *mut _, data.as_ptr())
    };
    checked(r as libc::c_int, "capset")
}

/// The observed OS credential state — the IB2 evidence read back **after** a drop. All uid/gid triples
/// at the unprivileged target and all capability sets empty is the only shape that proves a real drop
/// (a privileged saved-uid or a non-empty bounding set is re-acquirable, i.e. not dropped).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Credentials {
    /// real / effective / saved uid.
    pub uid: (u32, u32, u32),
    /// real / effective / saved gid.
    pub gid: (u32, u32, u32),
    /// The effective capability set as a 64-bit mask (0 = empty).
    pub cap_effective: u64,
    /// The permitted capability set as a 64-bit mask (0 = empty).
    pub cap_permitted: u64,
    /// The inheritable capability set as a 64-bit mask (0 = empty).
    pub cap_inheritable: u64,
    /// Whether the bounding set holds no capability.
    pub bounding_empty: bool,
    /// Whether the ambient set holds no capability.
    pub ambient_empty: bool,
    /// Whether `no_new_privs` is set (the `execve` privilege-gain guard `drop_to` sets in step 8).
    /// Reading it back confirms the irreversibility actually *stuck*, not just that the set-call
    /// returned — a future credential capture then evidences the whole drop, execve-guard included.
    pub no_new_privs: bool,
}

impl Credentials {
    /// The IB2 predicate: dropped to `(uid, gid)` with every capability set empty, both the bounding
    /// and ambient sets empty, and `no_new_privs` set. Anything else is not a full drop.
    #[must_use]
    pub fn is_fully_dropped_to(&self, uid: u32, gid: u32) -> bool {
        self.uid == (uid, uid, uid)
            && self.gid == (gid, gid, gid)
            && self.cap_effective == 0
            && self.cap_permitted == 0
            && self.cap_inheritable == 0
            && self.bounding_empty
            && self.ambient_empty
            && self.no_new_privs
    }
}

/// Read back the process credential state (uid/gid triples + all capability sets) for the IB2
/// observation. Read-only — it changes nothing.
#[must_use]
pub fn observe() -> Credentials {
    let (mut ru, mut eu, mut su) = (0u32, 0u32, 0u32);
    let (mut rg, mut eg, mut sg) = (0u32, 0u32, 0u32);
    // SAFETY: each pointer is a valid, writable local; getresuid/getresgid fill the three ids.
    unsafe {
        libc::getresuid(&mut ru, &mut eu, &mut su);
        libc::getresgid(&mut rg, &mut eg, &mut sg);
    }

    let mut header = CapHeader {
        version: LINUX_CAPABILITY_VERSION_3,
        pid: 0,
    };
    let mut data = [
        CapData { effective: 0, permitted: 0, inheritable: 0 },
        CapData { effective: 0, permitted: 0, inheritable: 0 },
    ];
    // SAFETY: v3 header + a 2-element v3 data array; capget fills `data` with the current cap sets.
    unsafe {
        libc::syscall(libc::SYS_capget, &mut header as *mut _, data.as_mut_ptr());
    }
    let join = |lo: u32, hi: u32| (lo as u64) | ((hi as u64) << 32);

    // SAFETY: prctl PR_GET_NO_NEW_PRIVS takes only integer args (2–5 must be 0) and returns the
    // current flag (1 set / 0 unset) as its value — no memory is touched.
    let no_new_privs = unsafe { libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) } == 1;

    Credentials {
        uid: (ru, eu, su),
        gid: (rg, eg, sg),
        cap_effective: join(data[0].effective, data[1].effective),
        cap_permitted: join(data[0].permitted, data[1].permitted),
        cap_inheritable: join(data[0].inheritable, data[1].inheritable),
        bounding_empty: bounding_set_empty(),
        ambient_empty: ambient_set_empty(),
        no_new_privs,
    }
}

/// True iff no capability remains in the bounding set (every `PR_CAPBSET_READ` up to the kernel's max
/// returns "not present").
fn bounding_set_empty() -> bool {
    let mut cap: libc::c_ulong = 0;
    loop {
        // SAFETY: prctl PR_CAPBSET_READ with an integer cap number.
        let present = unsafe { libc::prctl(libc::PR_CAPBSET_READ, cap, 0, 0, 0) };
        if present == -1 {
            return true; // walked every defined cap; none was present
        }
        if present == 1 {
            return false;
        }
        cap += 1;
    }
}

/// True iff no capability remains in the ambient set.
fn ambient_set_empty() -> bool {
    let mut cap: libc::c_ulong = 0;
    loop {
        // SAFETY: prctl PR_CAP_AMBIENT with the IS_SET sub-op and an integer cap number.
        let set = unsafe { libc::prctl(libc::PR_CAP_AMBIENT, libc::PR_CAP_AMBIENT_IS_SET, cap, 0, 0) };
        if set == -1 {
            return true; // cap number past the max → done
        }
        if set == 1 {
            return false;
        }
        cap += 1;
    }
}
