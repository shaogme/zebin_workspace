use core::{marker::PhantomData, num::NonZeroUsize};

use alloc::string::ToString;

use crate::{
    ZebinError,
    core::schema::{LayoutDirectory, LayoutView},
};

/// Validator for byte streams to ensure safety before access.
pub struct Validator<'a> {
    data: &'a [u8],
    layouts: Option<LayoutDirectory<'a>>,
    depth: usize,
    max_depth: usize,
}

pub struct DepthGuard<'v> {
    context: *mut Validator<'v>,
    _phantom: PhantomData<&'v mut Validator<'v>>,
}

impl<'v> Drop for DepthGuard<'v> {
    fn drop(&mut self) {
        unsafe {
            (*self.context).pop_depth();
        }
    }
}

impl<'a> Validator<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            layouts: None,
            depth: 0,
            max_depth: 256,
        }
    }

    pub fn with_layouts(data: &'a [u8], layouts: LayoutDirectory<'a>) -> Self {
        Self {
            data,
            layouts: Some(layouts),
            depth: 0,
            max_depth: 256,
        }
    }

    /// Enter a nested validation scope and automatically unwind on drop.
    pub fn enter(&mut self) -> Result<DepthGuard<'a>, ZebinError> {
        self.push_depth()?;
        Ok(DepthGuard {
            context: self as *mut _,
            _phantom: PhantomData,
        })
    }

    /// Check if a pointer and its associated size are within the buffer bounds.
    pub fn check_range(&self, ptr: *const u8, size: usize) -> Result<(), ZebinError> {
        if size == 0 {
            return Ok(());
        }

        let start = ptr as usize;
        let buffer_start = self.data.as_ptr() as usize;
        let buffer_end = buffer_start.checked_add(self.data.len()).ok_or_else(|| {
            ZebinError::ValidationError {
                message: "Buffer length overflow".to_string(),
                pos: 0,
            }
        })?;

        let end = start
            .checked_add(size)
            .ok_or_else(|| ZebinError::ValidationError {
                message: "Pointer range overflow".to_string(),
                pos: start.saturating_sub(buffer_start),
            })?;

        if start < buffer_start || end > buffer_end {
            return Err(ZebinError::ValidationError {
                message: "Pointer out of bounds".to_string(),
                pos: start.saturating_sub(buffer_start),
            });
        }
        Ok(())
    }

    /// Check if a pointer is properly aligned.
    pub fn check_alignment(
        &self,
        ptr: *const u8,
        alignment: NonZeroUsize,
    ) -> Result<(), ZebinError> {
        let alignment_value = alignment.get();
        let addr = ptr as usize;
        if !addr.is_multiple_of(alignment_value) {
            return Err(ZebinError::AlignmentError {
                expected: alignment,
                actual: unsafe { NonZeroUsize::new_unchecked(addr % alignment_value) },
                pos: addr.saturating_sub(self.data.as_ptr() as usize),
            });
        }
        Ok(())
    }

    /// Push to recursion depth.
    pub fn push_depth(&mut self) -> Result<(), ZebinError> {
        self.depth += 1;
        if self.depth > self.max_depth {
            self.depth = self.depth.saturating_sub(1);
            return Err(ZebinError::RecursionLimitExceeded);
        }
        Ok(())
    }

    /// Pop from recursion depth.
    pub fn pop_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Get the underlying data slice.
    pub fn data(&self) -> &[u8] {
        self.data
    }

    pub fn layout(&self, schema_id: u32) -> Result<LayoutView<'a>, ZebinError> {
        let layouts = self.layouts.ok_or_else(|| ZebinError::ValidationError {
            message: "Missing layout directory".to_string(),
            pos: 0,
        })?;
        layouts.lookup(schema_id)
    }
}
