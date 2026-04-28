use crate::{
    ZebinError,
    traits::{Archive, Serialize},
};
use alloc::{borrow::Cow, borrow::ToOwned, boxed::Box, rc::Rc, sync::Arc};

impl<T> Archive for Box<T>
where
    T: Archive,
{
    type Archived = T::Archived;
    type Resolver = T::Resolver;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        self.as_ref().resolve(archive_pos, resolver)
    }
}

impl<T> Serialize for Box<T>
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

impl<T> Archive for Rc<T>
where
    T: Archive,
{
    type Archived = T::Archived;
    type Resolver = T::Resolver;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        self.as_ref().resolve(archive_pos, resolver)
    }
}

impl<T> Serialize for Rc<T>
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

impl<T> Archive for Arc<T>
where
    T: Archive,
{
    type Archived = T::Archived;
    type Resolver = T::Resolver;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        self.as_ref().resolve(archive_pos, resolver)
    }
}

impl<T> Serialize for Arc<T>
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
    ) -> Result<Self::Archived, ZebinError> {
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
