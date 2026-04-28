use alloc::{
    borrow::{Cow, ToOwned},
    boxed::Box,
    collections::VecDeque,
    rc::Rc,
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};
use core::{num::NonZeroUsize, task::Poll};

use crate::byteops;
use crate::core::schema::{LayoutField, SchemaRevision, StableSchemaKey};

/// Archived-side binary layout contract.
pub trait ArchivedLayout {
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

/// Archived-side validation contract.
pub trait ArchivedValidate {
    /// Validate an archived value in-place.
    ///
    /// # Safety
    /// The pointer must point to a valid memory location that can be read.
    unsafe fn validate<C: ArchivedValidationContext + ?Sized>(
        _ptr: *const Self,
        _context: &mut C,
    ) -> Result<(), ZebinError> {
        Ok(())
    }
}

/// Object model layer: type-level archive/serialize/validate contracts.
pub trait Archive {
    /// The archived version of this type.
    type Archived: ArchivedLayout + ArchivedValidate;
    /// The resolver used to construct the archived version.
    type Resolver;

    /// Resolve the archived version using the given resolver.
    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError>;
}

/// Convert an archived value to a freshly allocated byte vector.
pub fn archived_bytes<T: ArchivedLayout>(archived: &T) -> Vec<u8> {
    T::archived_bytes(archived)
}

/// Validation context used by archived representations.
pub trait ArchivedValidationContext {
    fn push_depth(&mut self) -> Result<(), ZebinError>;

    fn pop_depth(&mut self);

    fn guard(&mut self) -> Result<ArchivedDepthGuard<Self>, ZebinError> {
        ArchivedDepthGuard::new(self)
    }

    fn check_range(&self, ptr: *const u8, size: usize) -> Result<(), ZebinError>;

    fn check_alignment(&self, ptr: *const u8, alignment: NonZeroUsize) -> Result<(), ZebinError>;

    fn resolved_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
    ) -> Result<crate::access::ResolvedLayout<'_>, ZebinError>;
}

/// RAII guard that restores validation depth when dropped.
pub struct ArchivedDepthGuard<C: ArchivedValidationContext + ?Sized> {
    context: *mut C,
    _marker: core::marker::PhantomData<*mut C>,
}

impl<C: ArchivedValidationContext + ?Sized> ArchivedDepthGuard<C> {
    pub fn new(context: &mut C) -> Result<Self, ZebinError> {
        context.push_depth()?;
        Ok(Self {
            context,
            _marker: core::marker::PhantomData,
        })
    }
}

impl<C: ArchivedValidationContext + ?Sized> Drop for ArchivedDepthGuard<C> {
    fn drop(&mut self) {
        unsafe {
            (*self.context).pop_depth();
        }
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

/// Byte-stream sink used by archive state machines.
pub trait ByteSink {
    fn pos(&self) -> usize;

    /// Write as many bytes as possible and return the amount consumed.
    fn write(&mut self, bytes: &[u8]) -> Result<usize, ZebinError>;

    /// Write as many alignment bytes as possible and return the amount consumed.
    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, ZebinError>;
}

/// Layout registration sink used by archive state machines.
pub trait LayoutSink {
    /// Register a layout descriptor for the current object.
    fn register_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
        layout: &[LayoutField],
    ) -> Result<(), ZebinError>;
}

/// Trait for resumable archive construction states.
pub trait ArchiveState {
    type Resolver;

    fn poll<E: ByteSink + LayoutSink + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError>;
}

/// Trait for types that can create resumable archive states.
pub trait ArchiveBuilder: Archive {
    type State<'a>: ArchiveState<Resolver = Self::Resolver>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError>;
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

impl<T: ArchivedLayout, const N: usize> ArchivedLayout for [T; N] {
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        byteops::fill(out, 0);
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

impl<T: ArchivedLayout + ArchivedValidate, const N: usize> ArchivedValidate for [T; N] {
    unsafe fn validate<C: ArchivedValidationContext + ?Sized>(
        ptr: *const Self,
        context: &mut C,
    ) -> Result<(), ZebinError> {
        let _guard = context.guard()?;
        context.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        context.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let data_ptr = ptr as *const T;
        let elem_size = core::mem::size_of::<T>();

        for index in 0..N {
            let element_ptr = if elem_size == 0 {
                data_ptr
            } else {
                unsafe { data_ptr.add(index) }
            };
            unsafe {
                T::validate(element_ptr, context)?;
            }
        }

        Ok(())
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

impl<const N: usize> ArchiveState for ByteState<N> {
    type Resolver = ();

    fn poll<E: ByteSink + LayoutSink + ?Sized>(
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
            impl ArchivedLayout for $t {
                const ALIGNMENT: NonZeroUsize = unsafe {
                    NonZeroUsize::new_unchecked(core::mem::size_of::<Self>())
                };

                fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
                    crate::byteops::copy_exact(out, &archived.to_le_bytes());
                }
            }

            impl ArchivedValidate for $t {}

            impl Archive for $t {
                type Archived = $t;
                type Resolver = ();

                fn resolve(&self, _pos: usize, _resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
                    Ok(*self)
                }
            }

            impl ArchiveBuilder for $t {
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
        )*
    };
}

impl_archive_for_primitive!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64);

impl ArchivedLayout for bool {
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        out[0] = *archived as u8;
    }
}

impl ArchivedValidate for bool {
    unsafe fn validate<C: ArchivedValidationContext + ?Sized>(
        ptr: *const Self,
        _context: &mut C,
    ) -> Result<(), ZebinError> {
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

impl ArchiveBuilder for bool {
    type State<'a>
        = ByteState<1>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(ByteState::new([*self as u8], NonZeroUsize::new(1).unwrap()))
    }
}

impl<T> Archive for Box<T>
where
    T: Archive,
{
    type Archived = T::Archived;
    type Resolver = T::Resolver;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        self.as_ref().resolve(archive_pos, resolver)
    }
}

impl<T> ArchiveBuilder for Box<T>
where
    T: ArchiveBuilder + Archive,
{
    type State<'a>
        = <T as ArchiveBuilder>::State<'a>
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

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        self.as_ref().resolve(archive_pos, resolver)
    }
}

impl<T> ArchiveBuilder for Rc<T>
where
    T: ArchiveBuilder + Archive,
{
    type State<'a>
        = <T as ArchiveBuilder>::State<'a>
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

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        self.as_ref().resolve(archive_pos, resolver)
    }
}

impl<T> ArchiveBuilder for Arc<T>
where
    T: ArchiveBuilder + Archive,
{
    type State<'a>
        = <T as ArchiveBuilder>::State<'a>
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

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        self.as_ref().resolve(archive_pos, resolver)
    }
}

impl<'a, B> ArchiveBuilder for Cow<'a, B>
where
    B: ?Sized + ToOwned + ArchiveBuilder + Archive,
{
    type State<'b>
        = <B as ArchiveBuilder>::State<'b>
    where
        Self: 'b;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        self.as_ref().begin()
    }
}
