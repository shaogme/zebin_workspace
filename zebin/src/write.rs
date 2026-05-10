pub mod encoder;
pub mod plan;
pub mod state;

use core::marker::PhantomData;

use crate::{
    error::{ValidateError, ZebinError},
    format::ArchiveHeader,
    traits::{Archive, ArchiveHeader as ArchiveHeaderTrait, Layout},
    write::{
        encoder::{LayoutRegistry, SliceEncoder},
        plan::{EncodePlan, measure_plan},
        state::{Serialize, SerializeState},
    },
};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

enum EncodePhase<'a, T, H = ArchiveHeader>
where
    T: Serialize + Archive + 'a,
    H: ArchiveHeaderTrait,
{
    Header {
        bytes: H::Bytes,
        cursor: usize,
    },
    Body {
        state: <T as Serialize>::State<'a>,
    },
    RootAlign {
        resolver: Option<<T as Archive>::Resolver>,
    },
    Root {
        archived: T::Archived,
        cursor: usize,
    },
    Layout {
        cursor: usize,
    },
    Done {
        _phantom: PhantomData<H>,
    },
}

pub type ZebinWriter<'a, T> = ArchiveWriter<'a, T, ArchiveHeader>;

/// Stateful archive writer that can stream into caller-provided buffers.
pub struct ArchiveWriter<'a, T, H = ArchiveHeader>
where
    T: Serialize + Archive + 'a,
    H: ArchiveHeaderTrait,
{
    value: &'a T,
    plan: EncodePlan<'a, H>,
    body_state: Option<<T as Serialize>::State<'a>>,
    phase: EncodePhase<'a, T, H>,
    archive_pos: usize,
    layouts: LayoutRegistry<'a>,
}

impl<'a, T, H> ArchiveWriter<'a, T, H>
where
    T: Serialize + Archive + 'a,
    H: ArchiveHeaderTrait,
{
    pub fn new(value: &'a T) -> Result<Self, ZebinError> {
        let plan = measure_plan::<T, H>(value)?;
        let header_bytes = plan.header.encode();
        Ok(Self {
            value,
            body_state: Some(value.begin_serialize()?),
            phase: EncodePhase::Header {
                bytes: header_bytes,
                cursor: 0,
            },
            archive_pos: 0,
            layouts: LayoutRegistry::default(),
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
        matches!(self.phase, EncodePhase::Done { .. })
    }

    pub fn write(&mut self, out: &mut [u8]) -> Result<usize, ZebinError> {
        if self.is_finished() || out.is_empty() {
            return Ok(0);
        }

        use crate::io::sink::ByteSink;
        let mut encoder = SliceEncoder::new(out, self.archive_pos, &mut self.layouts);
        loop {
            match &mut self.phase {
                EncodePhase::Header { bytes, cursor } => {
                    let written = encoder.write(&bytes.as_ref()[*cursor..])?;
                    *cursor += written;
                    if *cursor < bytes.as_ref().len() {
                        break;
                    }
                    let state = self
                        .body_state
                        .take()
                        .expect("body state should be initialized from writer construction");
                    self.phase = EncodePhase::Body { state };
                }
                EncodePhase::Body { state } => match state.poll(&mut encoder)? {
                    ::core::task::Poll::Pending => break,
                    ::core::task::Poll::Ready(resolver) => {
                        self.phase = EncodePhase::RootAlign {
                            resolver: Some(resolver),
                        };
                    }
                },
                EncodePhase::RootAlign { resolver } => {
                    encoder.align(<T::Archived as Layout>::ALIGNMENT)?;
                    if !encoder
                        .pos()
                        .is_multiple_of(<T::Archived as Layout>::ALIGNMENT.get())
                    {
                        break;
                    }
                    if encoder.pos() != self.plan.root_pos {
                        return Err(ValidateError::ValidationError {
                            message: "Root offset mismatch during emission",
                            pos: encoder.pos(),
                        }
                        .into());
                    }

                    let resolver = resolver
                        .take()
                        .expect("resolver available while entering root alignment");
                    let archived = self.value.resolve(encoder.pos(), resolver)?;
                    self.phase = EncodePhase::Root {
                        archived,
                        cursor: 0,
                    };
                }
                EncodePhase::Root { archived, cursor } => {
                    if encoder.pos() != self.plan.root_pos && *cursor == 0 {
                        return Err(ValidateError::ValidationError {
                            message: "Root offset mismatch during root write",
                            pos: encoder.pos(),
                        }
                        .into());
                    }

                    let size = archived.size_hint();
                    let mut temp_buf = [0u8; 1024];
                    if size > temp_buf.len() {
                        return Err(ZebinError::BufferTooSmall {
                            pos: encoder.pos(),
                            required: size,
                        });
                    }
                    T::Archived::write_archived_bytes(archived, &mut temp_buf[..size]);

                    let written = encoder.write(&temp_buf[*cursor..size])?;
                    *cursor += written;
                    if *cursor < size {
                        break;
                    }

                    if encoder.pos() != self.plan.layout_pos {
                        return Err(ValidateError::ValidationError {
                            message: "Layout offset mismatch during emission",
                            pos: encoder.pos(),
                        }
                        .into());
                    }

                    if encoder.layouts().count() != self.plan.layouts.count() {
                        return Err(ValidateError::ValidationError {
                            message: "Layout registry diverged during emission",
                            pos: encoder.pos(),
                        }
                        .into());
                    }

                    self.phase = EncodePhase::Layout { cursor: 0 };
                }
                EncodePhase::Layout { cursor } => {
                    let total_layout_len =
                        crate::write::plan::layout_section_len_registry(&self.plan.layouts)?;
                    let mut temp_buf = [0u8; 256];
                    let remaining = total_layout_len.saturating_sub(*cursor);
                    if remaining == 0 {
                        self.phase = EncodePhase::Done {
                            _phantom: PhantomData,
                        };
                        break;
                    }

                    let chunk_size = temp_buf.len().min(remaining);
                    crate::write::plan::fill_layout_section_chunk(
                        &self.plan.layouts,
                        *cursor,
                        &mut temp_buf[..chunk_size],
                    )?;

                    let written = encoder.write(&temp_buf[..chunk_size])?;
                    *cursor += written;
                    if written == 0 && chunk_size > 0 {
                        break;
                    }
                    if *cursor >= total_layout_len {
                        self.phase = EncodePhase::Done {
                            _phantom: PhantomData,
                        };
                        break;
                    }
                }
                EncodePhase::Done { .. } => break,
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
            total_written =
                total_written
                    .checked_add(chunk)
                    .ok_or(ZebinError::ArithmeticOverflow {
                        pos: self.archive_pos,
                    })?;
            if chunk == 0 && !self.is_finished() {
                return Err(ZebinError::SerializationError {
                    pos: self.archive_pos,
                    message: "writer stuck or output buffer too small",
                });
            }
        }
        Ok(total_written)
    }

    /// Create a chunked archive writer.
    pub fn encode_chunked(value: &'a T) -> Result<Self, ZebinError> {
        Self::new(value)
    }

    /// Archive a value into a newly allocated byte vector.
    #[cfg(feature = "alloc")]
    pub fn encode(value: &'a T) -> Result<Vec<u8>, ZebinError> {
        let mut writer = Self::encode_chunked(value)?;
        let mut buf = vec![0u8; writer.total_len()];
        writer.write_all(&mut buf)?;
        Ok(buf)
    }

    #[cfg(feature = "alloc")]
    pub fn encode_into(value: &'a T, buf: &mut Vec<u8>) -> Result<(), ZebinError> {
        let mut writer = Self::encode_chunked(value)?;
        buf.clear();
        buf.resize(writer.total_len(), 0);
        writer.write_all(buf)?;
        Ok(())
    }
}
