use std::num::NonZeroUsize;

use crate::core::schema::LayoutField;

/// Object model layer: type-level archive/serialize/validate contracts.
pub trait Archive {
    /// The archived version of this type.
    type Archived;
    /// The resolver used to construct the archived version.
    type Resolver;
    /// The alignment requirement for the archived version.
    const ALIGNMENT: NonZeroUsize;

    /// Resolve the archived version using the given resolver.
    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError>;

    /// Write a deterministic byte representation of an archived value.
    fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]);

    /// Convert an archived value to a freshly allocated byte vector.
    fn archived_bytes(archived: &Self::Archived) -> Vec<u8> {
        let mut out = vec![0u8; std::mem::size_of::<Self::Archived>()];
        Self::write_archived_bytes(archived, &mut out);
        out
    }
}

#[derive(Debug)]
pub enum ZebinError {
    Infallible,
    WriteError,
    ReadOnlyStorage,
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
}

impl std::fmt::Display for ZebinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZebinError::Infallible => write!(f, "infallible error"),
            ZebinError::WriteError => write!(f, "failed to write archive bytes"),
            ZebinError::ReadOnlyStorage => write!(f, "storage backend is read-only"),
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
        }
    }
}

impl std::error::Error for ZebinError {}

impl From<std::convert::Infallible> for ZebinError {
    fn from(error: std::convert::Infallible) -> Self {
        match error {}
    }
}

/// Trait for layout-aware encoders.
pub trait Encoder {
    type Error: std::error::Error + From<std::convert::Infallible>;

    fn pos(&self) -> usize;

    /// Write as many bytes as possible and return the amount consumed.
    fn write(&mut self, bytes: &[u8]) -> Result<usize, Self::Error>;

    /// Write as many alignment bytes as possible and return the amount consumed.
    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, Self::Error>;

    /// Register a layout descriptor for the current object and return its schema id.
    fn register_layout(&mut self, layout: &[LayoutField]) -> Result<u32, Self::Error>;
}

/// Result of polling a serialize state.
pub enum SerializePoll<T> {
    Pending,
    Ready(T),
    Error(ZebinError),
}

/// Trait for resumable serialization states.
pub trait SerializeState {
    type Resolver;

    fn poll<E: Encoder + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<SerializePoll<Self::Resolver>, E::Error>
    where
        E::Error: From<ZebinError>;
}

/// Trait for types that can create resumable serialization states.
pub trait Serialize: Archive {
    type State<'a>: SerializeState<Resolver = Self::Resolver>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError>;
}

/// Trait for types that can be validated for safety.
pub trait Validate<C: ?Sized> {
    /// The alignment requirement for the archived representation.
    const ALIGNMENT: NonZeroUsize;

    /// Validate the archived version of this type.
    ///
    /// # Safety
    /// The pointer must point to a valid memory location that can be read.
    unsafe fn validate(ptr: *const Self, context: &mut C) -> Result<(), ZebinError>;
}

/// Byte-oriented state used by fixed-width primitive encoders.
pub struct ByteState<const N: usize> {
    bytes: [u8; N],
    cursor: usize,
    alignment: NonZeroUsize,
    aligned: bool,
}

impl<const N: usize> ByteState<N> {
    pub fn new(bytes: [u8; N], alignment: NonZeroUsize) -> Self {
        Self {
            bytes,
            cursor: 0,
            alignment,
            aligned: false,
        }
    }
}

impl<const N: usize> SerializeState for ByteState<N> {
    type Resolver = ();

    fn poll<E: Encoder + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<SerializePoll<Self::Resolver>, E::Error>
    where
        E::Error: From<ZebinError>,
    {
        if !self.aligned {
            let _ = encoder.align(self.alignment)?;
            if encoder.pos() % self.alignment.get() != 0 {
                return Ok(SerializePoll::Pending);
            }
            self.aligned = true;
        }

        let written = encoder.write(&self.bytes[self.cursor..])?;
        self.cursor += written;
        if self.cursor < N {
            Ok(SerializePoll::Pending)
        } else {
            Ok(SerializePoll::Ready(()))
        }
    }
}

macro_rules! impl_archive_for_primitive {
    ($($t:ty),* $(,)?) => {
        $(
            impl Archive for $t {
                type Archived = $t;
                type Resolver = ();
                const ALIGNMENT: NonZeroUsize = unsafe {
                    NonZeroUsize::new_unchecked(std::mem::size_of::<Self>())
                };

                fn resolve(&self, _pos: usize, _resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
                    Ok(*self)
                }

                fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
                    out.copy_from_slice(&archived.to_le_bytes());
                }
            }

            impl Serialize for $t {
                type State<'a> = ByteState<{ std::mem::size_of::<$t>() }> where Self: 'a;

                fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
                    Ok(ByteState::new(
                        self.to_le_bytes(),
                        unsafe {
                            NonZeroUsize::new_unchecked(std::mem::size_of::<Self>())
                        },
                    ))
                }
            }

            impl<C: ?Sized> Validate<C> for $t {
                const ALIGNMENT: NonZeroUsize = unsafe {
                    NonZeroUsize::new_unchecked(std::mem::size_of::<Self>())
                };

                unsafe fn validate(_ptr: *const Self, _context: &mut C) -> Result<(), ZebinError> {
                    Ok(())
                }
            }
        )*
    };
}

impl_archive_for_primitive!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64);

impl Archive for bool {
    type Archived = bool;
    type Resolver = ();
    const ALIGNMENT: NonZeroUsize = unsafe { NonZeroUsize::new_unchecked(1) };

    fn resolve(&self, _pos: usize, _resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        Ok(*self)
    }

    fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
        out[0] = *archived as u8;
    }
}

impl Serialize for bool {
    type State<'a> = ByteState<1> where Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(ByteState::new([*self as u8], unsafe { NonZeroUsize::new_unchecked(1) }))
    }
}

impl<C: ?Sized> Validate<C> for bool {
    const ALIGNMENT: NonZeroUsize = unsafe { NonZeroUsize::new_unchecked(1) };

    unsafe fn validate(ptr: *const Self, _context: &mut C) -> Result<(), ZebinError> {
        let val = unsafe { *(ptr as *const u8) };
        if val > 1 {
            return Err(ZebinError::ValidationError {
                message: "Invalid bool value".to_string(),
                pos: ptr as usize,
            });
        }
        Ok(())
    }
}
