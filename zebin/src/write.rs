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

pub type ZebinWriter<'a, T, S = SliceEncoder<'a>> = ArchiveWriter<'a, T, S, ArchiveHeader>;

/// Stateful archive writer that can stream into caller-provided buffers.
///
/// Owns the value being archived; the value is moved into the body encoder on
/// the first `write` that enters the `Body` phase, after which the writer holds
/// no reference to the original allocation.
pub struct ArchiveWriter<'a, T, S, H = ArchiveHeader>
where
    T: Encode + Archive + 'a,
    S: ByteSink,
    H: ArchiveHeaderTrait,
{
    sink: S,
    value: Option<<T as Encode>::Input<'a>>,
    phase: EncodePhase<'a, T, H>,
}

impl<'a, T, S, H> ArchiveWriter<'a, T, S, H>
where
    T: Encode + Archive + 'a,
    S: ByteSink,
    H: ArchiveHeaderTrait,
    T::Archived: ArchivedLayout,
{
    pub fn new(sink: S) -> Result<Self, ZebinError> {
        let header = H::create(<T::Archived as ArchivedLayout>::OBJECT_ENCODING as u8);
        Ok(Self {
            sink,
            value: None,
            phase: EncodePhase::Header {
                bytes: header.encode(),
                cursor: 0,
                next_encoder: Some(T::encoder()),
            },
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
        self.sink.pos()
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.phase, EncodePhase::Done { .. })
    }

    fn drive(&mut self) -> Result<usize, ZebinError> {
        let start_pos = self.sink.pos();
        loop {
            match &mut self.phase {
                EncodePhase::Header {
                    bytes,
                    cursor,
                    next_encoder,
                } => {
                    let remaining = bytes.as_ref().len() - *cursor;
                    if self
                        .sink
                        .write(&bytes.as_ref()[*cursor..])?
                        .advance_cursor(cursor, remaining)
                        .is_pending()
                    {
                        break;
                    }
                    let encoder = next_encoder.take().ok_or(ZebinError::SerializationError {
                        pos: self.sink.pos(),
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
                            pos: self.sink.pos(),
                            message: "archive writer used after value taken",
                        })?;
                        match encoder.input(value, &mut self.sink)? {
                            core::task::Poll::Pending => {
                                *started = true;
                                break;
                            }
                            core::task::Poll::Ready(()) => {
                                *started = true;
                            }
                        }
                    } else {
                        match encoder.poll_pending(&mut self.sink)? {
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
                        match encoder.finish(&mut self.sink)? {
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
        Ok(self.sink.pos().saturating_sub(start_pos))
    }

    pub fn write(&mut self, value: <T as Encode>::Input<'a>) -> Result<usize, ZebinError> {
        if self.is_finished() {
            return Ok(0);
        }

        if self.value.is_none() && matches!(self.phase, EncodePhase::Header { .. }) {
            self.value = Some(value);
        }

        self.drive()
    }

    pub fn write_all(&mut self, value: <T as Encode>::Input<'a>) -> Result<usize, ZebinError> {
        if self.value.is_none() && matches!(self.phase, EncodePhase::Header { .. }) {
            self.value = Some(value);
        }

        let start_pos = self.sink.pos();
        while !self.is_finished() {
            let before = self.sink.pos();
            let _ = self.drive()?;
            let after = self.sink.pos();
            if before == after && !self.is_finished() {
                return Err(ZebinError::BufferTooSmall {
                    pos: after,
                    required: 0,
                });
            }
        }
        Ok(self.sink.pos().saturating_sub(start_pos))
    }
}

#[cfg(feature = "alloc")]
impl<'a, T, H> ArchiveWriter<'a, T, VecEncoder, H>
where
    T: Encode + Archive + 'a,
    H: ArchiveHeaderTrait,
    T::Archived: ArchivedLayout,
{
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

    pub fn encode_into(
        value: <T as Encode>::Input<'a>,
        buf: &mut Vec<u8>,
    ) -> Result<(), ZebinError> {
        buf.clear();
        *buf = Self::encode(value)?;
        Ok(())
    }
}
