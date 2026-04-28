use alloc::{
    borrow::{Cow, ToOwned},
    boxed::Box,
    collections::VecDeque,
    rc::Rc,
    string::String,
    sync::Arc,
    vec,
    vec::Vec,
};
use core::{num::NonZeroUsize, task::Poll};

use crate::core::schema::LayoutField;

/// Archived-side byte layout contract.
pub trait ArchivedBytes {
    /// The alignment requirement for the archived representation.
    const ALIGNMENT: NonZeroUsize;

    /// Write a deterministic byte representation of an archived value.
    fn write_archived_bytes(archived: &Self, out: &mut [u8]);

    /// Convert an archived value to a freshly allocated byte vector.
    fn archived_bytes(archived: &Self) -> Vec<u8>
    where
        Self: Sized,
    {
        let mut out = vec![0u8; core::mem::size_of::<Self>()];
        Self::write_archived_bytes(archived, &mut out);
        out
    }
}

/// Object model layer: type-level archive/serialize/validate contracts.
pub trait Archive {
    /// The archived version of this type.
    type Archived: ArchivedBytes;
    /// The resolver used to construct the archived version.
    type Resolver;

    /// Resolve the archived version using the given resolver.
    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError>;

    /// Convert an archived value to a freshly allocated byte vector.
    fn archived_bytes(archived: &Self::Archived) -> Vec<u8> {
        <Self::Archived as ArchivedBytes>::archived_bytes(archived)
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

impl core::fmt::Display for ZebinError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
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

impl core::error::Error for ZebinError {}

impl From<core::convert::Infallible> for ZebinError {
    fn from(error: core::convert::Infallible) -> Self {
        match error {}
    }
}

/// Trait for layout-aware encoders.
pub trait Encoder {
    fn pos(&self) -> usize;

    /// Write as many bytes as possible and return the amount consumed.
    fn write(&mut self, bytes: &[u8]) -> Result<usize, ZebinError>;

    /// Write as many alignment bytes as possible and return the amount consumed.
    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, ZebinError>;

    /// Register a layout descriptor for the current object and return its schema id.
    fn register_layout(&mut self, layout: &[LayoutField]) -> Result<u32, ZebinError>;
}

/// Trait for resumable serialization states.
pub trait SerializeState {
    type Resolver;

    fn poll<E: Encoder + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError>;
}

/// Trait for types that can create resumable serialization states.
pub trait Serialize: Archive {
    type State<'a>: SerializeState<Resolver = Self::Resolver>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError>;
}

/// Trait for types that can be validated for safety.
pub trait Validate<C: ?Sized>: ArchivedBytes {
    /// Validate the archived version of this type.
    ///
    /// # Safety
    /// The pointer must point to a valid memory location that can be read.
    unsafe fn validate(ptr: *const Self, context: &mut C) -> Result<(), ZebinError>;
}

/// Source of indexed sequence items.
pub trait SequenceSource<T> {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn get(&self, index: usize) -> &T;
}

impl<T> SequenceSource<T> for [T] {
    fn len(&self) -> usize {
        <[T]>::len(self)
    }

    fn get(&self, index: usize) -> &T {
        &self[index]
    }
}

impl<T, const N: usize> SequenceSource<T> for [T; N] {
    fn len(&self) -> usize {
        N
    }

    fn get(&self, index: usize) -> &T {
        &self[index]
    }
}

impl<T> SequenceSource<T> for Vec<T> {
    fn len(&self) -> usize {
        Vec::len(self)
    }

    fn get(&self, index: usize) -> &T {
        &self[index]
    }
}

impl<T> SequenceSource<T> for VecDeque<T> {
    fn len(&self) -> usize {
        VecDeque::len(self)
    }

    fn get(&self, index: usize) -> &T {
        &self[index]
    }
}

impl<T: ArchivedBytes, const N: usize> ArchivedBytes for [T; N] {
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        out.fill(0);
        let elem_size = core::mem::size_of::<T>();
        if elem_size == 0 {
            return;
        }
        for (index, item) in archived.iter().enumerate() {
            let start = index * elem_size;
            let end = start + elem_size;
            T::write_archived_bytes(item, &mut out[start..end]);
        }
    }
}

/// Buffer for sequence element resolvers while a sequence is being serialized.
pub trait SequenceResolverBuffer<T: Archive> {
    type Resolver;

    fn new(len: usize) -> Self
    where
        Self: Sized;

    fn store(&mut self, index: usize, resolver: T::Resolver);

    fn take(&mut self, index: usize) -> T::Resolver;

    fn finish(self, data_pos: usize) -> Self::Resolver;
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
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        if !self.aligned {
            encoder.align(self.alignment)?;
            if !encoder.pos().is_multiple_of(self.alignment.get()) {
                return Ok(Poll::Pending);
            }
            self.aligned = true;
        }

        let written = encoder.write(&self.bytes[self.cursor..])?;
        self.cursor += written;
        if self.cursor < N {
            Ok(Poll::Pending)
        } else {
            Ok(Poll::Ready(()))
        }
    }
}

macro_rules! impl_archive_for_primitive {
    ($($t:ty),* $(,)?) => {
        $(
            impl ArchivedBytes for $t {
                const ALIGNMENT: NonZeroUsize = unsafe {
                    NonZeroUsize::new_unchecked(core::mem::size_of::<Self>())
                };

                fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
                    out.copy_from_slice(&archived.to_le_bytes());
                }
            }

            impl Archive for $t {
                type Archived = $t;
                type Resolver = ();

                fn resolve(&self, _pos: usize, _resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
                    Ok(*self)
                }
            }

            impl Serialize for $t {
                type State<'a> = ByteState<{ core::mem::size_of::<$t>() }> where Self: 'a;

                fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
                    Ok(ByteState::new(
                        self.to_le_bytes(),
                        unsafe {
                            NonZeroUsize::new_unchecked(core::mem::size_of::<Self>())
                        },
                    ))
                }
            }

            impl<C: ?Sized> Validate<C> for $t {
                unsafe fn validate(_ptr: *const Self, _context: &mut C) -> Result<(), ZebinError> {
                    Ok(())
                }
            }
        )*
    };
}

impl_archive_for_primitive!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64);

impl ArchivedBytes for bool {
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        out[0] = *archived as u8;
    }
}

impl Archive for bool {
    type Archived = bool;
    type Resolver = ();

    fn resolve(
        &self,
        _pos: usize,
        _resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        Ok(*self)
    }
}

impl Serialize for bool {
    type State<'a>
        = ByteState<1>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(ByteState::new([*self as u8], NonZeroUsize::new(1).unwrap()))
    }
}

impl<C: ?Sized> Validate<C> for bool {
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

impl<T> Archive for Box<T>
where
    T: Archive,
{
    type Archived = T::Archived;
    type Resolver = T::Resolver;

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        self.as_ref().resolve(pos, resolver)
    }
}

impl<T> Serialize for Box<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = <T as Serialize>::State<'a>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        self.as_ref().begin()
    }
}

impl<T> Archive for Rc<T>
where
    T: Archive,
{
    type Archived = T::Archived;
    type Resolver = T::Resolver;

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        self.as_ref().resolve(pos, resolver)
    }
}

impl<T> Serialize for Rc<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = <T as Serialize>::State<'a>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        self.as_ref().begin()
    }
}

impl<T> Archive for Arc<T>
where
    T: Archive,
{
    type Archived = T::Archived;
    type Resolver = T::Resolver;

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        self.as_ref().resolve(pos, resolver)
    }
}

impl<T> Serialize for Arc<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = <T as Serialize>::State<'a>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        self.as_ref().begin()
    }
}

impl<'a, B> Archive for Cow<'a, B>
where
    B: ?Sized + ToOwned + Archive,
{
    type Archived = B::Archived;
    type Resolver = B::Resolver;

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        self.as_ref().resolve(pos, resolver)
    }
}

impl<'a, B> Serialize for Cow<'a, B>
where
    B: ?Sized + ToOwned + Serialize + Archive,
{
    type State<'b>
        = <B as Serialize>::State<'b>
    where
        Self: 'b;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        self.as_ref().begin()
    }
}
