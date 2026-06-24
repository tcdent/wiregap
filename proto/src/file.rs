//! The file service seeds: `file-chunk` (verbatim tail), `file-read` (unsafe segment stack), and
//! `file-error` (the shared error body carried on verb `0x27`). See [`file.md`].
//!
//! [`file.md`]: ../../docs/proto/file.md

use alloc::vec::Vec;

use crate::errors::ErrorBody;
use crate::reject::Reject;
use crate::segment::{UnsafeSegmentStack, scan_unsafe};
use crate::verb;
use crate::wire::Wire;

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
