use crate::{
    core::schema::{SchemaRevision, StableSchemaKey},
    validation::validator::ValidationPathSegment,
};
use core::num::NonZeroUsize;

/// Errors that can occur during the archiving/resolution process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveError {
    /// Relative pointer offset out of range.
    OffsetOutOfRange { pos: usize },
    /// Relative pointer offset is zero.
    ZeroOffset { pos: usize },
    /// A length or offset exceeded the capacity of its representation (e.g., u32).
    LengthOverflow { pos: usize },
    /// Arithmetic overflow during position calculation.
    ArithmeticOverflow { pos: usize },
    /// The resolver was in an invalid state for the type being resolved.
    InvalidResolver { pos: usize },
}

impl core::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ArchiveError::OffsetOutOfRange { pos } => {
                write!(f, "relative pointer offset out of range at {pos}")
            }
            ArchiveError::ZeroOffset { pos } => {
                write!(f, "relative pointer offset cannot be zero at {pos}")
            }
            ArchiveError::LengthOverflow { pos } => {
                write!(f, "length overflow at {pos}")
            }
            ArchiveError::ArithmeticOverflow { pos } => {
                write!(f, "arithmetic overflow at {pos}")
            }
            ArchiveError::InvalidResolver { pos } => {
                write!(f, "invalid resolver state at {pos}")
            }
        }
    }
}

impl core::error::Error for ArchiveError {}

/// Errors that can occur during archive header parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseHeaderError {
    /// Input buffer is smaller than the required header size.
    TooShort { pos: usize },
    /// Magic bytes do not match the expected value.
    InvalidMagic { pos: usize },
    /// Archive version is not supported.
    UnsupportedVersion { version: u8, pos: usize },
    /// Layout offset is zero.
    InvalidLayoutOffset { pos: usize },
    /// Root offset is zero.
    InvalidRootOffset { pos: usize },
}

impl core::fmt::Display for ParseHeaderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ParseHeaderError::TooShort { pos } => {
                write!(f, "header too short at {pos}")
            }
            ParseHeaderError::InvalidMagic { pos } => {
                write!(f, "invalid magic at {pos}")
            }
            ParseHeaderError::UnsupportedVersion { version, pos } => {
                write!(f, "unsupported archive version {version} at {pos}")
            }
            ParseHeaderError::InvalidLayoutOffset { pos } => {
                write!(f, "invalid layout offset at {pos}")
            }
            ParseHeaderError::InvalidRootOffset { pos } => {
                write!(f, "invalid root offset at {pos}")
            }
        }
    }
}

impl core::error::Error for ParseHeaderError {}

/// Errors that can occur during validation of archived data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidateError {
    Infallible,
    AlignmentError {
        expected: NonZeroUsize,
        actual: NonZeroUsize,
        pos: usize,
    },
    InvalidLayout {
        pos: usize,
    },
    ValidationError {
        message: &'static str,
        pos: usize,
    },
    MissingLayoutField {
        field_id: u16,
        pos: usize,
    },
    LayoutOffsetMismatch {
        field_id: u16,
        expected: u32,
        actual: u32,
        pos: usize,
    },
    MissingLayoutRevision {
        key: u32,
        revision: u32,
        pos: usize,
    },
    FieldOverflow {
        field: &'static str,
        pos: usize,
    },
    FieldOutOfBounds {
        field: &'static str,
        pos: usize,
    },
    RecursionLimitExceeded,
}

impl ValidateError {
    /// No-op for backward compatibility.
    pub fn at(self, _segment: ValidationPathSegment) -> Self {
        self
    }
}

impl core::fmt::Display for ValidateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ValidateError::Infallible => write!(f, "infallible error"),
            ValidateError::AlignmentError {
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
            ValidateError::InvalidLayout { pos, .. } => {
                write!(f, "invalid layout structure at {pos}")
            }
            ValidateError::ValidationError { message, pos, .. } => {
                write!(f, "validation error at {pos}: {message}")
            }
            ValidateError::MissingLayoutField { field_id, pos, .. } => {
                write!(f, "missing layout entry for field {field_id} at {pos}")
            }
            ValidateError::LayoutOffsetMismatch {
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
            ValidateError::MissingLayoutRevision {
                key, revision, pos, ..
            } => {
                write!(
                    f,
                    "missing layout entry for stable schema key {key} revision {revision} at {pos}"
                )
            }
            ValidateError::FieldOverflow { field, pos, .. } => {
                write!(f, "{field} overflow at {pos}")
            }
            ValidateError::FieldOutOfBounds { field, pos, .. } => {
                write!(f, "{field} out of bounds at {pos}")
            }
            ValidateError::RecursionLimitExceeded => write!(f, "recursion limit exceeded"),
        }
    }
}

impl From<core::convert::Infallible> for ValidateError {
    fn from(error: core::convert::Infallible) -> Self {
        match error {}
    }
}

impl core::error::Error for ValidateError {}

pub type AccessError = ValidateError;

/// Unified error type for the Zebin library.
#[derive(Debug)]
pub enum ZebinError {
    /// Arithmetic overflow during serialization or position calculation.
    ArithmeticOverflow { pos: usize },
    /// Output buffer is too small for the data being written.
    BufferTooSmall { pos: usize, required: usize },
    /// General error during serialization.
    SerializationError { pos: usize, message: &'static str },
    /// General error during deserialization/restoration.
    DeserializeError { message: &'static str },
    /// Error during layout registration.
    /// A different layout is already registered for the same stable schema key and revision.
    LayoutCollision {
        key: StableSchemaKey,
        revision: SchemaRevision,
    },
    /// The layout registry has reached its capacity.
    LayoutRegistryFull,
    /// Error during validation or access of archived data.
    Access(AccessError),
    /// Error during archive resolution.
    ArchiveError(ArchiveError),
    /// Error during header parsing.
    HeaderParseError(ParseHeaderError),
    #[cfg(feature = "mmap")]
    ReadOnlyStorage,
}

impl ZebinError {
    /// No-op for backward compatibility.
    pub fn at(self, _segment: ValidationPathSegment) -> Self {
        self
    }
}

impl From<AccessError> for ZebinError {
    fn from(error: AccessError) -> Self {
        ZebinError::Access(error)
    }
}

impl From<ArchiveError> for ZebinError {
    fn from(error: ArchiveError) -> Self {
        ZebinError::ArchiveError(error)
    }
}

impl From<ParseHeaderError> for ZebinError {
    fn from(error: ParseHeaderError) -> Self {
        ZebinError::HeaderParseError(error)
    }
}

impl core::fmt::Display for ZebinError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ZebinError::ArithmeticOverflow { pos } => {
                write!(f, "arithmetic overflow at {pos}")
            }
            ZebinError::BufferTooSmall { pos, required } => {
                write!(f, "buffer too small at {pos}: required {required} bytes")
            }
            ZebinError::SerializationError { pos, message } => {
                write!(f, "serialization error at {pos}: {message}")
            }
            ZebinError::DeserializeError { message } => {
                write!(f, "deserialize error: {message}")
            }
            ZebinError::LayoutCollision { key, revision } => {
                write!(f, "layout collision for key {key} revision {revision}")
            }
            ZebinError::LayoutRegistryFull => write!(f, "layout registry capacity exceeded"),
            ZebinError::Access(err) => write!(f, "{}", err),
            ZebinError::ArchiveError(err) => write!(f, "{}", err),
            ZebinError::HeaderParseError(err) => write!(f, "header parse error: {}", err),
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
