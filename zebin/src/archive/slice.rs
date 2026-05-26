use super::iter::{SeqEncoder, measure_block_index_overhead};
#[cfg(feature = "alloc")]
use crate::io::ForwardSequenceStrategy;
use crate::prelude::*;
use core::num::NonZeroUsize;
use core::task::Poll;

impl<T, const N: usize> FixedLayout for [T; N]
where
    T: FixedLayout,
{
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;
    const SIZE: usize = T::SIZE * N;

    fn write_fixed(archived: &Self, out: &mut [u8]) {
        for i in 0..N {
            T::write_fixed(&archived[i], &mut out[i * T::SIZE..(i + 1) * T::SIZE]);
        }
    }
}

impl<T, const N: usize> Archive for [T; N]
where
    T: Archive,
{
    type Archived = [T::Archived; N];
}

impl<T, U, const N: usize> Restore<[U; N]> for [T; N]
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<[U; N], ZebinError> {
        let mut out = core::mem::MaybeUninit::<[U; N]>::uninit();
        let out_ptr = out.as_mut_ptr() as *mut U;
        let mut initialized = 0usize;

        while initialized < N {
            match self[initialized].restore() {
                Ok(value) => unsafe {
                    out_ptr.add(initialized).write(value);
                    initialized += 1;
                },
                Err(error) => {
                    for index in 0..initialized {
                        unsafe {
                            out_ptr.add(index).drop_in_place();
                        }
                    }
                    return Err(error);
                }
            }
        }

        Ok(unsafe { out.assume_init() })
    }
}

impl<A, const N: usize> ArchivedLayout for [A; N]
where
    A: ArchivedLayout,
{
    const FIXED_SIZE: Option<usize> = match A::FIXED_SIZE {
        Some(size) => Some(size * N),
        None => None,
    };
    const ALIGNMENT: NonZeroUsize = A::ALIGNMENT;
}

impl<A, const N: usize> Decode for [A; N]
where
    A: Decode,
{
    type View<'a>
        = [A::View<'a>; N]
    where
        Self: 'a;
    #[cfg(feature = "alloc")]
    type DecodeStrategy = ForwardSequenceStrategy;

    fn decode<'a, C>(
        cursor: &mut Cursor<'a>,
        context: &mut C,
    ) -> Result<Self::View<'a>, DecodeError>
    where
        C: ValidationContext + ?Sized,
        Self: 'a,
    {
        let mut out = core::mem::MaybeUninit::<[A::View<'a>; N]>::uninit();
        let out_ptr = out.as_mut_ptr() as *mut A::View<'a>;
        let mut initialized = 0usize;

        while initialized < N {
            let mut guard = context.push_index(initialized);
            match A::decode(cursor, &mut *guard) {
                Ok(value) => unsafe {
                    out_ptr.add(initialized).write(value);
                    initialized += 1;
                },
                Err(error) => {
                    for index in 0..initialized {
                        unsafe {
                            out_ptr.add(index).drop_in_place();
                        }
                    }
                    return Err(error);
                }
            }
        }

        Ok(unsafe { out.assume_init() })
    }

    fn validate<'a, C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        for index in 0..N {
            let mut guard = context.push_index(index);
            A::validate(cursor, &mut *guard)?;
        }

        Ok(())
    }
}

pub struct ArrayEncoder<'a, T, const N: usize>
where
    T: Encode + Archive + 'a,
{
    items: Option<core::array::IntoIter<T, N>>,
    item_encoder: <T as Encode>::Encoder<'a>,
    started: bool,
    awaiting_next: bool,
}

impl<'a, T, const N: usize> ArrayEncoder<'a, T, N>
where
    T: Encode + Archive + 'a,
{
    pub(crate) fn new() -> Self {
        Self {
            items: None,
            item_encoder: T::encoder(),
            started: false,
            awaiting_next: true,
        }
    }
}

impl<'a, T, const N: usize> Encoder for ArrayEncoder<'a, T, N>
where
    T: Encode<Input<'a> = T> + Archive + 'a,
{
    type Input = [T; N];

    fn input<Sink: StorageMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut Sink,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        self.items = Some(item.into_iter());
        self.started = true;
        self.awaiting_next = true;
        self.poll_pending(sink)
    }

    fn poll_pending<Sink: StorageMut + ?Sized>(
        &mut self,
        sink: &mut Sink,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        if !self.started {
            return Err(ZebinError::SerializationError {
                pos: sink.pos(),
                message: "ArrayEncoder polled before input",
            });
        }
        let iter = self.items.as_mut().ok_or(ZebinError::SerializationError {
            pos: sink.pos(),
            message: "ArrayEncoder iterator missing",
        })?;
        loop {
            if self.awaiting_next {
                match iter.next() {
                    Some(item) => match self.item_encoder.input(item, sink)? {
                        core::task::Poll::Pending => {
                            self.awaiting_next = false;
                            return Ok(core::task::Poll::Pending);
                        }
                        core::task::Poll::Ready(()) => {
                            // ready immediately; loop to next item.
                        }
                    },
                    None => return Ok(core::task::Poll::Ready(())),
                }
            } else {
                match self.item_encoder.poll_pending(sink)? {
                    core::task::Poll::Pending => return Ok(core::task::Poll::Pending),
                    core::task::Poll::Ready(()) => {
                        self.awaiting_next = true;
                    }
                }
            }
        }
    }

    fn finish<Sink: StorageMut + ?Sized>(
        self,
        sink: &mut Sink,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        self.item_encoder.finish(sink)
    }
}

impl<T, const N: usize> Encode for [T; N]
where
    T: Encode + Archive,
    for<'a> T: Encode<Input<'a> = T> + 'a,
{
    type Input<'a>
        = [T; N]
    where
        Self: 'a;
    type Encoder<'a>
        = ArrayEncoder<'a, T, N>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        ArrayEncoder::new()
    }
}

impl<T, const N: usize> MeasureBody for [T; N]
where
    T: MeasureBody,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        let mut total = 0usize;
        for item in self.iter() {
            total = total
                .checked_add(item.measure_body()?)
                .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
        }
        Ok(total)
    }
}

impl<T> Archive for [T]
where
    T: Archive,
{
    type Archived = crate::archive::ArchivedIter<'static, T::Archived>;
}

/// Borrowed-iterator sequence encoder for DST inputs (`[T]`).
///
/// Each element is cloned out of the borrowed slice and fed to the owned
/// `SeqEncoder<T>`. This intentionally requires `T: Clone`; the memory benefit
/// only applies to the owned-collection path.
pub struct RefIterEncoder<'a, S: ?Sized, T>
where
    for<'b> &'b S: IntoIterator<Item = &'b T>,
    T: Encode + Archive + Clone + 'a,
{
    iter: Option<<&'a S as IntoIterator>::IntoIter>,
    seq_encoder: SeqEncoder<'a, T>,
}

impl<'a, S: ?Sized, T> RefIterEncoder<'a, S, T>
where
    for<'b> &'b S: IntoIterator<Item = &'b T>,
    T: Encode + Archive + Clone + 'a,
{
    pub fn new() -> Self {
        Self {
            iter: None,
            seq_encoder: SeqEncoder::new_indexed(),
        }
    }
}

impl<'a, S: ?Sized, T> Default for RefIterEncoder<'a, S, T>
where
    for<'b> &'b S: IntoIterator<Item = &'b T>,
    T: Encode + Archive + Clone + 'a,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, S: ?Sized, T> Encoder for RefIterEncoder<'a, S, T>
where
    for<'b> &'b S: IntoIterator<Item = &'b T>,
    T: Encode<Input<'a> = T> + Archive + Clone + 'a,
    T::Archived: ArchivedLayout,
{
    type Input = &'a S;

    fn input<Sink: StorageMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut Sink,
    ) -> Result<Poll<()>, ZebinError> {
        self.iter = Some(item.into_iter());
        self.poll_pending(sink)
    }

    fn poll_pending<Sink: StorageMut + ?Sized>(
        &mut self,
        sink: &mut Sink,
    ) -> Result<Poll<()>, ZebinError> {
        let iter = self.iter.as_mut().ok_or(ZebinError::SerializationError {
            pos: sink.pos(),
            message: "RefIterEncoder polled before input",
        })?;
        loop {
            if self.seq_encoder.poll_pending(sink)?.is_pending() {
                return Ok(Poll::Pending);
            }

            if self.seq_encoder.is_finished() {
                return Ok(Poll::Ready(()));
            }

            if !self.seq_encoder.is_finished() {
                if let Some(item) = iter.next() {
                    if self.seq_encoder.input(item.clone(), sink)?.is_pending() {
                        return Ok(Poll::Pending);
                    }
                } else {
                    if self.seq_encoder.finish_ref(sink)?.is_pending() {
                        return Ok(Poll::Pending);
                    }
                }
            }
        }
    }

    fn finish<Sink: StorageMut + ?Sized>(self, sink: &mut Sink) -> Result<Poll<()>, ZebinError> {
        self.seq_encoder.finish(sink)
    }
}

impl<T> Encode for [T]
where
    T: Encode + Archive + Clone,
    T::Archived: ArchivedLayout,
    for<'a> T: Encode<Input<'a> = T> + 'a,
{
    type Input<'a>
        = &'a [T]
    where
        Self: 'a;
    type Encoder<'a>
        = RefIterEncoder<'a, [T], T>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        RefIterEncoder::new()
    }
}

impl<T> MeasureBody for [T]
where
    T: MeasureBody,
    T: ArchivedLayout,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        // Sequence layout: each element is (1 byte marker) + optional alignment padding + body, ended with a 0-byte sentinel.
        let mut pos = 0usize;
        let alignment = <T as ArchivedLayout>::ALIGNMENT.get();
        let fixed = <T as ArchivedLayout>::FIXED_SIZE.is_some();
        let mut count = 0usize;
        for item in self.iter() {
            pos = pos
                .checked_add(1)
                .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
            if fixed {
                let pad = (alignment - (pos % alignment)) % alignment;
                pos = pos
                    .checked_add(pad)
                    .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
            }
            pos = pos
                .checked_add(item.measure_body()?)
                .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
            count += 1;
        }
        pos = pos
            .checked_add(1)
            .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
        // Block index overhead (only when count > chunk_size).
        if count > 64 {
            pos = pos
                .checked_add(measure_block_index_overhead(count, pos)?)
                .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
        }
        Ok(pos)
    }
}
