use alloc::string::String;
use core::num::NonZeroUsize;

#[derive(Debug)]
pub enum ZebinError {
    Infallible,
    WriteError,
    AlignmentError {
        expected: NonZeroUsize,
        actual: NonZeroUsize,
        pos: usize,
    },
    LayoutError,
    ValidationError {
        message: String,
        pos: usize,
    },
    RecursionLimitExceeded,
    #[cfg(feature = "mmap")]
    ReadOnlyStorage,
}

impl core::fmt::Display for ZebinError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ZebinError::Infallible => write!(f, "infallible error"),
            ZebinError::WriteError => write!(f, "failed to write archive bytes"),
            ZebinError::AlignmentError {
                expected,
                actual,
                pos,
            } => {
                write!(
                    f,
                    "alignment error at {pos}: expected alignment {}, actual remainder {}",
                    expected, actual
                )
            }
            ZebinError::LayoutError => write!(f, "layout error"),
            ZebinError::ValidationError { message, pos } => {
                write!(f, "validation error at {pos}: {message}")
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
