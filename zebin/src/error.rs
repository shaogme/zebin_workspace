use crate::schema::{FieldEncoding, ObjectEncoding, SchemaRevision, StableSchemaKey};
use core::num::NonZeroUsize;

/// Errors that can occur during archive header parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseHeaderError {
    TooShort { pos: usize },
    InvalidMagic { pos: usize },
    UnsupportedVersion { version: u8, pos: usize },
    InvalidObjectEncoding { flags: u8, pos: usize },
}

impl core::fmt::Display for ParseHeaderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ParseHeaderError::TooShort { pos } => write!(f, "header too short at {pos}"),
            ParseHeaderError::InvalidMagic { pos } => write!(f, "invalid magic at {pos}"),
            ParseHeaderError::UnsupportedVersion { version, pos } => {
                write!(f, "unsupported archive version {version} at {pos}")
            }
            ParseHeaderError::InvalidObjectEncoding { flags, pos } => {
                write!(f, "invalid object encoding flag {flags} at {pos}")
            }
        }
    }
}

impl core::error::Error for ParseHeaderError {}

/// Errors that can occur during validation or sequential decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
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
    UnexpectedObjectEncoding {
        expected: ObjectEncoding,
        actual: ObjectEncoding,
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

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DecodeError::Infallible => write!(f, "infallible error"),
            DecodeError::AlignmentError {
                expected,
                actual,
                pos,
            } => write!(
                f,
                "alignment error at {pos}: expected alignment {}, actual remainder {}",
                expected, actual
            ),
            DecodeError::InvalidFieldTable { pos } => write!(f, "invalid field table at {pos}"),
            DecodeError::ValidationError { message, pos } => {
                write!(f, "validation error at {pos}: {message}")
            }
            DecodeError::UnexpectedObjectEncoding {
                expected,
                actual,
                pos,
            } => write!(
                f,
                "object encoding mismatch at {pos}: expected {expected:?}, found {actual:?}"
            ),
            DecodeError::MissingField { field_id, pos } => {
                write!(f, "missing field {field_id} at {pos}")
            }
            DecodeError::DuplicateField { field_id, pos } => {
                write!(f, "duplicate field {field_id} at {pos}")
            }
            DecodeError::UnexpectedFieldEncoding {
                field_id,
                expected,
                actual,
                pos,
            } => write!(
                f,
                "field {field_id} encoding mismatch at {pos}: expected {expected:?}, found {actual:?}"
            ),
            DecodeError::FieldLengthMismatch {
                field_id,
                expected,
                actual,
                pos,
            } => write!(
                f,
                "field {field_id} length mismatch at {pos}: expected {expected}, consumed {actual}"
            ),
            DecodeError::FieldOverflow { field, pos } => write!(f, "{field} overflow at {pos}"),
            DecodeError::FieldOutOfBounds { field, pos } => {
                write!(f, "{field} out of bounds at {pos}")
            }
            DecodeError::RecursionLimitExceeded => write!(f, "recursion limit exceeded"),
        }
    }
}

impl From<core::convert::Infallible> for DecodeError {
    fn from(error: core::convert::Infallible) -> Self {
        match error {}
    }
}

impl core::error::Error for DecodeError {}

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
    Decode(DecodeError),
    ArchiveError(ArchiveError),
    HeaderParseError(ParseHeaderError),
    #[cfg(feature = "mmap")]
    ReadOnlyStorage,
    #[cfg(feature = "std")]
    Io(std::io::Error),
}

impl From<DecodeError> for ZebinError {
    fn from(error: DecodeError) -> Self {
        ZebinError::Decode(error)
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
            ZebinError::Decode(err) => write!(f, "{}", err),
            ZebinError::ArchiveError(err) => write!(f, "{}", err),
            ZebinError::HeaderParseError(err) => write!(f, "header parse error: {}", err),
            #[cfg(feature = "mmap")]
            ZebinError::ReadOnlyStorage => write!(f, "read-only storage"),
            #[cfg(feature = "std")]
            ZebinError::Io(err) => write!(f, "io error: {err}"),
        }
    }
}

#[cfg(feature = "std")]
impl From<std::io::Error> for ZebinError {
    fn from(error: std::io::Error) -> Self {
        ZebinError::Io(error)
    }
}

impl core::error::Error for ZebinError {}

impl From<core::convert::Infallible> for ZebinError {
    fn from(error: core::convert::Infallible) -> Self {
        match error {}
    }
}
