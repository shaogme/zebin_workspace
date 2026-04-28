#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(any(feature = "no_std", feature = "std")))]
compile_error!(
    "Please enable at least one of the features: no_std or std. 
	Use --no-default-features flag to disable default features when you need no_std."
);

pub extern crate alloc;

mod access;
mod archive;
mod core;
mod format;
mod layout;
mod num;
mod storage;
mod traits;

pub mod prelude {
    #[cfg(feature = "mmap")]
    pub use crate::storage::mmap::Mmap;
    pub use crate::traits::{ByteState, Encoder, Serialize, Validate, ZebinError};
    pub use crate::{
        ARCHIVE_HEADER_SIZE, ARCHIVE_MAGIC, ARCHIVE_VERSION, Archive, ArchiveHeader, ArchiveView,
        ArchiveWriter, LayoutDescriptor, LayoutDirectory, LayoutField, LayoutView, RelPtr,
        SerializePoll, SerializeState, Storage, Validator, decode, encode, encode_chunked,
        encode_into, validate,
    };
    pub use zebin_macros::{ZebinArchive, ZebinSerialize};
}

pub use crate::access::ArchiveView;
pub use crate::core::rel_ptr::RelPtr;
pub use crate::core::schema::{LayoutDescriptor, LayoutDirectory, LayoutField, LayoutView};
pub use crate::core::validator::Validator;
pub use crate::format::{ARCHIVE_HEADER_SIZE, ARCHIVE_MAGIC, ARCHIVE_VERSION, ArchiveHeader};
pub use crate::storage::Storage;
#[cfg(feature = "mmap")]
pub use crate::storage::mmap::Mmap;
pub use crate::traits::*;
pub use memoffset;
pub use zebin_macros::*;

use crate::{
    format::ArchiveHeader as ArchiveFormatHeader,
    layout::{MeasureEncoder, SliceEncoder, build_layout_section_bytes},
    num::{u32_to_usize, usize_to_nonzero_u32},
};

use alloc::{string::ToString, vec, vec::Vec};

struct EncodePlan {
    root_pos: usize,
    layout_pos: usize,
    total_len: usize,
    header_bytes: [u8; ARCHIVE_HEADER_SIZE],
    layout_bytes: Vec<u8>,
    layouts: Vec<LayoutDescriptor>,
}

fn measure_plan<T>(value: &T) -> Result<EncodePlan, ZebinError>
where
    T: Serialize + Archive,
{
    let mut encoder = MeasureEncoder::new(ARCHIVE_HEADER_SIZE);
    let mut state = value.begin()?;
    let resolver = loop {
        match state.poll(&mut encoder)? {
            SerializePoll::Pending => continue,
            SerializePoll::Error(err) => return Err(err),
            SerializePoll::Ready(resolver) => break resolver,
        }
    };

    encoder.align(T::ALIGNMENT)?;
    let root_offset = encoder.pos();
    let root_offset = usize_to_nonzero_u32(
        root_offset,
        || ZebinError::ValidationError {
            message: "Root offset exceeds u32 range".to_string(),
            pos: root_offset,
        },
        || ZebinError::ValidationError {
            message: "Root offset cannot be zero".to_string(),
            pos: root_offset,
        },
    )?;

    let root_pos = u32_to_usize(root_offset.get(), || ZebinError::WriteError)?;
    let archived = value.resolve(root_pos, resolver)?;
    let archived_bytes = T::archived_bytes(&archived);
    encoder.write(&archived_bytes)?;

    let layout_offset = encoder.pos();
    let layout_offset = usize_to_nonzero_u32(
        layout_offset,
        || ZebinError::ValidationError {
            message: "Layout section offset exceeds u32 range".to_string(),
            pos: layout_offset,
        },
        || ZebinError::ValidationError {
            message: "Layout section offset cannot be zero".to_string(),
            pos: layout_offset,
        },
    )?;

    let layouts = encoder.into_layouts();
    let layout_bytes = build_layout_section_bytes(&layouts)?;
    let layout_pos = u32_to_usize(layout_offset.get(), || ZebinError::WriteError)?;
    let total_len = layout_pos
        .checked_add(layout_bytes.len())
        .ok_or(ZebinError::WriteError)?;

    Ok(EncodePlan {
        root_pos,
        layout_pos,
        total_len,
        header_bytes: ArchiveFormatHeader::to_bytes(layout_offset, root_offset),
        layout_bytes,
        layouts,
    })
}

struct RootWriteState {
    bytes: Vec<u8>,
    cursor: usize,
}

impl RootWriteState {
    fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, cursor: 0 }
    }
}

enum EncodePhase<'a, T>
where
    T: Serialize + Archive + 'a,
{
    Header {
        cursor: usize,
    },
    Body {
        state: <T as Serialize>::State<'a>,
    },
    RootAlign {
        resolver: Option<<T as Archive>::Resolver>,
    },
    Root {
        state: RootWriteState,
    },
    Layout {
        cursor: usize,
    },
    Done,
}

/// Stateful archive writer that can stream into caller-provided buffers.
pub struct ArchiveWriter<'a, T>
where
    T: Serialize + Archive + 'a,
{
    value: &'a T,
    plan: EncodePlan,
    body_state: Option<<T as Serialize>::State<'a>>,
    phase: EncodePhase<'a, T>,
    archive_pos: usize,
    layouts: layout::LayoutRegistry,
}

impl<'a, T> ArchiveWriter<'a, T>
where
    T: Serialize + Archive + 'a,
{
    pub fn new(value: &'a T) -> Result<Self, ZebinError> {
        let plan = measure_plan(value)?;
        Ok(Self {
            value,
            body_state: Some(value.begin()?),
            phase: EncodePhase::Header { cursor: 0 },
            archive_pos: 0,
            layouts: layout::LayoutRegistry::default(),
            plan,
        })
    }

    pub fn total_len(&self) -> usize {
        self.plan.total_len
    }

    pub fn written(&self) -> usize {
        self.archive_pos
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.phase, EncodePhase::Done)
    }

    pub fn write(&mut self, out: &mut [u8]) -> Result<usize, ZebinError> {
        if self.is_finished() || out.is_empty() {
            return Ok(0);
        }

        let mut encoder = SliceEncoder::new(out, self.archive_pos, &mut self.layouts);
        loop {
            match &mut self.phase {
                EncodePhase::Header { cursor } => {
                    let written = encoder.write(&self.plan.header_bytes[*cursor..])?;
                    *cursor += written;
                    if *cursor < self.plan.header_bytes.len() {
                        break;
                    }
                    let state = self
                        .body_state
                        .take()
                        .expect("body state should be initialized from writer construction");
                    self.phase = EncodePhase::Body { state };
                }
                EncodePhase::Body { state } => match state.poll(&mut encoder)? {
                    SerializePoll::Pending => break,
                    SerializePoll::Error(err) => return Err(err),
                    SerializePoll::Ready(resolver) => {
                        self.phase = EncodePhase::RootAlign {
                            resolver: Some(resolver),
                        };
                    }
                },
                EncodePhase::RootAlign { resolver } => {
                    encoder.align(T::ALIGNMENT)?;
                    if !encoder.pos().is_multiple_of(T::ALIGNMENT.get()) {
                        break;
                    }
                    if encoder.pos() != self.plan.root_pos {
                        return Err(ZebinError::ValidationError {
                            message: "Root offset mismatch during emission".to_string(),
                            pos: encoder.pos(),
                        });
                    }

                    let resolver = resolver
                        .take()
                        .expect("resolver available while entering root alignment");
                    let archived = self.value.resolve(encoder.pos(), resolver)?;
                    let bytes = T::archived_bytes(&archived);
                    self.phase = EncodePhase::Root {
                        state: RootWriteState::new(bytes),
                    };
                }
                EncodePhase::Root { state } => {
                    if encoder.pos() != self.plan.root_pos && state.cursor == 0 {
                        return Err(ZebinError::ValidationError {
                            message: "Root offset mismatch during root write".to_string(),
                            pos: encoder.pos(),
                        });
                    }

                    let written = encoder.write(&state.bytes[state.cursor..])?;
                    state.cursor += written;
                    if state.cursor < state.bytes.len() {
                        break;
                    }

                    if encoder.pos() != self.plan.layout_pos {
                        return Err(ZebinError::ValidationError {
                            message: "Layout offset mismatch during emission".to_string(),
                            pos: encoder.pos(),
                        });
                    }

                    if encoder.layouts() != self.plan.layouts.as_slice() {
                        return Err(ZebinError::ValidationError {
                            message: "Layout registry diverged during emission".to_string(),
                            pos: encoder.pos(),
                        });
                    }

                    self.phase = EncodePhase::Layout { cursor: 0 };
                }
                EncodePhase::Layout { cursor } => {
                    let written = encoder.write(&self.plan.layout_bytes[*cursor..])?;
                    *cursor += written;
                    if *cursor < self.plan.layout_bytes.len() {
                        break;
                    }
                    self.phase = EncodePhase::Done;
                    break;
                }
                EncodePhase::Done => break,
            }
        }

        self.archive_pos = encoder.pos();
        Ok(encoder.written())
    }

    /// Writes the archive into the provided buffer until completion.
    pub fn write_all(&mut self, out: &mut [u8]) -> Result<usize, ZebinError> {
        let mut total_written = 0usize;
        while !self.is_finished() {
            let chunk = self.write(&mut out[total_written..])?;
            total_written = total_written
                .checked_add(chunk)
                .ok_or(ZebinError::WriteError)?;
            if chunk == 0 && !self.is_finished() {
                return Err(ZebinError::WriteError);
            }
        }
        Ok(total_written)
    }
}

/// Create a chunked archive writer that can be resumed with caller-provided buffers.
pub fn encode_chunked<T>(value: &T) -> Result<ArchiveWriter<'_, T>, ZebinError>
where
    T: Serialize + Archive,
{
    ArchiveWriter::new(value)
}

/// Serialize a value into a newly allocated byte vector.
pub fn encode<T>(value: &T) -> Result<Vec<u8>, ZebinError>
where
    T: Serialize + Archive,
{
    let mut writer = encode_chunked(value)?;
    let mut buf = vec![0u8; writer.total_len()];
    writer.write_all(&mut buf)?;
    Ok(buf)
}

/// Serialize a value into an existing vector, replacing its contents.
pub fn encode_into<T>(value: &T, buf: &mut Vec<u8>) -> Result<(), ZebinError>
where
    T: Serialize + Archive,
{
    let mut writer = encode_chunked(value)?;
    buf.clear();
    buf.resize(writer.total_len(), 0);
    writer.write_all(buf)?;
    Ok(())
}

/// Decode and validate a byte slice into a zero-copy archived view.
pub fn decode<'a, T>(bytes: &'a [u8]) -> Result<ArchiveView<'a, T>, ZebinError>
where
    T: Archive,
    T::Archived: Validate<Validator<'a>>,
{
    access::decode(bytes)
}

/// Validate a byte slice without keeping the archived view.
pub fn validate<'a, T>(bytes: &'a [u8]) -> Result<(), ZebinError>
where
    T: Archive,
    T::Archived: 'a,
    T::Archived: Validate<Validator<'a>>,
{
    access::validate::<T>(bytes)
}
