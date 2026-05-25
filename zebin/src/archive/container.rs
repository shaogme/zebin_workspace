use crate::{
    ZebinError,
    traits::{Archive, Encode, Restore},
};
use alloc::{borrow::Cow, borrow::ToOwned, boxed::Box, rc::Rc, sync::Arc};

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
        = <T as Encode>::Encoder<'a>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        self.as_ref().begin_encode()
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
        = <T as Encode>::Encoder<'a>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        self.as_ref().begin_encode()
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
        = <T as Encode>::Encoder<'a>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        self.as_ref().begin_encode()
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
        = <B as Encode>::Encoder<'b>
    where
        Self: 'b;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        self.as_ref().begin_encode()
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
