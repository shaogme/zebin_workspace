use alloc::{
    borrow::{Cow, ToOwned},
    boxed::Box,
    rc::Rc,
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};
use core::num::NonZeroUsize;

use crate::core::{schema::LayoutField, validator::Validator};

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
        let mut out = vec![0u8; core::mem::size_of::<Self::Archived>()];
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
    type Error: core::error::Error + From<core::convert::Infallible>;

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
            encoder.align(self.alignment)?;
            if !encoder.pos().is_multiple_of(self.alignment.get()) {
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
                    NonZeroUsize::new_unchecked(core::mem::size_of::<Self>())
                };

                fn resolve(&self, _pos: usize, _resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
                    Ok(*self)
                }

                fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
                    out.copy_from_slice(&archived.to_le_bytes());
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
                const ALIGNMENT: NonZeroUsize = unsafe {
                    NonZeroUsize::new_unchecked(core::mem::size_of::<Self>())
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
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();

    fn resolve(
        &self,
        _pos: usize,
        _resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        Ok(*self)
    }

    fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
        out[0] = *archived as u8;
    }
}

impl Serialize for bool {
    type State<'a>
        = ByteState<1>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(ByteState::new([*self as u8], unsafe {
            NonZeroUsize::new_unchecked(1)
        }))
    }
}

impl<C: ?Sized> Validate<C> for bool {
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();

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
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        self.as_ref().resolve(pos, resolver)
    }

    fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
        T::write_archived_bytes(archived, out)
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
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        self.as_ref().resolve(pos, resolver)
    }

    fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
        T::write_archived_bytes(archived, out)
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
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        self.as_ref().resolve(pos, resolver)
    }

    fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
        T::write_archived_bytes(archived, out)
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
    const ALIGNMENT: NonZeroUsize = B::ALIGNMENT;

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        self.as_ref().resolve(pos, resolver)
    }

    fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
        B::write_archived_bytes(archived, out)
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

impl<T> Archive for [T]
where
    T: Archive,
{
    type Archived = crate::archive::archived_vec::ArchivedVec<T::Archived>;
    type Resolver = usize;
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(8).unwrap();

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        crate::archive::archived_vec::resolve_sequence_archive(self, pos, resolver)
    }

    fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
        crate::archive::archived_vec::write_sequence_archive_bytes(archived, out);
    }
}

impl<T> Serialize for [T]
where
    T: Serialize + Archive,
{
    type State<'a>
        = crate::archive::archived_vec::SliceSerializeState<'a, T>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        crate::archive::archived_vec::SliceSerializeState::new(self)
    }
}

pub struct ArraySerializeState<'a, T, const N: usize>
where
    T: Serialize + Archive + 'a,
{
    items: &'a [T; N],
    index: usize,
    current_state: Option<Box<<T as Serialize>::State<'a>>>,
    resolvers: [Option<T::Resolver>; N],
}

impl<'a, T, const N: usize> ArraySerializeState<'a, T, N>
where
    T: Serialize + Archive + 'a,
{
    fn new(items: &'a [T; N]) -> Result<Self, ZebinError> {
        Ok(Self {
            items,
            index: 0,
            current_state: None,
            resolvers: core::array::from_fn(|_| None),
        })
    }
}

impl<'a, T, const N: usize> SerializeState for ArraySerializeState<'a, T, N>
where
    T: Serialize + Archive + 'a,
{
    type Resolver = [T::Resolver; N];

    fn poll<E: Encoder + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<SerializePoll<Self::Resolver>, E::Error>
    where
        E::Error: From<ZebinError>,
    {
        loop {
            if self.index >= N {
                return Ok(SerializePoll::Ready(core::array::from_fn(|i| {
                    self.resolvers[i]
                        .take()
                        .expect("array resolver stored when element serialization completed")
                })));
            }

            if self.current_state.is_none() {
                self.current_state = Some(Box::new(self.items[self.index].begin()?));
            }

            match self
                .current_state
                .as_mut()
                .expect("state initialized above")
                .poll(encoder)?
            {
                SerializePoll::Pending => return Ok(SerializePoll::Pending),
                SerializePoll::Error(err) => return Ok(SerializePoll::Error(err)),
                SerializePoll::Ready(resolver) => {
                    self.resolvers[self.index] = Some(resolver);
                    self.current_state = None;
                    self.index += 1;
                }
            }
        }
    }
}

impl<T, const N: usize> Archive for [T; N]
where
    T: Archive,
{
    type Archived = [T::Archived; N];
    type Resolver = [T::Resolver; N];
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        let elem_size = core::mem::size_of::<T::Archived>();
        let mut out = core::mem::MaybeUninit::<[T::Archived; N]>::uninit();
        let out_ptr = out.as_mut_ptr() as *mut T::Archived;
        let mut resolver_iter = resolver.into_iter();

        for (index, item) in self.iter().enumerate() {
            let item_resolver = resolver_iter.next().expect("array resolver length matches");
            let item_pos = pos
                .checked_add(index.checked_mul(elem_size).ok_or(ZebinError::WriteError)?)
                .ok_or(ZebinError::WriteError)?;
            let item = item.resolve(item_pos, item_resolver)?;
            unsafe {
                out_ptr.add(index).write(item);
            }
        }

        Ok(unsafe { out.assume_init() })
    }

    fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
        out.fill(0);
        let elem_size = core::mem::size_of::<T::Archived>();
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

impl<T, const N: usize> Serialize for [T; N]
where
    T: Serialize + Archive,
{
    type State<'a>
        = ArraySerializeState<'a, T, N>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        ArraySerializeState::new(self)
    }
}

impl<'v, T, const N: usize> Validate<Validator<'v>> for [T; N]
where
    T: Validate<Validator<'v>>,
{
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;

    unsafe fn validate(ptr: *const Self, context: &mut Validator<'v>) -> Result<(), ZebinError> {
        let _guard = context.enter()?;
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
