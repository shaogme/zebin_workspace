use core::num::NonZeroUsize;

/// A single segment in a validation path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValidationPathSegment {
    Field(&'static str),
    Index(usize),
    Variant(&'static str),
}

/// A fixed-capacity stack for validation path segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValidationPathStack {
    segments: [Option<ValidationPathSegment>; 8],
    len: usize,
}

impl ValidationPathStack {
    pub const fn new() -> Self {
        Self {
            segments: [None; 8],
            len: 0,
        }
    }

    pub fn push(&mut self, segment: ValidationPathSegment) {
        if self.len < 8 {
            self.segments[self.len] = Some(segment);
            self.len += 1;
        }
    }

    pub fn segments(&self) -> &[Option<ValidationPathSegment>] {
        &self.segments[..self.len]
    }

    pub fn format(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for i in (0..self.len).rev() {
            if let Some(segment) = self.segments[i] {
                match segment {
                    ValidationPathSegment::Field(name) => {
                        if i < self.len - 1 {
                            write!(f, ".")?;
                        }
                        write!(f, "{}", name)?;
                    }
                    ValidationPathSegment::Index(index) => write!(f, "[{}]", index)?,
                    ValidationPathSegment::Variant(name) => write!(f, "({})", name)?,
                }
            }
        }
        Ok(())
    }
}

impl Default for ValidationPathStack {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum ZebinError {
    Infallible,
    WriteError,
    AlignmentError {
        expected: NonZeroUsize,
        actual: NonZeroUsize,
        pos: usize,
        path: ValidationPathStack,
    },
    LayoutError,
    ValidationError {
        message: &'static str,
        pos: usize,
        path: ValidationPathStack,
    },
    MissingLayoutField {
        field_id: u16,
        pos: usize,
        path: ValidationPathStack,
    },
    LayoutOffsetMismatch {
        field_id: u16,
        expected: u32,
        actual: u32,
        pos: usize,
        path: ValidationPathStack,
    },
    MissingLayoutRevision {
        key: u32,
        revision: u32,
        pos: usize,
        path: ValidationPathStack,
    },
    UnsupportedArchiveVersion {
        version: u8,
        pos: usize,
        path: ValidationPathStack,
    },
    FieldOverflow {
        field: &'static str,
        pos: usize,
        path: ValidationPathStack,
    },
    FieldOutOfBounds {
        field: &'static str,
        pos: usize,
        path: ValidationPathStack,
    },
    RecursionLimitExceeded,
    #[cfg(feature = "mmap")]
    ReadOnlyStorage,
}

impl ZebinError {
    /// Attach a path segment to the error.
    pub fn at(mut self, segment: ValidationPathSegment) -> Self {
        match &mut self {
            ZebinError::AlignmentError { path, .. }
            | ZebinError::ValidationError { path, .. }
            | ZebinError::MissingLayoutField { path, .. }
            | ZebinError::LayoutOffsetMismatch { path, .. }
            | ZebinError::MissingLayoutRevision { path, .. }
            | ZebinError::UnsupportedArchiveVersion { path, .. }
            | ZebinError::FieldOverflow { path, .. }
            | ZebinError::FieldOutOfBounds { path, .. } => {
                path.push(segment);
            }
            _ => {}
        }
        self
    }

    /// Get the validation path if present.
    pub fn path(&self) -> Option<&ValidationPathStack> {
        match self {
            ZebinError::AlignmentError { path, .. }
            | ZebinError::ValidationError { path, .. }
            | ZebinError::MissingLayoutField { path, .. }
            | ZebinError::LayoutOffsetMismatch { path, .. }
            | ZebinError::MissingLayoutRevision { path, .. }
            | ZebinError::UnsupportedArchiveVersion { path, .. }
            | ZebinError::FieldOverflow { path, .. }
            | ZebinError::FieldOutOfBounds { path, .. } => Some(path),
            _ => None,
        }
    }
}

impl core::fmt::Display for ZebinError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Some(path) = self.path() {
            if path.len > 0 {
                write!(f, "at ")?;
                path.format(f)?;
                write!(f, ": ")?;
            }
        }

        match self {
            ZebinError::Infallible => write!(f, "infallible error"),
            ZebinError::WriteError => write!(f, "failed to write archive bytes"),
            ZebinError::AlignmentError {
                expected,
                actual,
                pos,
                ..
            } => {
                write!(
                    f,
                    "alignment error at {pos}: expected alignment {}, actual remainder {}",
                    expected, actual
                )
            }
            ZebinError::LayoutError => write!(f, "layout error"),
            ZebinError::ValidationError { message, pos, .. } => {
                write!(f, "validation error at {pos}: {message}")
            }
            ZebinError::MissingLayoutField { field_id, pos, .. } => {
                write!(f, "missing layout entry for field {field_id} at {pos}")
            }
            ZebinError::LayoutOffsetMismatch {
                field_id,
                expected,
                actual,
                pos,
                ..
            } => {
                write!(
                    f,
                    "layout offset mismatch for field {field_id} at {pos}: expected {expected}, found {actual}"
                )
            }
            ZebinError::MissingLayoutRevision {
                key, revision, pos, ..
            } => {
                write!(
                    f,
                    "missing layout entry for stable schema key {key} revision {revision} at {pos}"
                )
            }
            ZebinError::UnsupportedArchiveVersion { version, pos, .. } => {
                write!(f, "unsupported archive version {version} at {pos}")
            }
            ZebinError::FieldOverflow { field, pos, .. } => {
                write!(f, "{field} overflow at {pos}")
            }
            ZebinError::FieldOutOfBounds { field, pos, .. } => {
                write!(f, "{field} out of bounds at {pos}")
            }
            ZebinError::RecursionLimitExceeded => write!(f, "recursion limit exceeded"),
            #[cfg(feature = "mmap")]
            ZebinError::ReadOnlyStorage => write!(f, "read-only storage"),
        }
    }
}

impl core::error::Error for ZebinError {}

impl From<core::convert::Infallible> for ZebinError {
    fn from(error: core::convert::Infallible) -> Self {
        match error {}
    }
}
