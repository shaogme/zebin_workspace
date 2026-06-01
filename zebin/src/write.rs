use core::marker::PhantomData;

use crate::io_impl::storage::SliceSerializer;
use crate::prelude::*;

#[cfg(feature = "alloc")]
use crate::io_impl::storage::{VecSerializer, VecSerializerCursor};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

enum SerializePhase<'a, T, H = ArchiveHeader>
where
    T: Serialize + Archive + 'a,
    H: ArchiveHeaderTrait,
{
    Header {
        bytes: H::Bytes,
        cursor: usize,
        next_serializer: Option<<T as Serialize>::Serializer<'a>>,
    },
    Body {
        serializer: <T as Serialize>::Serializer<'a>,
        started: bool,
    },
    Done {
        _phantom: PhantomData<H>,
    },
}

pub type ZebinWriter<'a, T, S = SliceSerializer<'a>> =
    ArchiveWriter<'a, T, <S as StorageMut>::CursorMut<'a>, ArchiveHeader>;

/// Stateful archive writer that can stream into caller-provided buffers.
///
/// Owns the value being archived; the value is moved into the body serializer on
/// the first `write` that enters the `Body` phase, after which the writer holds
/// no reference to the original allocation.
pub struct ArchiveWriter<'a, T, C, H = ArchiveHeader>
where
    T: Serialize + Archive + 'a,
    C: CursorMut<'a>,
    H: ArchiveHeaderTrait,
{
    cursor_mut: C,
    value: Option<<T as Serialize>::Input<'a>>,
    phase: SerializePhase<'a, T, H>,
    pos: usize,
}

impl<'a, T, C, H> ArchiveWriter<'a, T, C, H>
where
    T: Serialize + Archive + 'a,
    C: CursorMut<'a>,
    H: ArchiveHeaderTrait,
    T::Archived: ArchivedLayout,
{
    pub fn new(cursor_mut: C) -> Result<Self, ZebinError> {
        let header = H::create(<T::Archived as ArchivedLayout>::OBJECT_ENCODING as u8);
        let pos = cursor_mut.pos();
        Ok(Self {
            cursor_mut,
            value: None,
            phase: SerializePhase::Header {
                bytes: header.serialize(),
                cursor: 0,
                next_serializer: Some(T::serializer()),
            },
            pos,
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
        self.pos
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.phase, SerializePhase::Done { .. })
    }

    fn drive(&mut self) -> Result<usize, ZebinError> {
        let mut writer = &mut self.cursor_mut;
        let start_pos = writer.pos();
        loop {
            match &mut self.phase {
                SerializePhase::Header {
                    bytes,
                    cursor,
                    next_serializer,
                } => {
                    let remaining = bytes.as_ref().len() - *cursor;
                    if writer
                        .write(&bytes.as_ref()[*cursor..])?
                        .advance_cursor(cursor, remaining)
                        .is_pending()
                    {
                        break;
                    }
                    let serializer = next_serializer.take().ok_or(ZebinError::SerializeError {
                        pos: writer.pos(),
                        message: "archive writer state machine error: body serializer missing",
                    })?;
                    self.phase = SerializePhase::Body {
                        serializer,
                        started: false,
                    };
                }
                SerializePhase::Body {
                    serializer,
                    started,
                } => {
                    if !*started {
                        let value = self.value.take().ok_or(ZebinError::SerializeError {
                            pos: writer.pos(),
                            message: "archive writer used after value taken",
                        })?;
                        match serializer.input(value, &mut writer)? {
                            core::task::Poll::Pending => {
                                *started = true;
                                break;
                            }
                            core::task::Poll::Ready(()) => {
                                *started = true;
                            }
                        }
                    } else {
                        match serializer.poll_pending(&mut writer)? {
                            core::task::Poll::Pending => break,
                            core::task::Poll::Ready(()) => {}
                        }
                    }

                    if let SerializePhase::Body { serializer, .. } = core::mem::replace(
                        &mut self.phase,
                        SerializePhase::Done {
                            _phantom: PhantomData,
                        },
                    ) {
                        match serializer.finish(&mut writer)? {
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
                SerializePhase::Done { .. } => break,
            }
        }
        let end_pos = writer.pos();
        self.pos = end_pos;
        Ok(end_pos.saturating_sub(start_pos))
    }

    pub fn write(&mut self, value: <T as Serialize>::Input<'a>) -> Result<usize, ZebinError> {
        if self.is_finished() {
            return Ok(0);
        }

        if self.value.is_none() && matches!(self.phase, SerializePhase::Header { .. }) {
            self.value = Some(value);
        }

        self.drive()
    }

    pub fn write_all(&mut self, value: <T as Serialize>::Input<'a>) -> Result<usize, ZebinError> {
        if self.value.is_none() && matches!(self.phase, SerializePhase::Header { .. }) {
            self.value = Some(value);
        }

        let start_pos = self.pos;
        while !self.is_finished() {
            let before = self.pos;
            let _ = self.drive()?;
            let after = self.pos;
            if before == after && !self.is_finished() {
                return Err(ZebinError::BufferTooSmall {
                    pos: after,
                    required: 0,
                });
            }
        }
        Ok(self.pos.saturating_sub(start_pos))
    }
}

#[cfg(feature = "alloc")]
impl<'a, T, H> ArchiveWriter<'a, T, VecSerializerCursor<'a>, H>
where
    T: Serialize + Archive + 'a,
    H: ArchiveHeaderTrait,
    T::Archived: ArchivedLayout,
{
    pub fn serialize(value: <T as Serialize>::Input<'a>) -> Result<Vec<u8>, ZebinError> {
        let header = H::create(<T::Archived as ArchivedLayout>::OBJECT_ENCODING as u8);
        let mut serializer = VecSerializer::new(0);
        {
            let mut writer = (&mut serializer).into_cursor_mut();
            writer.write(header.serialize().as_ref())?;

            let mut body_serializer = T::serializer();
            if body_serializer.input(value, &mut writer)?.is_pending() {
                while body_serializer.poll_pending(&mut writer)?.is_pending() {}
            }
            let _ = body_serializer.finish(&mut writer)?;
        }
        Ok(serializer.into_inner())
    }

    pub fn serialize_into(
        value: <T as Serialize>::Input<'a>,
        buf: &mut Vec<u8>,
    ) -> Result<(), ZebinError> {
        buf.clear();
        *buf = Self::serialize(value)?;
        Ok(())
    }
}
