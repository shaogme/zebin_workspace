use alloc::{boxed::Box, string::ToString};
use core::{mem::MaybeUninit, num::NonZeroUsize};

use crate::{
    Archive, Encoder, Serialize, SerializePoll, SerializeState, Validate, ZebinError,
    core::validator::Validator,
};

/// Archived representation for `Option<T>`.
#[repr(C)]
pub struct ArchivedOption<T> {
    tag: u8,
    value: MaybeUninit<T>,
}

impl<T> ArchivedOption<T> {
    pub fn is_some(&self) -> bool {
        self.tag == 1
    }

    pub fn is_none(&self) -> bool {
        self.tag == 0
    }

    /// Returns the archived payload if this option is `Some`.
    ///
    /// # Safety
    /// The caller must ensure the archived option is valid.
    pub unsafe fn as_ref(&self) -> Option<&T> {
        match self.tag {
            0 => None,
            1 => Some(unsafe { self.value.assume_init_ref() }),
            _ => None,
        }
    }
}

/// Resumable serialization state for `Option<T>`.
pub struct OptionSerializeState<'a, T>
where
    T: Serialize + Archive + 'a,
{
    is_some: bool,
    inner: Option<Box<<T as Serialize>::State<'a>>>,
}

impl<'a, T> OptionSerializeState<'a, T>
where
    T: Serialize + Archive + 'a,
{
    fn new(value: Option<&'a T>) -> Result<Self, ZebinError> {
        match value {
            Some(inner) => Ok(Self {
                is_some: true,
                inner: Some(Box::new(inner.begin()?)),
            }),
            None => Ok(Self {
                is_some: false,
                inner: None,
            }),
        }
    }
}

impl<'a, T> SerializeState for OptionSerializeState<'a, T>
where
    T: Serialize + Archive + 'a,
{
    type Resolver = Option<T::Resolver>;

    fn poll<E: Encoder + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<SerializePoll<Self::Resolver>, E::Error>
    where
        E::Error: From<ZebinError>,
    {
        if !self.is_some {
            return Ok(SerializePoll::Ready(None));
        }

        match self
            .inner
            .as_mut()
            .expect("inner state initialized for Some option")
            .poll(encoder)?
        {
            SerializePoll::Pending => Ok(SerializePoll::Pending),
            SerializePoll::Error(err) => Ok(SerializePoll::Error(err)),
            SerializePoll::Ready(resolver) => {
                self.is_some = false;
                self.inner = None;
                Ok(SerializePoll::Ready(Some(resolver)))
            }
        }
    }
}

impl<T> Archive for Option<T>
where
    T: Archive,
{
    type Archived = ArchivedOption<T::Archived>;
    type Resolver = Option<T::Resolver>;
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        match (self, resolver) {
            (Some(value), Some(resolver)) => {
                let value_offset = crate::memoffset::offset_of!(ArchivedOption<T::Archived>, value);
                let archived = value.resolve(pos + value_offset, resolver)?;
                Ok(ArchivedOption {
                    tag: 1,
                    value: MaybeUninit::new(archived),
                })
            }
            (None, None) => Ok(ArchivedOption {
                tag: 0,
                value: MaybeUninit::uninit(),
            }),
            _ => Err(ZebinError::WriteError),
        }
    }

    fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
        out.fill(0);
        out[0] = archived.tag;
        if archived.tag == 1 {
            let value_offset = crate::memoffset::offset_of!(ArchivedOption<T::Archived>, value);
            let value = unsafe { archived.value.assume_init_ref() };
            T::write_archived_bytes(
                value,
                &mut out[value_offset..value_offset + core::mem::size_of::<T::Archived>()],
            );
        }
    }
}

impl<T> Serialize for Option<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = OptionSerializeState<'a, T>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        OptionSerializeState::new(self.as_ref())
    }
}

impl<'v, T> Validate<Validator<'v>> for ArchivedOption<T>
where
    T: Validate<Validator<'v>>,
{
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;

    unsafe fn validate(ptr: *const Self, context: &mut Validator<'v>) -> Result<(), ZebinError> {
        let _guard = context.enter()?;
        context.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        context.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let archived = unsafe { &*ptr };
        match archived.tag {
            0 => Ok(()),
            1 => {
                let value_ptr = archived.value.as_ptr();
                context.check_alignment(value_ptr as *const u8, T::ALIGNMENT)?;
                context.check_range(value_ptr as *const u8, core::mem::size_of::<T>())?;
                unsafe {
                    T::validate(value_ptr, context)?;
                }
                Ok(())
            }
            _ => Err(ZebinError::ValidationError {
                message: "Invalid Option discriminant".to_string(),
                pos: ptr as usize,
            }),
        }
    }
}
