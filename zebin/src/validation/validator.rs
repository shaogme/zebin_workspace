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

/// Validator for managing sequential archive validation state.
pub struct Validator<'p> {
    depth: usize,
    config: ValidationConfig,
    path: Option<&'p mut ValidationPathStack>,
    last_error_path: Option<ValidationPathStack>,
}

impl<'p> Validator<'p> {
    pub fn new(config: ValidationConfig, path: Option<&'p mut ValidationPathStack>) -> Self {
        Self {
            depth: 0,
            config,
            path,
            last_error_path: None,
        }
    }

    pub fn with_max_depth(max_depth: usize, path: Option<&'p mut ValidationPathStack>) -> Self {
        Self::new(
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

    pub fn check_alignment(
        &mut self,
        pos: usize,
        alignment: NonZeroUsize,
    ) -> Result<(), AccessError> {
        let alignment_value = alignment.get();
        let remainder = pos % alignment_value;
        if remainder != 0 {
            let actual =
                NonZeroUsize::new(remainder).expect("misaligned position has non-zero remainder");
            return Err(self.error(AccessError::AlignmentError {
                expected: alignment,
                actual,
                pos,
            }));
        }
        Ok(())
    }

    pub fn check_sequence_len(&mut self, len: usize, pos: usize) -> Result<(), AccessError> {
        if len > self.config.max_sequence_len {
            return Err(self.error(AccessError::ValidationError {
                message: "Sequence length limit exceeded",
                pos,
            }));
        }
        Ok(())
    }

    pub fn push_depth(&mut self) -> Result<(), AccessError> {
        if self.depth >= self.config.max_depth {
            return Err(self.error(AccessError::RecursionLimitExceeded));
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
}

impl<'p> ValidationContext for Validator<'p> {
    fn push_depth(&mut self) -> Result<(), AccessError> {
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

    fn check_alignment(&mut self, pos: usize, alignment: NonZeroUsize) -> Result<(), AccessError> {
        self.check_alignment(pos, alignment)
    }

    fn check_sequence_len(&mut self, len: usize, pos: usize) -> Result<(), AccessError> {
        self.check_sequence_len(len, pos)
    }
}
