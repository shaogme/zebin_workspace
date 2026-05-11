use alloc::{boxed::Box, collections::VecDeque, vec::Vec};
use core::task::Poll;

use crate::{
    archive::slice::{ArchivedVec, SequenceSource},
    error::ZebinError,
    traits::{Archive, ByteSink, Decode, Restore, Serialize, SerializeState},
};

impl<T> SequenceSource<T> for VecDeque<T> {
    fn len(&self) -> usize {
        self.len()
    }

    fn get(&self, index: usize) -> &T {
        &self[index]
    }
}

pub fn archived_bytes<T>(value: &T) -> Result<Vec<u8>, ZebinError>
where
    T: Serialize + Archive + ?Sized,
{
    let len = crate::measure_serialized_len(value)?;
    let mut out = vec![0u8; len];
    let mut encoder = crate::write::encoder::SliceEncoder::new(&mut out, 0);
    let mut state = value.begin_serialize()?;
    loop {
        match state.poll(&mut encoder)? {
            Poll::Pending => continue,
            Poll::Ready(()) => return Ok(out),
        }
    }
}

pub struct SequenceArchiveState<'a, S, T>
where
    S: ?Sized + SequenceSource<T>,
    T: Serialize + Archive + 'a,
{
    source: &'a S,
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    aligned: bool,
    index: usize,
    current_state: Option<Box<<T as Serialize>::State<'a>>>,
}

impl<'a, S, T> SequenceArchiveState<'a, S, T>
where
    S: ?Sized + SequenceSource<T>,
    T: Serialize + Archive + 'a,
{
    pub(crate) fn new(source: &'a S) -> Result<Self, ZebinError> {
        let len = u32::try_from(source.len()).map_err(|_| ZebinError::SerializationError {
            pos: 0,
            message: "sequence length exceeds u32 range",
        })?;
        Ok(Self {
            source,
            len_prefix: len.to_le_bytes(),
            prefix_cursor: 0,
            aligned: false,
            index: 0,
            current_state: None,
        })
    }

    fn fixed_width() -> bool
    where
        T::Archived: for<'b> Decode<'b>,
    {
        <T::Archived as Decode<'static>>::FIXED_SIZE.is_some()
    }

    fn ensure_current_state(&mut self) -> Result<(), ZebinError>
    where
        T::Archived: for<'b> Decode<'b>,
    {
        if self.current_state.is_some() {
            return Ok(());
        }

        self.current_state = Some(Box::new(self.source.get(self.index).begin_serialize()?));
        Ok(())
    }
}

impl<'a, S, T> SerializeState<'a> for SequenceArchiveState<'a, S, T>
where
    S: ?Sized + SequenceSource<T>,
    T: Serialize + Archive + 'a,
    T::Archived: for<'b> Decode<'b>,
{
    fn poll<E: ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<Poll<()>, ZebinError> {
        if self.prefix_cursor < self.len_prefix.len() {
            let written = encoder.write(&self.len_prefix[self.prefix_cursor..])?;
            self.prefix_cursor += written;
            if self.prefix_cursor < self.len_prefix.len() {
                return Ok(Poll::Pending);
            }
        }

        if Self::fixed_width() && !self.aligned {
            encoder.align(<T::Archived as Decode<'static>>::ALIGNMENT)?;
            if !encoder
                .pos()
                .is_multiple_of(<T::Archived as Decode<'static>>::ALIGNMENT.get())
            {
                return Ok(Poll::Pending);
            }
            self.aligned = true;
        }

        while self.index < self.source.len() {
            self.ensure_current_state()?;

            let state = self
                .current_state
                .as_mut()
                .expect("current state initialized above");
            match state.poll(encoder)? {
                Poll::Pending => return Ok(Poll::Pending),
                Poll::Ready(()) => {
                    self.current_state = None;
                    self.index += 1;
                }
            }
        }

        Ok(Poll::Ready(()))
    }
}

pub struct ArrayArchiveState<'a, T, const N: usize>
where
    T: Serialize + Archive + 'a,
{
    items: &'a [T; N],
    index: usize,
    current_state: Option<<T as Serialize>::State<'a>>,
}

impl<'a, T, const N: usize> ArrayArchiveState<'a, T, N>
where
    T: Serialize + Archive + 'a,
{
    pub(crate) fn new(items: &'a [T; N]) -> Self {
        Self {
            items,
            index: 0,
            current_state: None,
        }
    }
}

impl<'a, T, const N: usize> SerializeState<'a> for ArrayArchiveState<'a, T, N>
where
    T: Serialize + Archive + 'a,
{
    fn poll<E: ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<Poll<()>, ZebinError> {
        while self.index < N {
            if self.current_state.is_none() {
                self.current_state = Some(self.items[self.index].begin_serialize()?);
            }
            let state = self
                .current_state
                .as_mut()
                .expect("array item state initialized above");
            match state.poll(encoder)? {
                Poll::Pending => return Ok(Poll::Pending),
                Poll::Ready(()) => {
                    self.current_state = None;
                    self.index += 1;
                }
            }
        }
        Ok(Poll::Ready(()))
    }
}

pub type VecArchiveState<'a, T> = SequenceArchiveState<'a, [T], T>;
pub type VecDequeArchiveState<'a, T> = SequenceArchiveState<'a, VecDeque<T>, T>;
pub type SliceArchiveState<'a, T> = SequenceArchiveState<'a, [T], T>;

impl<T: Archive> Archive for Vec<T> {
    type Archived = ArchivedVec<'static, T::Archived>;
}

impl<T> Serialize for Vec<T>
where
    T: Serialize + Archive,
    T::Archived: for<'b> Decode<'b>,
{
    type State<'a>
        = VecArchiveState<'a, T>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        VecArchiveState::new(self.as_slice())
    }
}

impl<T> Archive for VecDeque<T>
where
    T: Archive,
{
    type Archived = ArchivedVec<'static, T::Archived>;
}

impl<T> Serialize for VecDeque<T>
where
    T: Serialize + Archive,
    T::Archived: for<'b> Decode<'b>,
{
    type State<'a>
        = VecDequeArchiveState<'a, T>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        VecDequeArchiveState::new(self)
    }
}

impl<T> Serialize for [T]
where
    T: Serialize + Archive,
    T::Archived: for<'b> Decode<'b>,
{
    type State<'a>
        = SliceArchiveState<'a, T>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        SliceArchiveState::new(self)
    }
}

impl<T, const N: usize> Serialize for [T; N]
where
    T: Serialize + Archive,
{
    type State<'a>
        = ArrayArchiveState<'a, T, N>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(ArrayArchiveState::new(self))
    }
}

impl<T, U> Restore<VecDeque<U>> for ArchivedVec<'_, T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<VecDeque<U>, ZebinError> {
        let mut queue = VecDeque::with_capacity(self.len());
        for item in self.iter() {
            queue.push_back(item.restore()?);
        }
        Ok(queue)
    }
}

impl<T, U> Restore<VecDeque<U>> for VecDeque<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<VecDeque<U>, ZebinError> {
        let mut queue = VecDeque::with_capacity(self.len());
        for item in self {
            queue.push_back(item.restore()?);
        }
        Ok(queue)
    }
}
