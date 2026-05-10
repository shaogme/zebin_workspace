use crate::{
    ZebinError,
    error::{AccessError, ArchiveError, ValidateError},
    io::sink::{ByteSink, LayoutSink},
    traits::{Access, Archive, ArchiveHeader as ArchiveHeaderTrait, Layout, Serialize, Validate},
    validation::context::ValidationContext,
};
use core::num::NonZeroUsize;
use core::task::Poll;

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

use crate::write::state::SerializeState;

impl<'a, const N: usize> SerializeState<'a> for ByteState<N> {
    type Resolver = ();

    fn poll<E: ByteSink + LayoutSink<'a> + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        if !self.aligned {
            encoder.align(self.alignment)?;
            if encoder.pos() % self.alignment.get() != 0 {
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

            impl Validate for $t {
                unsafe fn validate<H, C>(_ptr: *const Self, _context: &mut C) -> Result<(), ValidateError>
                where
                    H: ArchiveHeaderTrait,
                    C: ValidationContext<H> + ?Sized,
                {
                    Ok(())
                }
            }

            impl<'a> Access<'a> for $t {
                type View = &'a Self;

                unsafe fn access<H, C>(
                    ptr: *const u8,
                    context: &mut C,
                ) -> Result<(Self::View, usize), AccessError>
                where
                    H: crate::traits::ArchiveHeader,
                    C: ValidationContext<H> + ?Sized,
                {
                    context.check_range(ptr, core::mem::size_of::<Self>())?;
                    let typed_ptr = ptr as *const Self;
                    unsafe { <$t as Validate>::validate::<H, C>(typed_ptr, context)?; }
                    Ok((unsafe { &*typed_ptr }, core::mem::size_of::<Self>()))
                }
            }

            impl Archive for $t {
                type Archived = $t;
                type Resolver = ();

                fn resolve(&self, _pos: usize, _resolver: Self::Resolver) -> Result<Self::Archived, ArchiveError> {
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
    unsafe fn validate<H, C>(ptr: *const Self, context: &mut C) -> Result<(), ValidateError>
    where
        H: crate::traits::ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let val = unsafe { *(ptr as *const u8) };
        if val > 1 {
            return Err(context.validation_error("Invalid bool value", ptr as usize));
        }
        Ok(())
    }
}

impl<'a> Access<'a> for bool {
    type View = &'a Self;

    unsafe fn access<H, C>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), AccessError>
    where
        H: crate::traits::ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        context.check_range(ptr, core::mem::size_of::<Self>())?;
        let typed_ptr = ptr as *const Self;
        unsafe {
            <Self as Validate>::validate::<H, C>(typed_ptr, context)?;
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
    ) -> Result<Self::Archived, ArchiveError> {
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
