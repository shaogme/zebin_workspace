use crate::prelude::*;
use alloc::{borrow::Cow, borrow::ToOwned, boxed::Box, rc::Rc, sync::Arc};
use core::task::Poll;

/// Serializer for `Box<T>`: takes ownership of the box, dereferences it once on
/// `input` to recover the inner `T`, and forwards into `T::Serializer`.
pub struct BoxSerializer<'a, T>
where
    T: Serialize + Archive + 'a,
{
    inner: <T as Serialize>::Serializer<'a>,
}

impl<'a, T> BoxSerializer<'a, T>
where
    T: Serialize + Archive + 'a,
{
    pub fn new() -> Self {
        Self {
            inner: T::serializer(),
        }
    }
}

impl<'a, T> Default for BoxSerializer<'a, T>
where
    T: Serialize + Archive + 'a,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, T> Serializer for BoxSerializer<'a, T>
where
    T: Serialize<Input<'a> = T> + Archive + 'a,
{
    type Input = Box<T>;

    fn input<S: CursorMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let inner: T = *item;
        self.inner.input(inner, sink)
    }

    fn poll_pending<S: CursorMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.inner.poll_pending(sink)
    }

    fn finish<S: CursorMut + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        self.inner.finish(sink)
    }
}

impl<T> Archive for Box<T>
where
    T: Archive + ?Sized,
{
    type Archived = T::Archived;
}

impl<T> Serialize for Box<T>
where
    T: Serialize + Archive,
    for<'a> T: Serialize<Input<'a> = T> + 'a,
{
    type Input<'a>
        = Box<T>
    where
        Self: 'a;
    type Serializer<'a>
        = BoxSerializer<'a, T>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        BoxSerializer::new()
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

/// `Box<str>`: serializes the inner `&str` (DST path).
pub struct BoxStrSerializer<'a> {
    inner: <str as Serialize>::Serializer<'a>,
    pending: Option<Box<str>>,
}

impl<'a> BoxStrSerializer<'a> {
    pub fn new() -> Self {
        Self {
            inner: <str as Serialize>::serializer(),
            pending: None,
        }
    }
}

impl<'a> Default for BoxStrSerializer<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Serializer for BoxStrSerializer<'a> {
    type Input = Box<str>;

    fn input<S: CursorMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        // SAFETY: we keep `pending` alive while `inner` borrows from it.
        // The `'a` lifetime in `<str as Serialize>::Serializer<'a>` is satisfied because
        // we drop the box only after the serializer finishes.
        self.pending = Some(item);
        let s_ref: &str = self.pending.as_ref().unwrap();
        let s_ref: &'a str = unsafe { core::mem::transmute::<&str, &'a str>(s_ref) };
        self.inner.input(s_ref, sink)
    }

    fn poll_pending<S: CursorMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.inner.poll_pending(sink)
    }

    fn finish<S: CursorMut + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        self.inner.finish(sink)
    }
}

impl Serialize for Box<str> {
    type Input<'a>
        = Box<str>
    where
        Self: 'a;
    type Serializer<'a>
        = BoxStrSerializer<'a>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        BoxStrSerializer::new()
    }
}

impl<A, T> Deserialize<Box<T>> for A
where
    T: Sized,
    A: Deserialize<T>,
{
    fn deserialize(&self) -> Result<Box<T>, ZebinError> {
        Ok(Box::new(self.deserialize()?))
    }
}

impl<A> Deserialize<Box<str>> for A
where
    A: Deserialize<alloc::string::String>,
{
    fn deserialize(&self) -> Result<Box<str>, ZebinError> {
        Ok(self.deserialize()?.into_boxed_str())
    }
}

impl<A, T> Deserialize<Box<[T]>> for A
where
    T: Clone,
    A: Deserialize<alloc::vec::Vec<T>>,
{
    fn deserialize(&self) -> Result<Box<[T]>, ZebinError> {
        Ok(self.deserialize()?.into_boxed_slice())
    }
}

/// Serializer for shared-ownership wrappers (`Rc<T>` / `Arc<T>` / `Cow<'_, T>`).
///
/// These wrappers are moved by value into the serializer. The inner `T` is
/// extracted (via `Clone` for `Rc`/`Arc`, `into_owned` for `Cow`) and fed into
/// `T`'s serializer.
pub struct SharedRefSerializer<'a, T, W>
where
    T: Serialize + Archive + 'a,
{
    inner: <T as Serialize>::Serializer<'a>,
    _phantom: core::marker::PhantomData<W>,
}

impl<'a, T, W> SharedRefSerializer<'a, T, W>
where
    T: Serialize + Archive + 'a,
{
    pub fn new() -> Self {
        Self {
            inner: T::serializer(),
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<'a, T, W> Default for SharedRefSerializer<'a, T, W>
where
    T: Serialize + Archive + 'a,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, T> Serializer for SharedRefSerializer<'a, T, Rc<T>>
where
    T: Serialize<Input<'a> = T> + Archive + Clone + 'a,
{
    type Input = Rc<T>;

    fn input<S: CursorMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let value: T = (*item).clone();
        self.inner.input(value, sink)
    }

    fn poll_pending<S: CursorMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.inner.poll_pending(sink)
    }

    fn finish<S: CursorMut + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        self.inner.finish(sink)
    }
}

impl<'a, T> Serializer for SharedRefSerializer<'a, T, Arc<T>>
where
    T: Serialize<Input<'a> = T> + Archive + Clone + 'a,
{
    type Input = Arc<T>;

    fn input<S: CursorMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let value: T = (*item).clone();
        self.inner.input(value, sink)
    }

    fn poll_pending<S: CursorMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.inner.poll_pending(sink)
    }

    fn finish<S: CursorMut + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        self.inner.finish(sink)
    }
}

/// Forwards an owned `Cow<B>` into `B::Owned`'s serializer via `into_owned()`.
pub struct CowSerializer<'r, 'cow, B: ?Sized>
where
    B: ToOwned + Serialize + Archive,
    <B as ToOwned>::Owned: Serialize + Archive + 'r,
{
    inner: <<B as ToOwned>::Owned as Serialize>::Serializer<'r>,
    _phantom: core::marker::PhantomData<&'cow B>,
}

impl<'r, 'cow, B: ?Sized> CowSerializer<'r, 'cow, B>
where
    B: ToOwned + Serialize + Archive,
    <B as ToOwned>::Owned: Serialize + Archive + 'r,
{
    pub fn new() -> Self {
        Self {
            inner: <<B as ToOwned>::Owned as Serialize>::serializer(),
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<'r, 'cow, B: ?Sized> Default for CowSerializer<'r, 'cow, B>
where
    B: ToOwned + Serialize + Archive,
    <B as ToOwned>::Owned: Serialize + Archive + 'r,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<'r, 'cow, B: ?Sized> Serializer for CowSerializer<'r, 'cow, B>
where
    B: ToOwned + Serialize + Archive,
    <B as ToOwned>::Owned: Serialize<Input<'r> = <B as ToOwned>::Owned> + Archive + 'r,
{
    type Input = Cow<'cow, B>;

    fn input<S: CursorMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let owned = item.into_owned();
        self.inner.input(owned, sink)
    }

    fn poll_pending<S: CursorMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.inner.poll_pending(sink)
    }

    fn finish<S: CursorMut + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        self.inner.finish(sink)
    }
}

impl<T> Archive for Rc<T>
where
    T: Archive,
{
    type Archived = T::Archived;
}

impl<T> Serialize for Rc<T>
where
    T: Serialize + Archive + Clone,
    for<'a> T: Serialize<Input<'a> = T> + 'a,
{
    type Input<'a>
        = Rc<T>
    where
        Self: 'a;
    type Serializer<'a>
        = SharedRefSerializer<'a, T, Rc<T>>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        SharedRefSerializer::new()
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

impl<A, T> Deserialize<Rc<T>> for A
where
    T: Sized,
    A: Deserialize<T>,
{
    fn deserialize(&self) -> Result<Rc<T>, ZebinError> {
        Ok(Rc::new(self.deserialize()?))
    }
}

impl<A> Deserialize<Rc<str>> for A
where
    A: Deserialize<alloc::string::String>,
{
    fn deserialize(&self) -> Result<Rc<str>, ZebinError> {
        Ok(self.deserialize()?.into())
    }
}

impl<A, T> Deserialize<Rc<[T]>> for A
where
    T: Clone,
    A: Deserialize<alloc::vec::Vec<T>>,
{
    fn deserialize(&self) -> Result<Rc<[T]>, ZebinError> {
        Ok(self.deserialize()?.into())
    }
}

impl<T> Archive for Arc<T>
where
    T: Archive,
{
    type Archived = T::Archived;
}

impl<T> Serialize for Arc<T>
where
    T: Serialize + Archive + Clone,
    for<'a> T: Serialize<Input<'a> = T> + 'a,
{
    type Input<'a>
        = Arc<T>
    where
        Self: 'a;
    type Serializer<'a>
        = SharedRefSerializer<'a, T, Arc<T>>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        SharedRefSerializer::new()
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

impl<A, T> Deserialize<Arc<T>> for A
where
    T: Sized,
    A: Deserialize<T>,
{
    fn deserialize(&self) -> Result<Arc<T>, ZebinError> {
        Ok(Arc::new(self.deserialize()?))
    }
}

impl<A> Deserialize<Arc<str>> for A
where
    A: Deserialize<alloc::string::String>,
{
    fn deserialize(&self) -> Result<Arc<str>, ZebinError> {
        Ok(self.deserialize()?.into())
    }
}

impl<A, T> Deserialize<Arc<[T]>> for A
where
    T: Clone,
    A: Deserialize<alloc::vec::Vec<T>>,
{
    fn deserialize(&self) -> Result<Arc<[T]>, ZebinError> {
        Ok(self.deserialize()?.into())
    }
}

impl<'cow, B> Archive for Cow<'cow, B>
where
    B: ?Sized + ToOwned + Archive,
{
    type Archived = B::Archived;
}

impl<'cow, B> Serialize for Cow<'cow, B>
where
    B: ?Sized + ToOwned + Serialize + Archive + 'cow,
    <B as ToOwned>::Owned: Serialize + Archive,
    for<'r> <B as ToOwned>::Owned: Serialize<Input<'r> = <B as ToOwned>::Owned> + 'r,
{
    type Input<'r>
        = Cow<'cow, B>
    where
        Self: 'r;
    type Serializer<'r>
        = CowSerializer<'r, 'cow, B>
    where
        Self: 'r;

    fn serializer<'r>() -> Self::Serializer<'r>
    where
        Self: 'r,
    {
        CowSerializer::new()
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

impl<'a, A, T> Deserialize<Cow<'a, T>> for A
where
    T: ToOwned + Archive + ?Sized,
    A: Deserialize<T::Owned>,
{
    fn deserialize(&self) -> Result<Cow<'a, T>, ZebinError> {
        Ok(Cow::Owned(self.deserialize()?))
    }
}
