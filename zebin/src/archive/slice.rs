use core::num::NonZeroUsize;

use crate::{
    core::rel_ptr::RelPtr,
    error::{AccessError, ArchiveError, ValidateError, ValidationPathSegment},
    traits::{Access, Archive, Layout, Validate},
    utils::num::{u32_to_usize, usize_to_u32},
    validation::context::ValidationContext,
};

/// Source of indexed sequence items.
pub trait SequenceSource<T> {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get(&self, index: usize) -> &T;
}

impl<T> SequenceSource<T> for [T] {
    fn len(&self) -> usize {
        <[T]>::len(self)
    }

    fn get(&self, index: usize) -> &T {
        &self[index]
    }
}

impl<T, const N: usize> SequenceSource<T> for [T; N] {
    fn len(&self) -> usize {
        N
    }

    fn get(&self, index: usize) -> &T {
        &self[index]
    }
}

/// An archived vector that uses a relative pointer.
#[repr(C)]
pub struct ArchivedVec<T> {
    pub(crate) ptr: Option<RelPtr<T>>,
    pub(crate) len: u32,
}

impl<T> ArchivedVec<T> {
    /// Access the archived vector as a slice.
    ///
    /// # Safety
    /// The caller must ensure the pointer and length are valid.
    pub unsafe fn as_slice(&self) -> &[T] {
        if self.len == 0 {
            return &[];
        }
        let len = u32_to_usize(self.len, || ValidateError::ValidationError {
            message: "ArchivedVec length exceeds usize range",
            pos: self as *const _ as usize,
            path: Default::default(),
        })
        .expect("validated archived vector length should fit in usize");
        let ptr = self
            .ptr
            .as_ref()
            .expect("non-empty archived vector must have a pointer");
        unsafe { core::slice::from_raw_parts(ptr.as_ptr(), len) }
    }

    /// Get the length of the vector.
    pub fn len(&self) -> usize {
        u32_to_usize(self.len, || ValidateError::ValidationError {
            message: "ArchivedVec length exceeds usize range",
            pos: self as *const _ as usize,
            path: Default::default(),
        })
        .expect("archived vector length should fit in usize")
    }

    /// Check if the vector is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<T> Layout for ArchivedVec<T>
where
    T: Layout,
{
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(8).unwrap();

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        crate::utils::byteops::fill(out, 0);
        if let Some(ptr) = &archived.ptr {
            out[0..8].copy_from_slice(&ptr.offset().to_le_bytes());
        }
        <u32 as Layout>::write_archived_bytes(&archived.len, &mut out[8..12]);
    }
}

impl<T> Validate for ArchivedVec<T>
where
    T: Layout + Validate,
{
    unsafe fn validate<H, C>(ptr: *const Self, context: &mut C) -> Result<(), ValidateError>
    where
        H: crate::traits::ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let mut guard = context.guard()?;
        guard.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        guard.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let archived = unsafe { &*ptr };

        let len = u32_to_usize(archived.len, || {
            guard.validation_error("ArchivedVec length exceeds usize range", ptr as usize)
        })?;
        if len > 0 {
            let data_ptr = archived.ptr.as_ref().ok_or_else(|| {
                guard.validation_error("Null pointer in non-empty ArchivedVec", ptr as usize)
            })?;
            let data_ptr = unsafe { data_ptr.as_ptr() };
            let total_size = len
                .checked_mul(core::mem::size_of::<T>())
                .ok_or_else(|| guard.validation_error("ArchivedVec size overflow", ptr as usize))?;
            guard.check_range(data_ptr as *const u8, total_size)?;
            guard.check_alignment(data_ptr as *const u8, T::ALIGNMENT)?;

            for i in 0..len {
                let element_ptr = unsafe { data_ptr.add(i) };
                unsafe {
                    T::validate::<H, _>(element_ptr, &mut *guard)
                        .map_err(|e| e.at(ValidationPathSegment::Index(i)))?;
                }
            }
        }

        Ok(())
    }
}

impl<'a, T: 'a> Access<'a> for ArchivedVec<T>
where
    T: Layout + Validate,
{
    type View = &'a Self;

    unsafe fn access<H, C>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), AccessError>
    where
        H: crate::traits::ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let typed_ptr = ptr as *const Self;
        unsafe {
            <Self as Validate>::validate::<H, C>(typed_ptr, context)?;
        }
        Ok((unsafe { &*typed_ptr }, core::mem::size_of::<Self>()))
    }
}

pub(crate) fn resolve_sequence_archive<S, T>(
    source: &S,
    archive_pos: usize,
    resolver: usize,
) -> Result<ArchivedVec<T::Archived>, ArchiveError>
where
    S: ?Sized + SequenceSource<T>,
    T: Archive,
{
    let ptr = if source.len() == 0 {
        None
    } else {
        Some(RelPtr::new(archive_pos, resolver)?)
    };
    Ok(ArchivedVec {
        ptr,
        len: usize_to_u32(source.len(), || ArchiveError::LengthOverflow {
            pos: archive_pos,
        })?,
    })
}

impl<T> Archive for [T]
where
    T: Archive,
{
    type Archived = ArchivedVec<T::Archived>;
    type Resolver = usize;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ArchiveError> {
        resolve_sequence_archive(self, archive_pos, resolver)
    }
}

impl<T, const N: usize> Archive for [T; N]
where
    T: Archive,
{
    type Archived = [T::Archived; N];
    type Resolver = [T::Resolver; N];

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ArchiveError> {
        let elem_size = core::mem::size_of::<T::Archived>();
        let mut out = core::mem::MaybeUninit::<[T::Archived; N]>::uninit();
        let out_ptr = out.as_mut_ptr() as *mut T::Archived;
        let mut resolver_iter = resolver.into_iter();

        for (index, item) in self.iter().enumerate() {
            let item_resolver = resolver_iter.next().expect("array resolver length matches");
            let item_pos = archive_pos
                .checked_add(
                    index
                        .checked_mul(elem_size)
                        .ok_or(ArchiveError::ArithmeticOverflow { pos: archive_pos })?,
                )
                .ok_or(ArchiveError::ArithmeticOverflow { pos: archive_pos })?;
            let item = item.resolve(item_pos, item_resolver)?;
            unsafe {
                out_ptr.add(index).write(item);
            }
        }

        Ok(unsafe { out.assume_init() })
    }
}
