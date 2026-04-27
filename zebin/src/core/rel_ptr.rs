use std::marker::PhantomData;
use std::num::NonZeroI64;

use crate::traits::ZebinError;

/// Zebin core: relative pointer.
/// Stores a 64-bit signed offset from the pointer's own location.
#[repr(transparent)]
pub struct RelPtr<T> {
    offset: NonZeroI64,
    _phantom: PhantomData<T>,
}

impl<T> RelPtr<T> {
    /// Create a new relative pointer from a source address to a target address.
    pub fn new(from: usize, to: usize) -> Result<Self, ZebinError> {
        let offset = to as i128 - from as i128;
        let offset = i64::try_from(offset).map_err(|_| ZebinError::ValidationError {
            message: "relative pointer offset out of range".to_string(),
            pos: from,
        })?;
        let offset = NonZeroI64::new(offset).ok_or_else(|| ZebinError::ValidationError {
            message: "relative pointer offset cannot be zero".to_string(),
            pos: from,
        })?;

        Ok(Self {
            offset,
            _phantom: PhantomData,
        })
    }

    /// Return the stored relative offset.
    pub fn offset(&self) -> i64 {
        self.offset.get()
    }

    /// Calculate the absolute pointer from the relative offset.
    ///
    /// # Safety
    /// The caller must ensure that the relative pointer is part of a valid
    /// buffer and the target address is within bounds.
    pub unsafe fn as_ptr(&self) -> *const T {
        let current_addr = self as *const _ as isize;
        current_addr
            .checked_add(self.offset.get() as isize)
            .expect("relative pointer offset out of range") as *const T
    }

    /// Access the value directly.
    ///
    /// # Safety
    /// Same as `as_ptr`.
    pub unsafe fn as_ref(&self) -> Option<&T> {
        let ptr = unsafe { self.as_ptr() };
        unsafe { Some(&*ptr) }
    }
}
