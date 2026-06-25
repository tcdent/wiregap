//! `list-entry` (0x25) — the directory-listing record grammar ([`file.md`] §3): a run of
//! `[fixed 24B metadata][unsafe name, 0x1F-terminated]` records to the frame end.
//!
//! This is a **terminator-bounded combined-record decoder** — distinct from 002's fixed-N tail
//! (records are variable-length) and from the 001 separator-stack scan. The record-internal `0x1F`
//! is a **terminator**: every name (including the last) is `0x1F`-terminated, because the next
//! record's metadata follows; the frame does **not** end an unterminated name, so a missing
//! terminator is a reject ([`Reject::UnterminatedRecord`]).
//!
//! The name's **alphabet** is validated by reusing the 001 [`scan_unsafe`] on the (`0x1F`-free) name
//! slice as a stack-of-1 — the unsafe alphabet stays single-source and `segment.rs` is byte-identical.
//! The name is the **typed** [`UnsafeSegment`] (not raw bytes), preserving the G4 split: a real
//! filename is display-only and physically cannot reach a log line.
//!
//! [`file.md`]: ../../docs/proto/file.md

use alloc::vec::Vec;

use crate::reject::Reject;
use crate::segment::{UnsafeSegment, scan_unsafe};
use crate::verb;
use crate::wire::Wire;

/// The segment terminator within a record (also the segment-stack delimiter, [`model.md`] §6).
const US: u8 = 0x1F;

/// A reserved file type — a `type_flags` low byte outside `0..=2`, carried verbatim. Inner is
/// **private**, constructed only in-crate (value `≥ 3`), so [`FileType::Other`] can never hold a
/// known type — round-trip canonical over the full `u8` (the 001/002 enforce-by-construction pattern).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReservedFileType(u8);

impl ReservedFileType {
    /// The raw reserved type byte (always `≥ 3`).
    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }
}

/// The entry's type — the low byte of `type_flags` ([`file.md`] §3). Known `0..=2`; an unrecognized
/// value is **carried raw** ([`FileType::Other`]), never rejected.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileType {
    File,
    Dir,
    Symlink,
    /// A type byte outside `0..=2` — see [`ReservedFileType`].
    Other(ReservedFileType),
}

impl FileType {
    /// Decode from `type_flags`: the **low byte** selects the type; the **high 3 bytes are dropped**
    /// (reserved-bit drop — the consumer owns any future meaning).
    fn from_type_flags(type_flags: u32) -> Self {
        match (type_flags & 0xFF) as u8 {
            0 => FileType::File,
            1 => FileType::Dir,
            2 => FileType::Symlink,
            // `other ≥ 3` is guaranteed by the arms above — the only construction site for `ReservedFileType`.
            other => FileType::Other(ReservedFileType(other)),
        }
    }

    /// Encode to `type_flags`: the low byte only; the reserved high 3 bytes are zero (canonical).
    fn to_type_flags(self) -> u32 {
        let low = match self {
            FileType::File => 0,
            FileType::Dir => 1,
            FileType::Symlink => 2,
            FileType::Other(reserved) => reserved.get(),
        };
        u32::from(low)
    }
}

/// One directory entry: fixed metadata plus the typed name.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ListEntry {
    pub size: u64,
    pub mtime_secs: u64,
    pub mtime_nanos: u32,
    pub file_type: FileType,
    /// The entry name — a **typed** [`UnsafeSegment`] (display-only, never logged; G4).
    pub name: UnsafeSegment,
}

impl ListEntry {
    /// Append this entry's wire form: `[24B metadata][name bytes][0x1F]`.
    pub(crate) fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.size.to_le_bytes());
        out.extend_from_slice(&self.mtime_secs.to_le_bytes());
        out.extend_from_slice(&self.mtime_nanos.to_le_bytes());
        out.extend_from_slice(&self.file_type.to_type_flags().to_le_bytes());
        out.extend_from_slice(self.name.as_bytes());
        out.push(US);
    }
}

/// Decode a `list-entry` body: a run of `[24B metadata][unsafe name 0x1F-terminated]` records to the
/// frame end. **Empty body → `Ok(vec![])`** (an empty directory). Total: every malformed framing
/// yields a typed [`Reject`], never a panic or over-read.
///
/// Rejects: truncated metadata (`< 24B` for a started record) → [`Reject::ShortBody`]; a name with no
/// `0x1F` terminator before the frame end → [`Reject::UnterminatedRecord`]; an empty name →
/// `EmptyStack`; a bad byte / traversal name → the 001 segment rejects.
//
// Live consumer: `ListEntries::decode` (0x25) routes through this.
pub(crate) fn decode_list_entries(tail: &[u8]) -> Result<Vec<ListEntry>, Reject> {
    let mut entries = Vec::new();
    let mut rest = tail;
    while !rest.is_empty() {
        // The fixed 24B metadata, read field-by-field (bounded; truncated → ShortBody).
        let (size, r) = rest
            .split_first_chunk::<8>()
            .ok_or(Reject::ShortBody { needed: 24, got: rest.len() })?;
        let (mtime_secs, r) = r
            .split_first_chunk::<8>()
            .ok_or(Reject::ShortBody { needed: 24, got: rest.len() })?;
        let (mtime_nanos, r) = r
            .split_first_chunk::<4>()
            .ok_or(Reject::ShortBody { needed: 24, got: rest.len() })?;
        let (type_flags, after) = r
            .split_first_chunk::<4>()
            .ok_or(Reject::ShortBody { needed: 24, got: rest.len() })?;

        // The FIRST 0x1F after the metadata terminates the name (a valid name has no 0x1F).
        let name_offset = tail.len() - after.len();
        let term = after
            .iter()
            .position(|&b| b == US)
            .ok_or(Reject::UnterminatedRecord { offset: name_offset })?;
        let (name_bytes, term_rest) = after
            .split_at_checked(term)
            .ok_or(Reject::UnterminatedRecord { offset: name_offset })?;

        // 001 single-source alphabet validation on the 0x1F-free slice (stack-of-1): an empty name →
        // EmptyStack, a `/`/NUL → BadSegmentByte, a `.`/`..` → TraversalSegment all propagate here.
        let name = match scan_unsafe(name_bytes)?.segments() {
            [seg] => seg.clone(),
            // INTENTIONALLY UNREACHABLE — the one structurally-uncoverable arm (009 coverage closeout,
            // operator-ratified keep+document). `name_bytes` is the 0x1F-free slice taken before the
            // terminator, so `scan_unsafe` yields exactly one segment; the multi-segment case cannot
            // arise from any input. Kept as the 3A defense-in-depth guard — a *defensive typed reject*,
            // never `unreachable!()`/panic (G2 totality). Excluded from the reachable-100% coverage
            // target: hitting it would require an impossible input.
            _ => return Err(Reject::UnterminatedRecord { offset: name_offset }),
        };

        // Consume the 0x1F terminator (`term_rest` starts with it; `get(1..)` is bounded, no panic).
        rest = term_rest.get(1..).unwrap_or(&[]);

        entries.push(ListEntry {
            size: u64::from_le_bytes(*size),
            mtime_secs: u64::from_le_bytes(*mtime_secs),
            mtime_nanos: u32::from_le_bytes(*mtime_nanos),
            file_type: FileType::from_type_flags(u32::from_le_bytes(*type_flags)),
            name,
        });
    }
    Ok(entries)
}

/// `list-entry` (0x25): the directory-listing response — a run of [`ListEntry`] records. Decode routes
/// through [`decode_list_entries`]; encode appends each record via [`ListEntry::encode_into`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ListEntries(pub Vec<ListEntry>);

impl Wire for ListEntries {
    const VERB: u32 = verb::LIST_ENTRY;

    fn encode(&self, out: &mut Vec<u8>) {
        for entry in &self.0 {
            entry.encode_into(out);
        }
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        decode_list_entries(body).map(ListEntries)
    }
}

#[cfg(test)]
mod tests {
    use super::{FileType, ReservedFileType, decode_list_entries};
    use crate::reject::Reject;
    use alloc::vec;
    use alloc::vec::Vec;

    /// Build a record: 24B metadata (size/mtime_secs/mtime_nanos/type_flags LE) + name + 0x1F.
    fn record(size: u64, type_flags: u32, name: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&size.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes()); // mtime_secs
        b.extend_from_slice(&0u32.to_le_bytes()); // mtime_nanos
        b.extend_from_slice(&type_flags.to_le_bytes());
        b.extend_from_slice(name);
        b.push(0x1F);
        b
    }

    #[test]
    fn empty_body_is_empty_directory() {
        assert_eq!(decode_list_entries(&[]), Ok(Vec::new()));
    }

    #[test]
    fn one_and_n_records() {
        let one = decode_list_entries(&record(42, 0, b"file.txt")).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].size, 42);
        assert_eq!(one[0].file_type, FileType::File);
        assert_eq!(one[0].name.as_bytes(), b"file.txt");

        let mut body = record(1, 1, b"sub dir");
        body.extend(record(2, 2, b"link"));
        body.extend(record(3, 0, b"a.b"));
        let n = decode_list_entries(&body).unwrap();
        assert_eq!(n.len(), 3);
        assert_eq!(n[0].file_type, FileType::Dir);
        assert_eq!(n[1].file_type, FileType::Symlink);
        assert_eq!(n[2].name.as_bytes(), b"a.b");
    }

    #[test]
    fn truncated_metadata_is_short_body() {
        // a started record with < 24 metadata bytes
        let buf = vec![0u8; 20];
        assert_eq!(
            decode_list_entries(&buf),
            Err(Reject::ShortBody { needed: 24, got: 20 })
        );
    }

    #[test]
    fn unterminated_name_is_unterminated_record() {
        // valid 24B metadata + name bytes but NO 0x1F before frame end
        let mut buf = Vec::new();
        buf.extend_from_slice(&[0u8; 24]);
        buf.extend_from_slice(b"noterminator");
        assert_eq!(
            decode_list_entries(&buf),
            Err(Reject::UnterminatedRecord { offset: 24 })
        );
    }

    #[test]
    fn empty_name_is_empty_stack() {
        // metadata then an immediate 0x1F (empty name)
        assert_eq!(
            decode_list_entries(&record(0, 0, b"")),
            Err(Reject::EmptyStack)
        );
    }

    #[test]
    fn bad_byte_and_traversal_names_reject_via_001_scan() {
        assert_eq!(
            decode_list_entries(&record(0, 0, b"a/b")),
            Err(Reject::BadSegmentByte { index: 0, offset: 1, byte: b'/' })
        );
        assert_eq!(
            decode_list_entries(&record(0, 0, b"..")),
            Err(Reject::TraversalSegment { index: 0 })
        );
    }

    #[test]
    fn file_type_roundtrips_over_full_u8_and_drops_high_bytes() {
        for b in 0u8..=255 {
            let ft = FileType::from_type_flags(u32::from(b));
            // to_type_flags is the low byte only
            assert_eq!(ft.to_type_flags(), u32::from(b));
            // and it round-trips
            assert_eq!(FileType::from_type_flags(ft.to_type_flags()), ft);
            // known vs reserved
            match b {
                0 => assert_eq!(ft, FileType::File),
                1 => assert_eq!(ft, FileType::Dir),
                2 => assert_eq!(ft, FileType::Symlink),
                n => assert_eq!(ft, FileType::Other(ReservedFileType(n))),
            }
        }
        // high 3 bytes are dropped on decode and emitted zero
        let ft = FileType::from_type_flags(0xABCD_EF02);
        assert_eq!(ft, FileType::Symlink);
        assert_eq!(ft.to_type_flags(), 2);
    }
}
