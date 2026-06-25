//! The file service ([`file.md`]): `file-chunk` (verbatim tail), `file-read` (unsafe stack), and
//! `file-error` (shared error body) are 001 seeds; `file-write` (safe stack), `file-data` (verbatim),
//! `list` and `request-file` (unsafe stack) reuse the 001 scans — new message types only, never a
//! re-implemented decoder. (`list-entry`'s record grammar lives in [`crate::list_entry`].)
//!
//! [`file.md`]: ../../docs/proto/file.md

use alloc::vec::Vec;

use crate::errors::ErrorBody;
use crate::reject::Reject;
use crate::segment::{SafeSegmentStack, UnsafeSegmentStack, scan_safe, scan_unsafe};
use crate::verb;
use crate::wire::{Wire, verbatim_verb};

/// `file-chunk` (`box → proxy`): verbatim bytes, appended in order to the quarantine file. Pure
/// verbatim tail — the whole body is the content, no fixed prefix, no boundary to police (a chunk
/// may be empty).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileChunk {
    pub bytes: Vec<u8>,
}

impl Wire for FileChunk {
    const VERB: u32 = verb::FILE_CHUNK;

    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.bytes);
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        Ok(FileChunk {
            bytes: body.to_vec(),
        })
    }
}

/// `file-read` (`box → proxy`): names a source as an **unsafe** segment stack (a real filename).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileRead {
    pub source: UnsafeSegmentStack,
}

impl Wire for FileRead {
    const VERB: u32 = verb::FILE_READ;

    fn encode(&self, out: &mut Vec<u8>) {
        self.source.encode(out);
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        Ok(FileRead {
            source: scan_unsafe(body)?,
        })
    }
}

/// `file-error` (`either`): the shared error body carried on verb `0x27`. `net-error` (`0x32`,
/// deferred) is the same body on a different verb.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileError(pub ErrorBody);

impl Wire for FileError {
    const VERB: u32 = verb::FILE_ERROR;

    fn encode(&self, out: &mut Vec<u8>) {
        self.0.encode_body(out);
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        ErrorBody::decode_body(body).map(FileError)
    }
}

/// `file-write` (0x20, box→proxy): declares a **safe** segment stack destination under the quarantine
/// root. Routes through the 001 `scan_safe` — not a re-implementation.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileWrite(pub SafeSegmentStack);

impl Wire for FileWrite {
    const VERB: u32 = verb::FILE_WRITE;

    fn encode(&self, out: &mut Vec<u8>) {
        self.0.encode(out);
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        scan_safe(body).map(FileWrite)
    }
}

verbatim_verb! {
    /// `file-data` (0x23, proxy→box): verbatim read bytes (the 001 verbatim tail, like `file-chunk`).
    FileData, verb::FILE_DATA
}

/// `list` (0x24, box→proxy / head→box): names a directory as an **unsafe** segment stack. Routes
/// through the 001 `scan_unsafe`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct List(pub UnsafeSegmentStack);

impl Wire for List {
    const VERB: u32 = verb::LIST;

    fn encode(&self, out: &mut Vec<u8>) {
        self.0.encode(out);
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        scan_unsafe(body).map(List)
    }
}

/// `request-file` (0x26, head→box): names a source as an **unsafe** segment stack on the box. Routes
/// through the 001 `scan_unsafe`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RequestFile(pub UnsafeSegmentStack);

impl Wire for RequestFile {
    const VERB: u32 = verb::REQUEST_FILE;

    fn encode(&self, out: &mut Vec<u8>) {
        self.0.encode(out);
    }

    fn decode(body: &[u8]) -> Result<Self, Reject> {
        scan_unsafe(body).map(RequestFile)
    }
}
