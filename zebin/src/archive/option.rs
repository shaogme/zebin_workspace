use core::{mem::MaybeUninit, num::NonZeroUsize, task::Poll};

use crate::{
    error::{AccessError, ArchiveError, ValidateError, ZebinError},
    read::ResolvedLayout,
    traits::{
        Access, Archive, ArchiveHeader, ArchivedDefault, ByteSink, Layout, LayoutSink, OptRestorer,
        OptRestorerOption, Restore, RestoreFromView, Serialize, SerializeState, Validate,
    },
    utils::byteops,
    validation::context::ValidationContext,
};

/// Archived representation for `Option<T>`.
#[repr(C)]
pub struct ArchivedOption<T> {
    tag: u8,
    value: MaybeUninit<T>,
}

impl<T> ArchivedOption<T> {
    pub fn is_some(&self) -> bool {
        self.tag == 1
    }

    pub fn is_none(&self) -> bool {
        self.tag == 0
    }

    /// Returns the archived payload if this option is `Some`.
    ///
    /// # Safety
    /// The caller must ensure the archived option is valid.
    pub unsafe fn as_ref(&self) -> Option<&T> {
        match self.tag {
            0 => None,
            1 => Some(unsafe { self.value.assume_init_ref() }),
            _ => None,
        }
    }
}

impl<T> Layout for ArchivedOption<T>
where
    T: Layout,
{
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        byteops::fill(out, 0);
        out[0] = archived.tag;
        if archived.tag == 1 {
            let value_offset = crate::memoffset::offset_of!(ArchivedOption<T>, value);
            let value = unsafe { archived.value.assume_init_ref() };
            T::write_archived_bytes(
                value,
                &mut out[value_offset..value_offset + core::mem::size_of::<T>()],
            );
        }
    }
}

impl<T> Validate for ArchivedOption<T>
where
    T: Layout + Validate,
{
    unsafe fn validate<H, C>(ptr: *const Self, context: &mut C) -> Result<(), ValidateError>
    where
        H: ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let mut guard = context.guard()?;
        guard.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        guard.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let archived = unsafe { &*ptr };
        match archived.tag {
            0 => Ok(()),
            1 => {
                let value_ptr = archived.value.as_ptr();
                guard.check_alignment(value_ptr as *const u8, T::ALIGNMENT)?;
                guard.check_range(value_ptr as *const u8, core::mem::size_of::<T>())?;
                {
                    let mut _path_guard = guard.push_variant("Some");
                    unsafe {
                        T::validate::<H, _>(value_ptr, &mut *_path_guard)?;
                    }
                }
                Ok(())
            }
            _ => Err(guard.validation_error("Invalid Option discriminant", ptr as usize)),
        }
    }
}

impl<'a, T: 'a> Access<'a> for ArchivedOption<T>
where
    T: Layout + Validate,
{
    type View = &'a Self;

    unsafe fn access<H, C>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), AccessError>
    where
        H: ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let typed_ptr = ptr as *const Self;
        unsafe {
            <Self as Validate>::validate::<H, C>(typed_ptr, context)?;
        }
        Ok((unsafe { &*typed_ptr }, core::mem::size_of::<Self>()))
    }
}

impl<T: 'static> ArchivedDefault for ArchivedOption<T> {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchivedOption<()> = ArchivedOption {
            tag: 0,
            value: MaybeUninit::uninit(),
        };
        unsafe { &*(&DEFAULT as *const ArchivedOption<()> as *const ArchivedOption<T>) }
    }
}

impl<T, U> Restore<Option<U>> for ArchivedOption<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<Option<U>, ZebinError> {
        match unsafe { self.as_ref() } {
            Some(value) => Ok(Some(value.restore()?)),
            None => Ok(None),
        }
    }
}

impl<'a, T, U, H: ArchiveHeader> RestoreFromView<'a, Option<U>, H> for ArchivedOption<T>
where
    T: for<'b> RestoreFromView<'b, U, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<Option<U>, ZebinError> {
        match unsafe { self.as_ref() } {
            Some(value) => {
                let item_layout = crate::read::get_nested_layout(layout, value)?;
                Ok(Some(value.restore_from_view(&item_layout)?))
            }
            None => Ok(None),
        }
    }
}

impl<T, U> Restore<Option<U>> for Option<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<Option<U>, ZebinError> {
        match self {
            Some(v) => Ok(Some(v.restore()?)),
            None => Ok(None),
        }
    }
}

impl<'a, T, U, H: ArchiveHeader> RestoreFromView<'a, Option<U>, H> for Option<&'a T>
where
    T: RestoreFromView<'a, U, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<Option<U>, ZebinError> {
        match *self {
            Some(val) => {
                let item_layout = crate::read::get_nested_layout(layout, val)?;
                Ok(Some(val.restore_from_view(&item_layout)?))
            }
            None => Ok(None),
        }
    }
}

/// Resumable serialization state for `Option<T>`.
pub struct OptionArchiveState<'a, T>
where
    T: Serialize + Archive + 'a,
{
    inner: Option<<T as Serialize>::State<'a>>,
}

impl<'a, T> OptionArchiveState<'a, T>
where
    T: Serialize + Archive + 'a,
{
    fn new(value: Option<&'a T>) -> Result<Self, ZebinError> {
        match value {
            Some(inner) => Ok(Self {
                inner: Some(inner.begin_serialize()?),
            }),
            None => Ok(Self { inner: None }),
        }
    }
}

impl<'a, T> SerializeState<'a> for OptionArchiveState<'a, T>
where
    T: Serialize + Archive + 'a,
{
    type Resolver = Option<T::Resolver>;

    fn poll<E: ByteSink + LayoutSink<'a> + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        if self.inner.is_none() {
            return Ok(Poll::Ready(None));
        }

        match self
            .inner
            .as_mut()
            .expect("inner state initialized for Some option")
            .poll(encoder)?
        {
            Poll::Pending => Ok(Poll::Pending),
            Poll::Ready(resolver) => {
                self.inner = None;
                Ok(Poll::Ready(Some(resolver)))
            }
        }
    }
}

impl<T> Archive for Option<T>
where
    T: Archive,
{
    type Archived = ArchivedOption<T::Archived>;
    type Resolver = Option<T::Resolver>;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ArchiveError> {
        match (self, resolver) {
            (Some(value), Some(resolver)) => {
                let value_offset = crate::memoffset::offset_of!(ArchivedOption<T::Archived>, value);
                let archived = value.resolve(archive_pos + value_offset, resolver)?;
                Ok(ArchivedOption {
                    tag: 1,
                    value: MaybeUninit::new(archived),
                })
            }
            (None, None) => Ok(ArchivedOption {
                tag: 0,
                value: MaybeUninit::uninit(),
            }),
            _ => Err(ArchiveError::InvalidResolver { pos: archive_pos }),
        }
    }
}

impl<T> Serialize for Option<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = OptionArchiveState<'a, T>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        OptionArchiveState::new(self.as_ref())
    }
}

impl<'a, A, T, H: ArchiveHeader> OptRestorerOption<'a, T, H>
    for OptRestorer<'a, ArchivedOption<A>, H>
where
    ArchivedOption<A>: RestoreFromView<'a, Option<T>, H> + Layout,
{
    fn restore(self) -> Result<Option<T>, ZebinError> {
        match self.data {
            Some(archived) => {
                let nested = crate::read::get_nested_layout(self.layout, archived)?;
                archived.restore_from_view(&nested)
            }
            None => Ok(None),
        }
    }
}
