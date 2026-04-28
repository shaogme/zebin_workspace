pub mod encoder;
pub mod plan;
pub mod state;

use crate::{
    error::ZebinError,
    traits::{Archive, Layout},
    write::{
        encoder::{LayoutRegistry, SliceEncoder},
        plan::{EncodePlan, measure_plan},
        state::{Serialize, SerializeState},
    },
};
use alloc::{string::ToString, vec, vec::Vec};

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
    layouts: LayoutRegistry,
}

impl<'a, T> ArchiveWriter<'a, T>
where
    T: Serialize + Archive + 'a,
{
    pub fn new(value: &'a T) -> Result<Self, ZebinError> {
        let plan = measure_plan(value)?;
        Ok(Self {
            value,
            body_state: Some(value.begin_serialize()?),
            phase: EncodePhase::Header { cursor: 0 },
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
        matches!(self.phase, EncodePhase::Done)
    }

    pub fn write(&mut self, out: &mut [u8]) -> Result<usize, ZebinError> {
        if self.is_finished() || out.is_empty() {
            return Ok(0);
        }

        use crate::io::sink::ByteSink;
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
                    ::core::task::Poll::Pending => break,
                    ::core::task::Poll::Ready(resolver) => {
                        self.phase = EncodePhase::RootAlign {
                            resolver: Some(resolver),
                        };
                    }
                },
                EncodePhase::RootAlign { resolver } => {
                    encoder.align(<T::Archived as Layout>::ALIGNMENT)?;
                    if encoder.pos() % <T::Archived as Layout>::ALIGNMENT.get() != 0 {
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
                    let bytes = crate::traits::archived_bytes(&archived);
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

/// Archive a value into a newly allocated byte vector.
pub fn encode<T>(value: &T) -> Result<Vec<u8>, ZebinError>
where
    T: Serialize + Archive,
{
    let mut writer = encode_chunked(value)?;
    let mut buf = vec![0u8; writer.total_len()];
    writer.write_all(&mut buf)?;
    Ok(buf)
}

/// Archive a value into an existing vector, replacing its contents.
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
