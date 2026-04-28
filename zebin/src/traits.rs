use alloc::{
    borrow::{Cow, ToOwned},
    collections::VecDeque,
    format,
    rc::Rc,
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};
use core::{num::NonZeroUsize, task::Poll};

use crate::core::schema::{LayoutField, ObjectEncoding, SchemaRevision, StableSchemaKey};
use crate::utils::byteops;

/// A single segment in a validation path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ValidationPathSegment {
    Field(&'static str),
    Index(usize),
    Variant(&'static str),
}

/// RAII guard that restores validation path state when dropped.
pub struct ValidationPathGuard<'a, C: ValidationContext + ?Sized> {
    context: &'a mut C,
}

impl<'a, C: ValidationContext + ?Sized> Drop for ValidationPathGuard<'a, C> {
    fn drop(&mut self) {
        self.context.pop_path_segment();
    }
}

impl<'a, C: ValidationContext + ?Sized> core::ops::Deref for ValidationPathGuard<'a, C> {
    type Target = C;

    fn deref(&self) -> &Self::Target {
        self.context
    }
}

impl<'a, C: ValidationContext + ?Sized> core::ops::DerefMut for ValidationPathGuard<'a, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.context
    }
}

/// Archived-side binary layout contract.
pub trait Layout: Sized {
    /// The alignment requirement for the archived representation.
    const ALIGNMENT: NonZeroUsize;

    /// Object encoding family used in schema metadata and archive flags.
    const ENCODING: ObjectEncoding = ObjectEncoding::Fixed;

    /// Optional byte width used when the archived form is not a plain fixed-size overlay.
    fn size_hint(&self) -> usize {
        core::mem::size_of::<Self>()
    }

    /// Write a deterministic byte representation of an archived value.
    fn write_archived_bytes(archived: &Self, out: &mut [u8]);

    /// Convert an archived value to a freshly allocated byte vector.
    fn archived_bytes(archived: &Self) -> Vec<u8> {
        let mut out = vec![0u8; archived.size_hint()];
        Self::write_archived_bytes(archived, &mut out);
        out
    }
}

/// Archived-side validation contract.
pub trait Validate {
    /// Validate an archived value in-place.
    ///
    /// # Safety
    /// The pointer must point to a valid memory location that can be read.
    unsafe fn validate<C: ValidationContext + ?Sized>(
        _ptr: *const Self,
        _context: &mut C,
    ) -> Result<(), ZebinError> {
        Ok(())
    }
}

/// Archived-side decode contract for borrowing a validated view from bytes.
pub trait Access<'a>: Sized {
    type View: 'a;

    /// Decode a borrowed view from a validated archived pointer.
    ///
    /// The returned span is the number of bytes consumed by this archived value.
    ///
    /// # Safety
    /// `ptr` must point to the first byte of a readable archived value at `context`'s current archive.
    unsafe fn access<C: ValidationContext + ?Sized>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), ZebinError>;
}

/// Object model layer: type-level archive/serialize/validate contracts.
pub trait Archive {
    /// The archived version of this type.
    type Archived: Layout + Validate;
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
pub fn archived_bytes<T: Layout>(archived: &T) -> Vec<u8> {
    T::archived_bytes(archived)
}

/// Validation context used by archived representations.
pub trait ValidationContext {
    fn push_depth(&mut self) -> Result<(), ZebinError>;

    fn pop_depth(&mut self);

    fn guard(&mut self) -> Result<ArchivedDepthGuard<'_, Self>, ZebinError> {
        ArchivedDepthGuard::new(self)
    }

    fn check_range(&self, ptr: *const u8, size: usize) -> Result<(), ZebinError>;

    fn check_alignment(&self, ptr: *const u8, alignment: NonZeroUsize) -> Result<(), ZebinError>;

    fn push_path_segment(
        &mut self,
        segment: ValidationPathSegment,
    ) -> Result<ValidationPathGuard<'_, Self>, ZebinError> {
        self.push_path_segment_raw(segment);
        Ok(ValidationPathGuard { context: self })
    }

    fn push_path_segment_raw(&mut self, segment: ValidationPathSegment);

    fn pop_path_segment(&mut self);

    fn path(&self) -> &[ValidationPathSegment];

    fn validation_error(&self, message: impl Into<String>, pos: usize) -> ZebinError {
        let path = self.path();
        if path.is_empty() {
            ZebinError::ValidationError {
                message: message.into(),
                pos,
            }
        } else {
            ZebinError::ValidationError {
                message: format_path_message(path, message.into()),
                pos,
            }
        }
    }

    fn resolved_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
    ) -> Result<crate::access::ResolvedLayout<'_>, ZebinError>;
}

/// RAII guard that restores validation depth when dropped.
pub struct ArchivedDepthGuard<'a, C: ValidationContext + ?Sized> {
    context: &'a mut C,
}

impl<'a, C: ValidationContext + ?Sized> ArchivedDepthGuard<'a, C> {
    pub fn new(context: &'a mut C) -> Result<Self, ZebinError> {
        context.push_depth()?;
        Ok(Self { context })
    }

    pub fn check_range(&mut self, ptr: *const u8, size: usize) -> Result<(), ZebinError> {
        self.context.check_range(ptr, size)
    }

    pub fn check_alignment(
        &mut self,
        ptr: *const u8,
        alignment: NonZeroUsize,
    ) -> Result<(), ZebinError> {
        self.context.check_alignment(ptr, alignment)
    }
}

fn format_path_message(path: &[ValidationPathSegment], message: String) -> String {
    use alloc::string::ToString;

    let mut prefix = String::new();
    for segment in path {
        match segment {
            ValidationPathSegment::Field(name) => {
                if !prefix.is_empty() {
                    prefix.push('.');
                }
                prefix.push_str(name);
            }
            ValidationPathSegment::Index(index) => {
                prefix.push('[');
                prefix.push_str(&index.to_string());
                prefix.push(']');
            }
            ValidationPathSegment::Variant(name) => {
                if !prefix.is_empty() {
                    prefix.push_str("::");
                }
                prefix.push_str(name);
            }
        }
    }

    if prefix.is_empty() {
        message
    } else {
        format!("{prefix}: {message}")
    }
}

impl<'a, C: ValidationContext + ?Sized> core::ops::Deref for ArchivedDepthGuard<'a, C> {
    type Target = C;

    fn deref(&self) -> &Self::Target {
        self.context
    }
}

impl<'a, C: ValidationContext + ?Sized> core::ops::DerefMut for ArchivedDepthGuard<'a, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.context
    }
}

impl<'a, C: ValidationContext + ?Sized> Drop for ArchivedDepthGuard<'a, C> {
    fn drop(&mut self) {
        self.context.pop_depth();
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
        encoding: ObjectEncoding,
        layout: &[LayoutField],
    ) -> Result<(), ZebinError>;
}

/// Trait for resumable archive construction states.
pub trait SerializeState {
    type Resolver;

    fn poll<E: ByteSink + LayoutSink + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError>;
}

/// Trait for types that can create resumable archive states.
pub trait Serialize: Archive {
    type State<'a>: SerializeState<Resolver = Self::Resolver>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError>;
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

impl<T: Layout, const N: usize> Layout for [T; N] {
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

impl<T: Layout + Validate, const N: usize> Validate for [T; N] {
    unsafe fn validate<C: ValidationContext + ?Sized>(
        ptr: *const Self,
        context: &mut C,
    ) -> Result<(), ZebinError> {
        let mut guard = context.guard()?;
        guard.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        guard.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let data_ptr = ptr as *const T;
        let elem_size = core::mem::size_of::<T>();

        for index in 0..N {
            let element_ptr = if elem_size == 0 {
                data_ptr
            } else {
                unsafe { data_ptr.add(index) }
            };
            unsafe {
                let mut path_guard =
                    guard.push_path_segment(ValidationPathSegment::Index(index))?;
                T::validate(element_ptr, &mut *path_guard)?;
            }
        }

        Ok(())
    }
}

impl<'a, T, const N: usize> Access<'a> for [T; N]
where
    T: Layout + Validate + 'a,
{
    type View = &'a Self;

    unsafe fn access<C: ValidationContext + ?Sized>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), ZebinError> {
        let typed_ptr = ptr as *const Self;
        unsafe {
            <Self as Validate>::validate(typed_ptr, context)?;
        }
        Ok((unsafe { &*typed_ptr }, core::mem::size_of::<Self>()))
    }
}

/// Buffer for sequence element resolvers while a sequence is being serialized.
pub trait SequenceResolverBuffer<T: Archive>: Sized {
    type Resolver;

    fn new(len: usize) -> Self;

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
            impl Layout for $t {
                const ALIGNMENT: NonZeroUsize = unsafe {
                    NonZeroUsize::new_unchecked(core::mem::size_of::<Self>())
                };

                fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
                    crate::utils::byteops::copy_exact(out, &archived.to_le_bytes());
                }
            }

            impl Validate for $t {}

            impl<'a> Access<'a> for $t {
                type View = &'a Self;

                unsafe fn access<C: ValidationContext + ?Sized>(
                    ptr: *const u8,
                    context: &mut C,
                ) -> Result<(Self::View, usize), ZebinError> {
                    context.check_range(ptr, core::mem::size_of::<Self>())?;
                    let typed_ptr = ptr as *const Self;
                    unsafe { <$t as Validate>::validate(typed_ptr, context)?; }
                    Ok((unsafe { &*typed_ptr }, core::mem::size_of::<Self>()))
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

                fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
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

impl Layout for bool {
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        out[0] = *archived as u8;
    }
}

impl Validate for bool {
    unsafe fn validate<C: ValidationContext + ?Sized>(
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

impl<'a> Access<'a> for bool {
    type View = &'a Self;

    unsafe fn access<C: ValidationContext + ?Sized>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), ZebinError> {
        context.check_range(ptr, core::mem::size_of::<Self>())?;
        let typed_ptr = ptr as *const Self;
        unsafe {
            <Self as Validate>::validate(typed_ptr, context)?;
        }
        Ok((unsafe { &*typed_ptr }, core::mem::size_of::<Self>()))
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

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
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

impl<T> Serialize for Box<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = <T as Serialize>::State<'a>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        self.as_ref().begin_serialize()
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

impl<T> Serialize for Rc<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = <T as Serialize>::State<'a>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        self.as_ref().begin_serialize()
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

impl<T> Serialize for Arc<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = <T as Serialize>::State<'a>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        self.as_ref().begin_serialize()
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

impl<'a, B> Serialize for Cow<'a, B>
where
    B: ?Sized + ToOwned + Serialize + Archive,
{
    type State<'b>
        = <B as Serialize>::State<'b>
    where
        Self: 'b;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        self.as_ref().begin_serialize()
    }
}
