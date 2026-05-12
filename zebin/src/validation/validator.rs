use core::num::NonZeroUsize;

use crate::{
    error::AccessError,
    validation::{
        context::ValidationContext,
        path::{ValidationPathSegment, ValidationPathStack},
    },
};

/// Validator for byte streams to ensure safe sequential decoding.
pub struct Validator<'a> {
    data: &'a [u8],
    depth: usize,
    max_depth: usize,
    path: ValidationPathStack,
    last_error_path: Option<ValidationPathStack>,
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
            self.record_error_path();
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
