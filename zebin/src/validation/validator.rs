use core::{marker::PhantomData, mem::MaybeUninit, num::NonZeroUsize};

#[cfg(feature = "alloc")]
use crate::alloc::vec::Vec;
use crate::{error::AccessError, validation::context::ValidationContext};

/// Validator for byte streams to ensure safe sequential decoding.
pub struct Validator<'a> {
    data: &'a [u8],
    depth: usize,
    max_depth: usize,
    path: ValidationPathStack,
    last_error_path: Option<ValidationPathStack>,
}

pub struct DepthGuard<'v> {
    context: *mut Validator<'v>,
    _phantom: PhantomData<&'v mut Validator<'v>>,
}

impl Drop for DepthGuard<'_> {
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
            depth: 0,
            max_depth: 128,
            path: ValidationPathStack::new(),
            last_error_path: None,
        }
    }

    pub fn enter(&mut self) -> Result<DepthGuard<'a>, AccessError> {
        self.push_depth()?;
        Ok(DepthGuard {
            context: self as *mut _,
            _phantom: PhantomData,
        })
    }

    pub fn check_range(&mut self, pos: usize, size: usize) -> Result<(), AccessError> {
        if size == 0 {
            return Ok(());
        }

        let end = pos
            .checked_add(size)
            .ok_or_else(|| self.validation_error("Pointer range overflow", pos))?;
        if end > self.data.len() {
            return Err(self.validation_error("Pointer out of bounds", pos));
        }
        Ok(())
    }

    pub fn check_alignment(
        &mut self,
        pos: usize,
        alignment: NonZeroUsize,
    ) -> Result<(), AccessError> {
        let alignment_value = alignment.get();
        if !pos.is_multiple_of(alignment_value) {
            return Err(AccessError::AlignmentError {
                expected: alignment,
                actual: unsafe { NonZeroUsize::new_unchecked(pos % alignment_value) },
                pos,
            });
        }
        Ok(())
    }

    pub fn push_depth(&mut self) -> Result<(), AccessError> {
        self.depth += 1;
        if self.depth > self.max_depth {
            self.depth = self.depth.saturating_sub(1);
            return Err(AccessError::RecursionLimitExceeded);
        }
        Ok(())
    }

    pub fn pop_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    pub fn push_path(&mut self, segment: ValidationPathSegment) {
        self.path.push(segment);
    }

    pub fn pop_path(&mut self) {
        self.path.pop();
    }

    pub fn record_error_path(&mut self) {
        if self.last_error_path.is_none() {
            self.last_error_path = Some(self.path.clone());
        }
    }

    pub fn last_error_path(&self) -> Option<&ValidationPathStack> {
        self.last_error_path.as_ref()
    }

    pub fn data(&self) -> &[u8] {
        self.data
    }

    fn validation_error(&mut self, message: &'static str, pos: usize) -> AccessError {
        self.record_error_path();
        AccessError::ValidationError { message, pos }
    }
}

impl ValidationContext for Validator<'_> {
    fn push_depth(&mut self) -> Result<(), AccessError> {
        Validator::push_depth(self)
    }

    fn pop_depth(&mut self) {
        Validator::pop_depth(self)
    }

    fn push_path(&mut self, segment: ValidationPathSegment) {
        Validator::push_path(self, segment)
    }

    fn pop_path(&mut self) {
        Validator::pop_path(self)
    }

    fn record_error_path(&mut self) {
        Validator::record_error_path(self)
    }

    fn check_range(&mut self, pos: usize, size: usize) -> Result<(), AccessError> {
        Validator::check_range(self, pos, size)
    }

    fn check_alignment(&mut self, pos: usize, alignment: NonZeroUsize) -> Result<(), AccessError> {
        Validator::check_alignment(self, pos, alignment)
    }
}

/// A single segment in a validation path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValidationPathSegment {
    Field(&'static str),
    Index(usize),
    Variant(&'static str),
}

/// A fixed-capacity stack for validation path segments.
#[derive(Clone)]
pub struct ValidationPathStack {
    segments: [MaybeUninit<ValidationPathSegment>; 32],
    len: usize,
    #[cfg(feature = "alloc")]
    extra: Option<Vec<ValidationPathSegment>>,
}

impl Default for ValidationPathStack {
    fn default() -> Self {
        Self::new()
    }
}

impl ValidationPathStack {
    pub const fn new() -> Self {
        Self {
            segments: [MaybeUninit::uninit(); 32],
            len: 0,
            #[cfg(feature = "alloc")]
            extra: None,
        }
    }

    pub fn push(&mut self, segment: ValidationPathSegment) {
        if self.len < 32 {
            self.segments[self.len].write(segment);
            self.len += 1;
        } else {
            #[cfg(feature = "alloc")]
            {
                if let Some(extra) = &mut self.extra {
                    extra.push(segment);
                } else {
                    self.extra = Some(crate::alloc::vec![segment]);
                }
            }
        }
    }

    pub fn pop(&mut self) {
        #[cfg(feature = "alloc")]
        if let Some(extra) = &mut self.extra
            && extra.pop().is_some()
        {
            if extra.is_empty() {
                self.extra = None;
            }
            return;
        }

        if self.len > 0 {
            self.len -= 1;
        }
    }

    pub fn format(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut first = true;

        #[cfg(feature = "alloc")]
        if let Some(extra) = &self.extra {
            for segment in extra.iter().rev() {
                Self::format_segment(f, segment, &mut first)?;
            }
        }

        for i in (0..self.len).rev() {
            let segment = unsafe { self.segments[i].assume_init_ref() };
            Self::format_segment(f, segment, &mut first)?;
        }
        Ok(())
    }

    fn format_segment(
        f: &mut core::fmt::Formatter<'_>,
        segment: &ValidationPathSegment,
        first: &mut bool,
    ) -> core::fmt::Result {
        match segment {
            ValidationPathSegment::Field(name) => {
                if !*first {
                    write!(f, ".")?;
                }
                write!(f, "{}", name)?;
            }
            ValidationPathSegment::Index(index) => write!(f, "[{}]", index)?,
            ValidationPathSegment::Variant(name) => write!(f, "({})", name)?,
        }
        *first = false;
        Ok(())
    }
}
