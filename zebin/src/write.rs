pub mod encoder;

use core::marker::PhantomData;

use crate::{prelude::*, write::encoder::SliceEncoder};

#[cfg(feature = "alloc")]
use crate::write::encoder::VecEncoder;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

enum EncodePhase<'a, T, H = ArchiveHeader>
where
    T: Encode + Archive + 'a,
    H: ArchiveHeaderTrait,
{
    Header {
        bytes: H::Bytes,
        cursor: usize,
        next_encoder: Option<<T as Encode>::Encoder<'a>>,
    },
    Body {
        encoder: <T as Encode>::Encoder<'a>,
        started: bool,
    },
    Done {
        _phantom: PhantomData<H>,
    },
}

pub type ZebinWriter<'a, T> = ArchiveWriter<'a, T, ArchiveHeader>;

/// Stateful archive writer that can stream into caller-provided buffers.
///
/// Owns the value being archived; the value is moved into the body encoder on
/// the first `write` that enters the `Body` phase, after which the writer holds
/// no reference to the original allocation.
pub struct ArchiveWriter<'a, T, H = ArchiveHeader>
where
    T: Encode + Archive + 'a,
    H: ArchiveHeaderTrait,
{
    value: Option<<T as Encode>::Input<'a>>,
    phase: EncodePhase<'a, T, H>,
    archive_pos: usize,
}

impl<'a, T, H> ArchiveWriter<'a, T, H>
where
    T: Encode + Archive + 'a,
    H: ArchiveHeaderTrait,
    T::Archived: ArchivedLayout,
{
    pub fn new(value: <T as Encode>::Input<'a>) -> Result<Self, ZebinError> {
        let header = H::create(<T::Archived as ArchivedLayout>::OBJECT_ENCODING as u8);
        Ok(Self {
            value: Some(value),
            phase: EncodePhase::Header {
                bytes: header.encode(),
                cursor: 0,
                next_encoder: Some(T::encoder()),
            },
            archive_pos: 0,
        })
    }

    /// Returns the total archive length when known statically.
    ///
    /// Pre-measurement was dropped together with the move-by-value transition;
    /// only fixed-layout archives advertise their length up-front. Variable-size
    /// archives return `None` and grow as bytes are produced.
    pub fn total_len(&self) -> Option<usize> {
        <T::Archived as ArchivedLayout>::FIXED_SIZE.map(|sz| H::SIZE + sz)
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

        let mut encoder_sink = SliceEncoder::new(out, self.archive_pos);
        loop {
            match &mut self.phase {
                EncodePhase::Header {
                    bytes,
                    cursor,
                    next_encoder,
                } => {
                    let remaining = bytes.as_ref().len() - *cursor;
                    if encoder_sink
                        .write(&bytes.as_ref()[*cursor..])?
                        .advance_cursor(cursor, remaining)
                        .is_pending()
                    {
                        break;
                    }
                    let encoder = next_encoder.take().ok_or(ZebinError::SerializationError {
                        pos: encoder_sink.pos(),
                        message: "archive writer state machine error: body encoder missing",
                    })?;
                    self.phase = EncodePhase::Body {
                        encoder,
                        started: false,
                    };
                }
                EncodePhase::Body { encoder, started } => {
                    if !*started {
                        let value = self.value.take().ok_or(ZebinError::SerializationError {
                            pos: encoder_sink.pos(),
                            message: "archive writer used after value taken",
                        })?;
                        match encoder.input(value, &mut encoder_sink)? {
                            core::task::Poll::Pending => {
                                *started = true;
                                break;
                            }
                            core::task::Poll::Ready(()) => {
                                *started = true;
                            }
                        }
                    } else {
                        match encoder.poll_pending(&mut encoder_sink)? {
                            core::task::Poll::Pending => break,
                            core::task::Poll::Ready(()) => {}
                        }
                    }

                    if let EncodePhase::Body { encoder, .. } = core::mem::replace(
                        &mut self.phase,
                        EncodePhase::Done {
                            _phantom: PhantomData,
                        },
                    ) {
                        match encoder.finish(&mut encoder_sink)? {
                            core::task::Poll::Pending => {
                                break;
                            }
                            core::task::Poll::Ready(()) => {
                                break;
                            }
                        }
                    } else {
                        unreachable!();
                    }
                }
                EncodePhase::Done { .. } => break,
            }
        }

        self.archive_pos = encoder_sink.pos();
        Ok(encoder_sink.written())
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
                    required: 0,
                });
            }
        }
        Ok(total_written)
    }

    pub fn encode_chunked(value: <T as Encode>::Input<'a>) -> Result<Self, ZebinError> {
        Self::new(value)
    }

    #[cfg(feature = "alloc")]
    pub fn encode(value: <T as Encode>::Input<'a>) -> Result<Vec<u8>, ZebinError> {
        let header = H::create(<T::Archived as ArchivedLayout>::OBJECT_ENCODING as u8);
        let mut encoder = VecEncoder::new(0);
        encoder.write(header.encode().as_ref())?;

        let mut body_encoder = T::encoder();
        if body_encoder.input(value, &mut encoder)?.is_pending() {
            while body_encoder.poll_pending(&mut encoder)?.is_pending() {}
        }
        let _ = body_encoder.finish(&mut encoder)?;
        Ok(encoder.into_inner())
    }

    #[cfg(feature = "alloc")]
    pub fn encode_into(
        value: <T as Encode>::Input<'a>,
        buf: &mut Vec<u8>,
    ) -> Result<(), ZebinError> {
        buf.clear();
        *buf = Self::encode(value)?;
        Ok(())
    }
}
