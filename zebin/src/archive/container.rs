use crate::prelude::*;
use alloc::{borrow::Cow, borrow::ToOwned, boxed::Box, rc::Rc, sync::Arc};
use core::task::Poll;

/// Encoder for `Box<T>`: takes ownership of the box, dereferences it once on
/// `input` to recover the inner `T`, and forwards into `T::Encoder`.
pub struct BoxEncoder<'a, T>
where
    T: Encode + Archive + 'a,
{
    inner: <T as Encode>::Encoder<'a>,
}

impl<'a, T> BoxEncoder<'a, T>
where
    T: Encode + Archive + 'a,
{
    pub fn new() -> Self {
        Self {
            inner: T::encoder(),
        }
    }
}

impl<'a, T> Default for BoxEncoder<'a, T>
where
    T: Encode + Archive + 'a,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, T> Encoder for BoxEncoder<'a, T>
where
    T: Encode<Input<'a> = T> + Archive + 'a,
{
    type Input = Box<T>;

    fn input<S: StorageMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let inner: T = *item;
        self.inner.input(inner, sink)
    }

    fn poll_pending<S: StorageMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.inner.poll_pending(sink)
    }

    fn finish<S: StorageMut + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        self.inner.finish(sink)
    }
}

impl<T> Archive for Box<T>
where
    T: Archive + ?Sized,
{
    type Archived = T::Archived;
}

impl<T> Encode for Box<T>
where
    T: Encode + Archive,
    for<'a> T: Encode<Input<'a> = T> + 'a,
{
    type Input<'a>
        = Box<T>
    where
        Self: 'a;
    type Encoder<'a>
        = BoxEncoder<'a, T>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        BoxEncoder::new()
    }
}

impl<T> MeasureBody for Box<T>
where
    T: MeasureBody + ?Sized,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        (**self).measure_body()
    }
}

/// `Box<str>`: encodes the inner `&str` (DST path).
pub struct BoxStrEncoder<'a> {
    inner: <str as Encode>::Encoder<'a>,
    pending: Option<Box<str>>,
}

impl<'a> BoxStrEncoder<'a> {
    pub fn new() -> Self {
        Self {
            inner: <str as Encode>::encoder(),
            pending: None,
        }
    }
}

impl<'a> Default for BoxStrEncoder<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Encoder for BoxStrEncoder<'a> {
    type Input = Box<str>;

    fn input<S: StorageMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        // SAFETY: we keep `pending` alive while `inner` borrows from it.
        // The `'a` lifetime in `<str as Encode>::Encoder<'a>` is satisfied because
        // we drop the box only after the encoder finishes.
        self.pending = Some(item);
        let s_ref: &str = self.pending.as_ref().unwrap();
        let s_ref: &'a str = unsafe { core::mem::transmute::<&str, &'a str>(s_ref) };
        self.inner.input(s_ref, sink)
    }

    fn poll_pending<S: StorageMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.inner.poll_pending(sink)
    }

    fn finish<S: StorageMut + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        self.inner.finish(sink)
    }
}

impl Encode for Box<str> {
    type Input<'a>
        = Box<str>
    where
        Self: 'a;
    type Encoder<'a>
        = BoxStrEncoder<'a>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        BoxStrEncoder::new()
    }
}

impl<A, T> Restore<Box<T>> for A
where
    T: Sized,
    A: Restore<T>,
{
    fn restore(&self) -> Result<Box<T>, ZebinError> {
        Ok(Box::new(self.restore()?))
    }
}

impl<A> Restore<Box<str>> for A
where
    A: Restore<alloc::string::String>,
{
    fn restore(&self) -> Result<Box<str>, ZebinError> {
        Ok(self.restore()?.into_boxed_str())
    }
}

impl<A, T> Restore<Box<[T]>> for A
where
    T: Clone,
    A: Restore<alloc::vec::Vec<T>>,
{
    fn restore(&self) -> Result<Box<[T]>, ZebinError> {
        Ok(self.restore()?.into_boxed_slice())
    }
}

/// Encoder for shared-ownership wrappers (`Rc<T>` / `Arc<T>` / `Cow<'_, T>`).
///
/// These wrappers are moved by value into the encoder. The inner `T` is
/// extracted (via `Clone` for `Rc`/`Arc`, `into_owned` for `Cow`) and fed into
/// `T`'s encoder.
pub struct SharedRefEncoder<'a, T, W>
where
    T: Encode + Archive + 'a,
{
    inner: <T as Encode>::Encoder<'a>,
    _phantom: core::marker::PhantomData<W>,
}

impl<'a, T, W> SharedRefEncoder<'a, T, W>
where
    T: Encode + Archive + 'a,
{
    pub fn new() -> Self {
        Self {
            inner: T::encoder(),
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<'a, T, W> Default for SharedRefEncoder<'a, T, W>
where
    T: Encode + Archive + 'a,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, T> Encoder for SharedRefEncoder<'a, T, Rc<T>>
where
    T: Encode<Input<'a> = T> + Archive + Clone + 'a,
{
    type Input = Rc<T>;

    fn input<S: StorageMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let value: T = (*item).clone();
        self.inner.input(value, sink)
    }

    fn poll_pending<S: StorageMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.inner.poll_pending(sink)
    }

    fn finish<S: StorageMut + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        self.inner.finish(sink)
    }
}

impl<'a, T> Encoder for SharedRefEncoder<'a, T, Arc<T>>
where
    T: Encode<Input<'a> = T> + Archive + Clone + 'a,
{
    type Input = Arc<T>;

    fn input<S: StorageMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let value: T = (*item).clone();
        self.inner.input(value, sink)
    }

    fn poll_pending<S: StorageMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.inner.poll_pending(sink)
    }

    fn finish<S: StorageMut + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        self.inner.finish(sink)
    }
}

/// Forwards an owned `Cow<B>` into `B::Owned`'s encoder via `into_owned()`.
pub struct CowEncoder<'r, 'cow, B: ?Sized>
where
    B: ToOwned + Encode + Archive,
    <B as ToOwned>::Owned: Encode + Archive + 'r,
{
    inner: <<B as ToOwned>::Owned as Encode>::Encoder<'r>,
    _phantom: core::marker::PhantomData<&'cow B>,
}

impl<'r, 'cow, B: ?Sized> CowEncoder<'r, 'cow, B>
where
    B: ToOwned + Encode + Archive,
    <B as ToOwned>::Owned: Encode + Archive + 'r,
{
    pub fn new() -> Self {
        Self {
            inner: <<B as ToOwned>::Owned as Encode>::encoder(),
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<'r, 'cow, B: ?Sized> Default for CowEncoder<'r, 'cow, B>
where
    B: ToOwned + Encode + Archive,
    <B as ToOwned>::Owned: Encode + Archive + 'r,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<'r, 'cow, B: ?Sized> Encoder for CowEncoder<'r, 'cow, B>
where
    B: ToOwned + Encode + Archive,
    <B as ToOwned>::Owned: Encode<Input<'r> = <B as ToOwned>::Owned> + Archive + 'r,
{
    type Input = Cow<'cow, B>;

    fn input<S: StorageMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let owned = item.into_owned();
        self.inner.input(owned, sink)
    }

    fn poll_pending<S: StorageMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.inner.poll_pending(sink)
    }

    fn finish<S: StorageMut + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        self.inner.finish(sink)
    }
}

impl<T> Archive for Rc<T>
where
    T: Archive,
{
    type Archived = T::Archived;
}

impl<T> Encode for Rc<T>
where
    T: Encode + Archive + Clone,
    for<'a> T: Encode<Input<'a> = T> + 'a,
{
    type Input<'a>
        = Rc<T>
    where
        Self: 'a;
    type Encoder<'a>
        = SharedRefEncoder<'a, T, Rc<T>>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        SharedRefEncoder::new()
    }
}

impl<T> MeasureBody for Rc<T>
where
    T: MeasureBody + ?Sized,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        (**self).measure_body()
    }
}

impl<A, T> Restore<Rc<T>> for A
where
    T: Sized,
    A: Restore<T>,
{
    fn restore(&self) -> Result<Rc<T>, ZebinError> {
        Ok(Rc::new(self.restore()?))
    }
}

impl<A> Restore<Rc<str>> for A
where
    A: Restore<alloc::string::String>,
{
    fn restore(&self) -> Result<Rc<str>, ZebinError> {
        Ok(self.restore()?.into())
    }
}

impl<A, T> Restore<Rc<[T]>> for A
where
    T: Clone,
    A: Restore<alloc::vec::Vec<T>>,
{
    fn restore(&self) -> Result<Rc<[T]>, ZebinError> {
        Ok(self.restore()?.into())
    }
}

impl<T> Archive for Arc<T>
where
    T: Archive,
{
    type Archived = T::Archived;
}

impl<T> Encode for Arc<T>
where
    T: Encode + Archive + Clone,
    for<'a> T: Encode<Input<'a> = T> + 'a,
{
    type Input<'a>
        = Arc<T>
    where
        Self: 'a;
    type Encoder<'a>
        = SharedRefEncoder<'a, T, Arc<T>>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        SharedRefEncoder::new()
    }
}

impl<T> MeasureBody for Arc<T>
where
    T: MeasureBody + ?Sized,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        (**self).measure_body()
    }
}

impl<A, T> Restore<Arc<T>> for A
where
    T: Sized,
    A: Restore<T>,
{
    fn restore(&self) -> Result<Arc<T>, ZebinError> {
        Ok(Arc::new(self.restore()?))
    }
}

impl<A> Restore<Arc<str>> for A
where
    A: Restore<alloc::string::String>,
{
    fn restore(&self) -> Result<Arc<str>, ZebinError> {
        Ok(self.restore()?.into())
    }
}

impl<A, T> Restore<Arc<[T]>> for A
where
    T: Clone,
    A: Restore<alloc::vec::Vec<T>>,
{
    fn restore(&self) -> Result<Arc<[T]>, ZebinError> {
        Ok(self.restore()?.into())
    }
}

impl<'cow, B> Archive for Cow<'cow, B>
where
    B: ?Sized + ToOwned + Archive,
{
    type Archived = B::Archived;
}

impl<'cow, B> Encode for Cow<'cow, B>
where
    B: ?Sized + ToOwned + Encode + Archive + 'cow,
    <B as ToOwned>::Owned: Encode + Archive,
    for<'r> <B as ToOwned>::Owned: Encode<Input<'r> = <B as ToOwned>::Owned> + 'r,
{
    type Input<'r>
        = Cow<'cow, B>
    where
        Self: 'r;
    type Encoder<'r>
        = CowEncoder<'r, 'cow, B>
    where
        Self: 'r;

    fn encoder<'r>() -> Self::Encoder<'r>
    where
        Self: 'r,
    {
        CowEncoder::new()
    }
}

impl<'cow, B> MeasureBody for Cow<'cow, B>
where
    B: ?Sized + ToOwned + MeasureBody,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        (**self).measure_body()
    }
}

impl<'a, A, T> Restore<Cow<'a, T>> for A
where
    T: ToOwned + Archive + ?Sized,
    A: Restore<T::Owned>,
{
    fn restore(&self) -> Result<Cow<'a, T>, ZebinError> {
        Ok(Cow::Owned(self.restore()?))
    }
}
