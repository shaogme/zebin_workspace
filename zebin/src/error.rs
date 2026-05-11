use crate::{
    core::schema::{FieldEncoding, SchemaRevision, StableSchemaKey},
    validation::validator::ValidationPathSegment,
};
use core::num::NonZeroUsize;

/// Errors that can occur during archive header parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseHeaderError {
    TooShort { pos: usize },
    InvalidMagic { pos: usize },
    UnsupportedVersion { version: u8, pos: usize },
}

impl core::fmt::Display for ParseHeaderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ParseHeaderError::TooShort { pos } => write!(f, "header too short at {pos}"),
            ParseHeaderError::InvalidMagic { pos } => write!(f, "invalid magic at {pos}"),
            ParseHeaderError::UnsupportedVersion { version, pos } => {
                write!(f, "unsupported archive version {version} at {pos}")
            }
        }
    }
}

impl core::error::Error for ParseHeaderError {}

/// Errors that can occur during validation or sequential decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessError {
    Infallible,
    AlignmentError {
        expected: NonZeroUsize,
        actual: NonZeroUsize,
        pos: usize,
    },
    InvalidFieldTable {
        pos: usize,
    },
    ValidationError {
        message: &'static str,
        pos: usize,
    },
    MissingField {
        field_id: u16,
        pos: usize,
    },
    DuplicateField {
        field_id: u16,
        pos: usize,
    },
    UnexpectedFieldEncoding {
        field_id: u16,
        expected: FieldEncoding,
        actual: FieldEncoding,
        pos: usize,
    },
    FieldLengthMismatch {
        field_id: u16,
        expected: usize,
        actual: usize,
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

impl AccessError {
    pub fn at(self, _segment: ValidationPathSegment) -> Self {
        self
    }
}

impl core::fmt::Display for AccessError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AccessError::Infallible => write!(f, "infallible error"),
            AccessError::AlignmentError {
                expected,
                actual,
                pos,
            } => write!(
                f,
                "alignment error at {pos}: expected alignment {}, actual remainder {}",
                expected, actual
            ),
            AccessError::InvalidFieldTable { pos } => write!(f, "invalid field table at {pos}"),
            AccessError::ValidationError { message, pos } => {
                write!(f, "validation error at {pos}: {message}")
            }
            AccessError::MissingField { field_id, pos } => {
                write!(f, "missing field {field_id} at {pos}")
            }
            AccessError::DuplicateField { field_id, pos } => {
                write!(f, "duplicate field {field_id} at {pos}")
            }
            AccessError::UnexpectedFieldEncoding {
                field_id,
                expected,
                actual,
                pos,
            } => write!(
                f,
                "field {field_id} encoding mismatch at {pos}: expected {expected:?}, found {actual:?}"
            ),
            AccessError::FieldLengthMismatch {
                field_id,
                expected,
                actual,
                pos,
            } => write!(
                f,
                "field {field_id} length mismatch at {pos}: expected {expected}, consumed {actual}"
            ),
            AccessError::FieldOverflow { field, pos } => write!(f, "{field} overflow at {pos}"),
            AccessError::FieldOutOfBounds { field, pos } => {
                write!(f, "{field} out of bounds at {pos}")
            }
            AccessError::RecursionLimitExceeded => write!(f, "recursion limit exceeded"),
        }
    }
}

impl From<core::convert::Infallible> for AccessError {
    fn from(error: core::convert::Infallible) -> Self {
        match error {}
    }
}

impl core::error::Error for AccessError {}

/// Errors that can occur during the archiving process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveError {
    LengthOverflow { pos: usize },
    ArithmeticOverflow { pos: usize },
    InvalidResolver { pos: usize },
}

impl core::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ArchiveError::LengthOverflow { pos } => write!(f, "length overflow at {pos}"),
            ArchiveError::ArithmeticOverflow { pos } => write!(f, "arithmetic overflow at {pos}"),
            ArchiveError::InvalidResolver { pos } => write!(f, "invalid resolver state at {pos}"),
        }
    }
}

impl core::error::Error for ArchiveError {}

/// Unified error type for the Zebin library.
#[derive(Debug)]
pub enum ZebinError {
    ArithmeticOverflow {
        pos: usize,
    },
    BufferTooSmall {
        pos: usize,
        required: usize,
    },
    SerializationError {
        pos: usize,
        message: &'static str,
    },
    DeserializeError {
        message: &'static str,
    },
    LayoutCollision {
        key: StableSchemaKey,
        revision: SchemaRevision,
    },
    LayoutRegistryFull,
    Access(AccessError),
    ArchiveError(ArchiveError),
    HeaderParseError(ParseHeaderError),
    #[cfg(feature = "mmap")]
    ReadOnlyStorage,
}

impl ZebinError {
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
            ZebinError::ArithmeticOverflow { pos } => write!(f, "arithmetic overflow at {pos}"),
            ZebinError::BufferTooSmall { pos, required } => {
                write!(f, "buffer too small at {pos}: required {required} bytes")
            }
            ZebinError::SerializationError { pos, message } => {
                write!(f, "serialization error at {pos}: {message}")
            }
            ZebinError::DeserializeError { message } => write!(f, "deserialize error: {message}"),
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
