use crate::{
    ZebinError,
    error::ArchiveError,
    read::ResolvedLayout,
    traits::{Archive, ArchiveHeader, Layout, Restore, RestoreFromView, Serialize},
};
use alloc::{borrow::Cow, borrow::ToOwned, boxed::Box, rc::Rc, sync::Arc};

impl<T: ?Sized> Archive for Box<T>
where
    T: Archive,
{
    type Archived = T::Archived;
    type Resolver = T::Resolver;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ArchiveError> {
        self.as_ref().resolve(archive_pos, resolver)
    }
}

impl<T: ?Sized> Serialize for Box<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = <T as Serialize>::State<'a>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        self.as_ref().begin_serialize()
    }
}

impl<A, T> Restore<Box<T>> for A
where
    T: Sized,
    A: Restore<T> + Layout,
{
    fn restore(&self) -> Result<Box<T>, ZebinError> {
        Ok(Box::new(self.restore()?))
    }
}

impl<'a, A, T, H: ArchiveHeader> RestoreFromView<'a, Box<T>, H> for A
where
    T: Sized,
    A: RestoreFromView<'a, T, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<Box<T>, ZebinError> {
        Ok(Box::new(self.restore_from_view(layout)?))
    }
}

impl<A> Restore<Box<str>> for A
where
    A: Restore<String> + Layout,
{
    fn restore(&self) -> Result<Box<str>, ZebinError> {
        Ok(self.restore()?.into_boxed_str())
    }
}

impl<'a, A, H: ArchiveHeader> RestoreFromView<'a, Box<str>, H> for A
where
    A: RestoreFromView<'a, String, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<Box<str>, ZebinError> {
        Ok(self.restore_from_view(layout)?.into_boxed_str())
    }
}

impl<A, T> Restore<Box<[T]>> for A
where
    T: Clone,
    A: Restore<Vec<T>> + Layout,
{
    fn restore(&self) -> Result<Box<[T]>, ZebinError> {
        Ok(self.restore()?.into_boxed_slice())
    }
}

impl<'a, A, T, H: ArchiveHeader> RestoreFromView<'a, Box<[T]>, H> for A
where
    T: Clone,
    A: RestoreFromView<'a, Vec<T>, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<Box<[T]>, ZebinError> {
        Ok(self.restore_from_view(layout)?.into_boxed_slice())
    }
}

impl<T: ?Sized> Archive for Rc<T>
where
    T: Archive,
{
    type Archived = T::Archived;
    type Resolver = T::Resolver;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ArchiveError> {
        self.as_ref().resolve(archive_pos, resolver)
    }
}

impl<T: ?Sized> Serialize for Rc<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = <T as Serialize>::State<'a>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        self.as_ref().begin_serialize()
    }
}

impl<A, T> Restore<Rc<T>> for A
where
    T: Sized,
    A: Restore<T> + Layout,
{
    fn restore(&self) -> Result<Rc<T>, ZebinError> {
        Ok(Rc::new(self.restore()?))
    }
}

impl<'a, A, T, H: ArchiveHeader> RestoreFromView<'a, Rc<T>, H> for A
where
    T: Sized,
    A: RestoreFromView<'a, T, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<Rc<T>, ZebinError> {
        Ok(Rc::new(self.restore_from_view(layout)?))
    }
}

impl<A> Restore<Rc<str>> for A
where
    A: Restore<String> + Layout,
{
    fn restore(&self) -> Result<Rc<str>, ZebinError> {
        Ok(self.restore()?.into())
    }
}

impl<'a, A, H: ArchiveHeader> RestoreFromView<'a, Rc<str>, H> for A
where
    A: RestoreFromView<'a, String, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<Rc<str>, ZebinError> {
        Ok(self.restore_from_view(layout)?.into())
    }
}

impl<A, T> Restore<Rc<[T]>> for A
where
    T: Clone,
    A: Restore<Vec<T>> + Layout,
{
    fn restore(&self) -> Result<Rc<[T]>, ZebinError> {
        Ok(self.restore()?.into())
    }
}

impl<'a, A, T, H: ArchiveHeader> RestoreFromView<'a, Rc<[T]>, H> for A
where
    T: Clone,
    A: RestoreFromView<'a, Vec<T>, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<Rc<[T]>, ZebinError> {
        Ok(self.restore_from_view(layout)?.into())
    }
}

impl<T: ?Sized> Archive for Arc<T>
where
    T: Archive,
{
    type Archived = T::Archived;
    type Resolver = T::Resolver;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ArchiveError> {
        self.as_ref().resolve(archive_pos, resolver)
    }
}

impl<T: ?Sized> Serialize for Arc<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = <T as Serialize>::State<'a>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        self.as_ref().begin_serialize()
    }
}

impl<A, T> Restore<Arc<T>> for A
where
    T: Sized,
    A: Restore<T> + Layout,
{
    fn restore(&self) -> Result<Arc<T>, ZebinError> {
        Ok(Arc::new(self.restore()?))
    }
}

impl<'a, A, T, H: ArchiveHeader> RestoreFromView<'a, Arc<T>, H> for A
where
    T: Sized,
    A: RestoreFromView<'a, T, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<Arc<T>, ZebinError> {
        Ok(Arc::new(self.restore_from_view(layout)?))
    }
}

impl<A> Restore<Arc<str>> for A
where
    A: Restore<String> + Layout,
{
    fn restore(&self) -> Result<Arc<str>, ZebinError> {
        Ok(self.restore()?.into())
    }
}

impl<'a, A, H: ArchiveHeader> RestoreFromView<'a, Arc<str>, H> for A
where
    A: RestoreFromView<'a, String, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<Arc<str>, ZebinError> {
        Ok(self.restore_from_view(layout)?.into())
    }
}

impl<A, T> Restore<Arc<[T]>> for A
where
    T: Clone,
    A: Restore<Vec<T>> + Layout,
{
    fn restore(&self) -> Result<Arc<[T]>, ZebinError> {
        Ok(self.restore()?.into())
    }
}

impl<'a, A, T, H: ArchiveHeader> RestoreFromView<'a, Arc<[T]>, H> for A
where
    T: Clone,
    A: RestoreFromView<'a, Vec<T>, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<Arc<[T]>, ZebinError> {
        Ok(self.restore_from_view(layout)?.into())
    }
}

impl<'a, B> Archive for Cow<'a, B>
where
    B: ?Sized + ToOwned + Archive,
{
    type Archived = B::Archived;
    type Resolver = B::Resolver;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ArchiveError> {
        self.as_ref().resolve(archive_pos, resolver)
    }
}

impl<'a, B> Serialize for Cow<'a, B>
where
    B: ?Sized + ToOwned + Serialize + Archive,
{
    type State<'b>
        = <B as Serialize>::State<'b>
    where
        Self: 'b;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        self.as_ref().begin_serialize()
    }
}

impl<'a, A, T> Restore<Cow<'a, T>> for A
where
    T: ToOwned + Archive + ?Sized,
    A: Restore<T::Owned> + Layout,
{
    fn restore(&self) -> Result<Cow<'a, T>, ZebinError> {
        Ok(Cow::Owned(self.restore()?))
    }
}

impl<'a, A, T, H: ArchiveHeader> RestoreFromView<'a, Cow<'a, T>, H> for A
where
    T: ToOwned + Archive + ?Sized,
    A: RestoreFromView<'a, T::Owned, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<Cow<'a, T>, ZebinError> {
        Ok(Cow::Owned(self.restore_from_view(layout)?))
    }
}
