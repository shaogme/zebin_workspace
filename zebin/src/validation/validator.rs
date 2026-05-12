use core::num::NonZeroUsize;

use crate::{
    error::DecodeError,
    validation::{
        context::ValidationContext,
        path::{ValidationPathSegment, ValidationPathStack},
    },
};

/// Runtime configuration for sequential archive validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationConfig {
    pub max_depth: usize,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self { max_depth: 128 }
    }
}

/// Validator for byte streams to ensure safe sequential decoding.
pub struct Validator<'a, 'p> {
    data: &'a [u8],
    depth: usize,
    config: ValidationConfig,
    path: Option<&'p mut ValidationPathStack>,
    last_error_path: Option<ValidationPathStack>,
}

impl<'a> Validator<'a, 'static> {
    pub fn new(data: &'a [u8]) -> Self {
        Self::with_config(data, ValidationConfig::default(), None)
    }
}

impl<'a, 'p> Validator<'a, 'p> {
    pub fn with_config(
        data: &'a [u8],
        config: ValidationConfig,
        path: Option<&'p mut ValidationPathStack>,
    ) -> Self {
        Self {
            data,
            depth: 0,
            config,
            path,
            last_error_path: None,
        }
    }

    pub fn with_max_depth(
        data: &'a [u8],
        max_depth: usize,
        path: Option<&'p mut ValidationPathStack>,
    ) -> Self {
        Self::with_config(data, ValidationConfig { max_depth }, path)
    }

    pub fn config(&self) -> ValidationConfig {
        self.config
    }

    pub fn check_range(&mut self, pos: usize, size: usize) -> Result<(), DecodeError> {
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
    ) -> Result<(), DecodeError> {
        let alignment_value = alignment.get();
        if !pos.is_multiple_of(alignment_value) {
            self.record_error_path();
            return Err(DecodeError::AlignmentError {
                expected: alignment,
                actual: unsafe { NonZeroUsize::new_unchecked(pos % alignment_value) },
                pos,
            });
        }
        Ok(())
    }

    pub fn push_depth(&mut self) -> Result<(), DecodeError> {
        self.depth += 1;
        if self.depth > self.config.max_depth {
            self.depth = self.depth.saturating_sub(1);
            return Err(DecodeError::RecursionLimitExceeded);
        }
        Ok(())
    }

    pub fn pop_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    pub fn push_path(&mut self, segment: ValidationPathSegment) {
        if let Some(path) = &mut self.path {
            path.push(segment);
        }
    }

    pub fn pop_path(&mut self) {
        if let Some(path) = &mut self.path {
            path.pop();
        }
    }

    pub fn record_error_path(&mut self) {
        if self.last_error_path.is_none() && self.path.is_some() {
            self.last_error_path = self.path.as_ref().map(|p| (*p).clone());
        }
    }

    pub fn last_error_path(&self) -> Option<&ValidationPathStack> {
        self.last_error_path.as_ref()
    }

    pub fn data(&self) -> &[u8] {
        self.data
    }

    fn validation_error(&mut self, message: &'static str, pos: usize) -> DecodeError {
        self.record_error_path();
        DecodeError::ValidationError { message, pos }
    }
}

impl ValidationContext for Validator<'_, '_> {
    fn push_depth(&mut self) -> Result<(), DecodeError> {
        self.push_depth()
    }

    fn pop_depth(&mut self) {
        self.pop_depth()
    }

    fn push_path(&mut self, segment: ValidationPathSegment) {
        self.push_path(segment)
    }

    fn pop_path(&mut self) {
        self.pop_path()
    }

    fn record_error_path(&mut self) {
        self.record_error_path()
    }

    fn check_range(&mut self, pos: usize, size: usize) -> Result<(), DecodeError> {
        self.check_range(pos, size)
    }

    fn check_alignment(&mut self, pos: usize, alignment: NonZeroUsize) -> Result<(), DecodeError> {
        self.check_alignment(pos, alignment)
    }
}
