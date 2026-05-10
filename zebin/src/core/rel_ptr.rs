use core::marker::PhantomData;
use core::num::NonZeroI64;

use crate::error::ArchiveError;

/// Zebin core: relative pointer.
/// Stores a 64-bit signed offset from the pointer's own location.
#[repr(transparent)]
pub struct RelPtr<T> {
    offset: NonZeroI64,
    _phantom: PhantomData<T>,
}

impl<T> RelPtr<T> {
    /// Create a new relative pointer from a source address to a target address.
    #[inline]
    pub fn new(from: usize, to: usize) -> Result<Self, ArchiveError> {
        let offset = if to >= from {
            let diff = to - from;
            i64::try_from(diff).map_err(|_| ArchiveError::OffsetOutOfRange { pos: from })?
        } else {
            let diff = from - to;
            if diff > (i64::MAX as usize).wrapping_add(1) {
                return Err(ArchiveError::OffsetOutOfRange { pos: from });
            }
            (diff as i64).wrapping_neg()
        };

        let offset = NonZeroI64::new(offset).ok_or(ArchiveError::ZeroOffset { pos: from })?;

        Ok(Self {
            offset,
            _phantom: PhantomData,
        })
    }

    /// Return the stored relative offset.
    #[inline]
    pub fn offset(&self) -> i64 {
        self.offset.get()
    }

    /// Calculate the absolute pointer from the relative offset.
    ///
    /// # Safety
    /// The caller must ensure that the relative pointer is part of a valid
    /// buffer and the target address is within bounds.
    #[inline]
    pub unsafe fn as_ptr(&self) -> *const T {
        let current_addr = self as *const _ as isize;
        current_addr.wrapping_add(self.offset.get() as isize) as *const T
    }

    /// Access the value directly.
    ///
    /// # Safety
    /// Same as `as_ptr`.
    #[inline]
    pub unsafe fn as_ref(&self) -> Option<&T> {
        let ptr = unsafe { self.as_ptr() };
        unsafe { Some(&*ptr) }
    }
}
