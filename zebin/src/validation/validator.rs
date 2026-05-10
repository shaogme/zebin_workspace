use core::{marker::PhantomData, num::NonZeroUsize};

use crate::{
    ZebinError,
    core::schema::{LayoutDirectory, SchemaRevision, StableSchemaKey},
    format::ArchiveHeader,
    read::ResolvedLayout,
    traits::ArchiveHeader as ArchiveHeaderTrait,
    validation::context::ValidationContext,
};

/// Validator for byte streams to ensure safety before access.
pub struct Validator<'a, H = ArchiveHeader>
where
    H: ArchiveHeaderTrait,
{
    data: &'a [u8],
    layouts: Option<LayoutDirectory<'a>>,
    cached_layout: Option<(StableSchemaKey, SchemaRevision, ResolvedLayout<'a, H>)>,
    depth: usize,
    max_depth: usize,
}

pub struct DepthGuard<'v, H = ArchiveHeader>
where
    H: ArchiveHeaderTrait,
{
    context: *mut Validator<'v, H>,
    _phantom: PhantomData<&'v mut Validator<'v, H>>,
}

impl<'v, H: ArchiveHeaderTrait> Drop for DepthGuard<'v, H> {
    fn drop(&mut self) {
        unsafe {
            (*self.context).pop_depth();
        }
    }
}

impl<'a, H: ArchiveHeaderTrait> Validator<'a, H> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            layouts: None,
            cached_layout: None,
            depth: 0,
            max_depth: 128,
        }
    }

    pub fn with_layouts(data: &'a [u8], layouts: LayoutDirectory<'a>) -> Self {
        Self {
            data,
            layouts: Some(layouts),
            cached_layout: None,
            depth: 0,
            max_depth: 128,
        }
    }

    /// Enter a nested validation scope and automatically unwind on drop.
    pub fn enter(&mut self) -> Result<DepthGuard<'a, H>, ZebinError> {
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
        let buffer_end = buffer_start
            .checked_add(self.data.len())
            .ok_or_else(|| self.validation_error("Buffer length overflow", 0))?;

        let end = start.checked_add(size).ok_or_else(|| {
            self.validation_error("Pointer range overflow", start.saturating_sub(buffer_start))
        })?;

        if start < buffer_start || end > buffer_end {
            return Err(
                self.validation_error("Pointer out of bounds", start.saturating_sub(buffer_start))
            );
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
                path: Default::default(),
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

    pub fn resolved_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
    ) -> Result<ResolvedLayout<'a, H>, ZebinError> {
        if let Some((cached_key, cached_revision, resolved)) = &self.cached_layout
            && *cached_key == stable_schema_key
            && *cached_revision == schema_revision
        {
            return Ok(*resolved);
        }

        let layouts = self.layouts.ok_or_else(|| ZebinError::ValidationError {
            message: "Missing layout directory",
            pos: 0,
            path: Default::default(),
        })?;
        let header = H::parse(self.data)?;
        let layout = layouts.lookup(stable_schema_key, schema_revision)?;
        let resolved = ResolvedLayout::from_parts(self.data, header, layout);
        self.cached_layout = Some((stable_schema_key, schema_revision, resolved));
        Ok(resolved)
    }
}

impl<'a, H: ArchiveHeaderTrait> ValidationContext<H> for Validator<'a, H> {
    fn push_depth(&mut self) -> Result<(), ZebinError> {
        Validator::push_depth(self)
    }

    fn pop_depth(&mut self) {
        Validator::pop_depth(self)
    }

    fn check_range(&self, ptr: *const u8, size: usize) -> Result<(), ZebinError> {
        Validator::check_range(self, ptr, size)
    }

    fn check_alignment(&self, ptr: *const u8, alignment: NonZeroUsize) -> Result<(), ZebinError> {
        Validator::check_alignment(self, ptr, alignment)
    }

    fn resolved_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
    ) -> Result<ResolvedLayout<'a, H>, ZebinError> {
        Validator::resolved_layout(self, stable_schema_key, schema_revision)
    }
}
