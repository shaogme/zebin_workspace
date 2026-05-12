use alloc::{boxed::Box, collections::VecDeque, vec::Vec};
use core::task::Poll;

use crate::{Cursor, FieldEncoding, ObjectEncoding, ValidationContext};
use crate::{
    archive::slice::SequenceSource,
    error::{DecodeError, ZebinError},
    traits::{
        Archive, ArchivedDefault, ArchivedLayout, ByteSink, Decode, Restore, SchemaAware,
        Serialize, SerializeState,
    },
};

impl<'a, T> SchemaAware for ArchivedVec<'a, T> {
    fn stable_schema_key(&self) -> u32 {
        0
    }

    fn schema_revision(&self) -> u32 {
        0
    }
}

impl<T> SequenceSource<T> for VecDeque<T> {
    fn len(&self) -> usize {
        self.len()
    }

    fn get(&self, index: usize) -> &T {
        &self[index]
    }
}

/// Decoded archived vector view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedVec<'a, T> {
    items: Vec<T>,
    _marker: core::marker::PhantomData<&'a ()>,
}

impl<'a, T> ArchivedVec<'a, T> {
    pub fn new(items: Vec<T>) -> Self {
        Self {
            items,
            _marker: core::marker::PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub unsafe fn as_slice(&self) -> &[T] {
        self.items.as_slice()
    }

    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.items.iter()
    }
}

impl<'a, T: 'static> ArchivedDefault for ArchivedVec<'a, T> {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchivedVec<'static, ()> = ArchivedVec {
            items: Vec::new(),
            _marker: core::marker::PhantomData,
        };
        unsafe { &*(&DEFAULT as *const ArchivedVec<'static, ()> as *const Self) }
    }
}

impl<A> ArchivedLayout for ArchivedVec<'_, A>
where
    A: ArchivedLayout,
{
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::Sequence;
    const FIELD_ENCODING: FieldEncoding = FieldEncoding::Sequence;
}

impl<'marker, 'a, A> Decode<'a> for ArchivedVec<'marker, A>
where
    A: Decode<'a>,
{
    type View = ArchivedVec<'a, A::View>;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let len = cursor.read_u32(context)? as usize;
        let mut items = Vec::with_capacity(len);

        if let Some(_fixed_size) = A::FIXED_SIZE {
            cursor.align(A::ALIGNMENT, context)?;
            for index in 0..len {
                let mut guard = context.push_index(index);
                items.push(A::decode(cursor, &mut *guard)?);
            }
        } else {
            for index in 0..len {
                let mut guard = context.push_index(index);
                items.push(A::decode(cursor, &mut *guard)?);
            }
        }

        Ok(ArchivedVec::new(items))
    }

    fn validate<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let len = cursor.read_u32(context)? as usize;

        if A::FIXED_SIZE.is_some() {
            cursor.align(A::ALIGNMENT, context)?;
        }

        for index in 0..len {
            let mut guard = context.push_index(index);
            A::validate(cursor, &mut *guard)?;
        }

        Ok(())
    }
}

impl<T, U> Restore<Vec<U>> for ArchivedVec<'_, T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<Vec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.items.len());
        for item in &self.items {
            out.push(item.restore()?);
        }
        Ok(out)
    }
}

impl<T, U> Restore<Vec<U>> for Vec<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<Vec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.len());
        for item in self {
            out.push(item.restore()?);
        }
        Ok(out)
    }
}

impl<T> Archive for [T]
where
    T: Archive,
{
    type Archived = ArchivedVec<'static, T::Archived>;
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
        <T::Archived as ArchivedLayout>::FIXED_SIZE.is_some()
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
            encoder.align(<T::Archived as ArchivedLayout>::ALIGNMENT)?;
            if !encoder
                .pos()
                .is_multiple_of(<T::Archived as ArchivedLayout>::ALIGNMENT.get())
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
