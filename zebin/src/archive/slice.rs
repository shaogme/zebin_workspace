#[cfg(feature = "alloc")]
use crate::io::ForwardSequenceStrategy;
use crate::prelude::*;
use core::num::NonZeroUsize;

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

impl<'a, A, const N: usize> Decode<'a> for [A; N]
where
    A: Decode<'a>,
{
    type View = [A::View; N];
    #[cfg(feature = "alloc")]
    type DecodeStrategy = ForwardSequenceStrategy;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let mut out = core::mem::MaybeUninit::<[A::View; N]>::uninit();
        let out_ptr = out.as_mut_ptr() as *mut A::View;
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

    fn validate<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
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
    items: Option<&'a [T; N]>,
    index: usize,
    current_encoder: Option<(<T as Encode>::Encoder<'a>, bool)>,
}

impl<'a, T, const N: usize> ArrayEncoder<'a, T, N>
where
    T: Encode + Archive + 'a,
{
    pub(crate) fn new() -> Self {
        Self {
            items: None,
            index: 0,
            current_encoder: None,
        }
    }
}

impl<'a, T, const N: usize> Encoder<'a> for ArrayEncoder<'a, T, N>
where
    T: Encode + Archive + 'a,
{
    type Input = &'a [T; N];

    fn input<Sink: ByteSink + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut Sink,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        self.items = Some(item);
        self.poll_pending(sink)
    }

    fn poll_pending<Sink: ByteSink + ?Sized>(
        &mut self,
        sink: &mut Sink,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        let items = self.items.ok_or(ZebinError::SerializationError {
            pos: sink.pos(),
            message: "ArrayEncoder polled before input",
        })?;
        while self.index < N {
            if self.current_encoder.is_none() {
                self.current_encoder = Some((T::encoder(), false));
            }
            let (encoder, started) = self
                .current_encoder
                .as_mut()
                .expect("array item encoder initialized above");

            let progress = if !*started {
                match encoder.input(&items[self.index], sink)? {
                    core::task::Poll::Pending => {
                        *started = true;
                        core::task::Poll::Pending
                    }
                    core::task::Poll::Ready(()) => core::task::Poll::Ready(()),
                }
            } else {
                encoder.poll_pending(sink)?
            };

            match progress {
                core::task::Poll::Pending => return Ok(core::task::Poll::Pending),
                core::task::Poll::Ready(()) => {
                    let (encoder, _) = self.current_encoder.take().expect("present");
                    let _ = encoder.finish(sink)?;
                    self.index += 1;
                }
            }
        }
        Ok(core::task::Poll::Ready(()))
    }

    fn finish<Sink: ByteSink + ?Sized>(
        self,
        _sink: &mut Sink,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        Ok(core::task::Poll::Ready(()))
    }
}

impl<T, const N: usize> Encode for [T; N]
where
    T: Encode + Archive,
{
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

impl<T> Archive for [T]
where
    T: Archive,
{
    type Archived = crate::archive::ArchivedIter<'static, T::Archived>;
}

impl<T> Encode for [T]
where
    T: Encode + Archive,
    T::Archived: ArchivedLayout,
    for<'b> &'b [T]: IntoIterator<Item = &'b T>,
    for<'b> <&'b [T] as IntoIterator>::IntoIter: ExactSizeIterator,
{
    type Encoder<'a>
        = crate::archive::IterEncoder<'a, [T], T>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        crate::archive::IterEncoder::new()
    }
}
