pub mod encoder;

use core::marker::PhantomData;

use crate::{
    error::ZebinError,
    format::ArchiveHeader,
    traits::{
        Archive, ArchiveHeader as ArchiveHeaderTrait, ArchivedLayout, ByteSink, Decode, Serialize,
        SerializeState,
    },
    write::encoder::{MeasureEncoder, SliceEncoder},
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
        next_state: Option<<T as Serialize>::State<'a>>,
    },
    Body {
        state: <T as Serialize>::State<'a>,
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
    phase: EncodePhase<'a, T, H>,
    archive_pos: usize,
    total_len: usize,
}

impl<'a, T, H> ArchiveWriter<'a, T, H>
where
    T: Serialize + Archive + 'a,
    H: ArchiveHeaderTrait,
    T::Archived: for<'b> Decode<'b>,
{
    pub fn new(value: &'a T) -> Result<Self, ZebinError> {
        let total_len = measure_total_len::<T, H>(value)?;
        let header = H::create(<T::Archived as ArchivedLayout>::OBJECT_ENCODING as u8);
        Ok(Self {
            phase: EncodePhase::Header {
                bytes: header.encode(),
                cursor: 0,
                next_state: Some(value.begin_serialize()?),
            },
            archive_pos: 0,
            total_len,
        })
    }

    pub fn total_len(&self) -> usize {
        self.total_len
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

        let mut encoder = SliceEncoder::new(out, self.archive_pos);
        loop {
            match &mut self.phase {
                EncodePhase::Header {
                    bytes,
                    cursor,
                    next_state,
                } => {
                    let written = encoder.write(&bytes.as_ref()[*cursor..])?;
                    *cursor += written;
                    if *cursor < bytes.as_ref().len() {
                        break;
                    }
                    let state = next_state.take().ok_or(ZebinError::SerializationError {
                        pos: encoder.pos(),
                        message: "writer body state was already consumed",
                    })?;
                    self.phase = EncodePhase::Body { state };
                }
                EncodePhase::Body { state } => match state.poll(&mut encoder)? {
                    core::task::Poll::Pending => break,
                    core::task::Poll::Ready(()) => {
                        self.phase = EncodePhase::Done {
                            _phantom: PhantomData,
                        };
                        break;
                    }
                },
                EncodePhase::Done { .. } => break,
            }
        }

        self.archive_pos = encoder.pos();
        Ok(encoder.written())
    }

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
                return Err(ZebinError::BufferTooSmall {
                    pos: self.archive_pos,
                    required: self.total_len.saturating_sub(self.archive_pos),
                });
            }
        }
        Ok(total_written)
    }

    pub fn encode_chunked(value: &'a T) -> Result<Self, ZebinError> {
        Self::new(value)
    }

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

fn measure_total_len<'a, T, H>(value: &'a T) -> Result<usize, ZebinError>
where
    T: Serialize + Archive + 'a,
    H: ArchiveHeaderTrait,
{
    let body_len = measure_body_len_from_pos(value, H::SIZE)?;
    H::SIZE
        .checked_add(body_len)
        .ok_or(ZebinError::ArithmeticOverflow { pos: H::SIZE })
}

pub(crate) fn measure_body_len_from_pos<T>(value: &T, start_pos: usize) -> Result<usize, ZebinError>
where
    T: Serialize + Archive + ?Sized,
{
    let mut encoder = MeasureEncoder::new(start_pos);
    let mut state = value.begin_serialize()?;
    loop {
        match state.poll(&mut encoder)? {
            core::task::Poll::Pending => continue,
            core::task::Poll::Ready(()) => {
                return encoder
                    .pos()
                    .checked_sub(start_pos)
                    .ok_or(ZebinError::ArithmeticOverflow { pos: start_pos });
            }
        }
    }
}

pub fn measure_body_len<T>(value: &T) -> Result<usize, ZebinError>
where
    T: Serialize + Archive + ?Sized,
{
    measure_body_len_from_pos(value, 0)
}
