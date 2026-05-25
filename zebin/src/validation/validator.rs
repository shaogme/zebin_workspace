use core::num::NonZeroUsize;

use crate::{prelude::*, validation::ValidationPathSegment};

/// Runtime configuration for sequential archive validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationConfig {
    pub max_depth: usize,
    pub max_sequence_len: usize,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            max_depth: 128,
            max_sequence_len: 1_048_576,
        }
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
        Self::with_config(
            data,
            ValidationConfig {
                max_depth,
                ..ValidationConfig::default()
            },
            path,
        )
    }

    pub fn config(&self) -> ValidationConfig {
        self.config
    }

    pub fn check_range(&mut self, pos: usize, size: usize) -> Result<(), DecodeError> {
        if size == 0 {
            if pos <= self.data.len() {
                return Ok(());
            }
            return Err(self.validation_error("Pointer out of bounds", pos));
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
        let remainder = pos % alignment_value;
        if remainder != 0 {
            let actual =
                NonZeroUsize::new(remainder).expect("misaligned position has non-zero remainder");
            return Err(self.error(DecodeError::AlignmentError {
                expected: alignment,
                actual,
                pos,
            }));
        }
        Ok(())
    }

    pub fn check_sequence_len(&mut self, len: usize, pos: usize) -> Result<(), DecodeError> {
        if len > self.config.max_sequence_len {
            return Err(self.error(DecodeError::ValidationError {
                message: "Sequence length limit exceeded",
                pos,
            }));
        }
        Ok(())
    }

    pub fn push_depth(&mut self) -> Result<(), DecodeError> {
        if self.depth >= self.config.max_depth {
            return Err(self.error(DecodeError::RecursionLimitExceeded));
        }
        self.depth += 1;
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
        if self.last_error_path.is_none()
            && let Some(path) = self.path.as_ref()
        {
            self.last_error_path = Some((*path).clone());
        }
    }

    pub fn last_error_path(&self) -> Option<&ValidationPathStack> {
        self.last_error_path.as_ref()
    }

    pub fn data(&self) -> &[u8] {
        self.data
    }

    fn validation_error(&mut self, message: &'static str, pos: usize) -> DecodeError {
        self.error(DecodeError::ValidationError { message, pos })
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

    fn check_sequence_len(&mut self, len: usize, pos: usize) -> Result<(), DecodeError> {
        self.check_sequence_len(len, pos)
    }
}
