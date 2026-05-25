use crate::prelude::*;
use alloc::{borrow::Cow, borrow::ToOwned, boxed::Box, rc::Rc, sync::Arc};
use core::{ops::Deref, task::Poll};

/// Resumable serialization state for deref-able containers like `Box`, `Rc`, `Arc`, `Cow`.
pub struct DerefEncoder<'a, E, I>
where
    E: Encoder<'a>,
    I: ?Sized,
{
    encoder: E,
    _phantom: core::marker::PhantomData<&'a I>,
}

impl<'a, E, I> DerefEncoder<'a, E, I>
where
    E: Encoder<'a>,
    I: ?Sized,
{
    pub fn new(encoder: E) -> Self {
        Self {
            encoder,
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<'a, E, I> Encoder<'a> for DerefEncoder<'a, E, I>
where
    E: Encoder<'a>,
    I: ?Sized + Deref,
    &'a <I as Deref>::Target: Into<E::Input>,
{
    type Input = &'a I;

    fn input<S: ByteSink + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.encoder.input((&**item).into(), sink)
    }

    fn poll_pending<S: ByteSink + ?Sized>(&mut self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        self.encoder.poll_pending(sink)
    }

    fn finish<S: ByteSink + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        self.encoder.finish(sink)
    }
}

impl<T: ?Sized> Archive for Box<T>
where
    T: Archive,
{
    type Archived = T::Archived;
}

impl<T: ?Sized> Encode for Box<T>
where
    T: Encode + Archive,
{
    type Encoder<'a>
        = DerefEncoder<'a, <T as Encode>::Encoder<'a>, Box<T>>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        Ok(DerefEncoder::new(self.as_ref().begin_encode()?))
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

impl<T: ?Sized> Archive for Rc<T>
where
    T: Archive,
{
    type Archived = T::Archived;
}

impl<T: ?Sized> Encode for Rc<T>
where
    T: Encode + Archive,
{
    type Encoder<'a>
        = DerefEncoder<'a, <T as Encode>::Encoder<'a>, Rc<T>>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        Ok(DerefEncoder::new(self.as_ref().begin_encode()?))
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

impl<T: ?Sized> Archive for Arc<T>
where
    T: Archive,
{
    type Archived = T::Archived;
}

impl<T: ?Sized> Encode for Arc<T>
where
    T: Encode + Archive,
{
    type Encoder<'a>
        = DerefEncoder<'a, <T as Encode>::Encoder<'a>, Arc<T>>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        Ok(DerefEncoder::new(self.as_ref().begin_encode()?))
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

impl<'a, B> Archive for Cow<'a, B>
where
    B: ?Sized + ToOwned + Archive,
{
    type Archived = B::Archived;
}

impl<'a, B> Encode for Cow<'a, B>
where
    B: ?Sized + ToOwned + Encode + Archive,
{
    type Encoder<'b>
        = DerefEncoder<'b, <B as Encode>::Encoder<'b>, Cow<'a, B>>
    where
        Self: 'b;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        Ok(DerefEncoder::new(self.as_ref().begin_encode()?))
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
