use core::num::NonZeroUsize;

use crate::{
    core::schema::FieldEncoding,
    error::{AccessError, ZebinError},
    read::Cursor,
    traits::{Archive, Decode, FixedLayout, Restore},
    validation::context::ValidationContext,
};

impl<T, const N: usize> FixedLayout for [T; N]
where
    T: FixedLayout,
{
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;
    const SIZE: usize = T::SIZE * N;

    fn write_fixed(archived: &Self, out: &mut [u8]) {
        for i in 0..N {
            T::write_fixed(&archived[i], &mut out[i * T::SIZE..(i + 1) * T::SIZE]);
        }
    }
}

/// Source of indexed sequence items.
pub trait SequenceSource<T> {
    fn len(&self) -> usize;

    #[cfg(feature = "alloc")]
    fn get(&self, index: usize) -> &T;
}

impl<T> SequenceSource<T> for [T] {
    fn len(&self) -> usize {
        <[T]>::len(self)
    }

    #[cfg(feature = "alloc")]
    fn get(&self, index: usize) -> &T {
        &self[index]
    }
}

impl<T, const N: usize> SequenceSource<T> for [T; N] {
    fn len(&self) -> usize {
        N
    }

    #[cfg(feature = "alloc")]
    fn get(&self, index: usize) -> &T {
        &self[index]
    }
}

impl<T, const N: usize> Archive for [T; N]
where
    T: Archive,
{
    type Archived = [T::Archived; N];
}

impl<T, U, const N: usize> Restore<[U; N]> for [T; N]
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<[U; N], ZebinError> {
        let mut out = core::mem::MaybeUninit::<[U; N]>::uninit();
        let out_ptr = out.as_mut_ptr() as *mut U;
        let mut initialized = 0usize;

        while initialized < N {
            match self[initialized].restore() {
                Ok(value) => unsafe {
                    out_ptr.add(initialized).write(value);
                    initialized += 1;
                },
                Err(error) => {
                    for index in 0..initialized {
                        unsafe {
                            out_ptr.add(index).drop_in_place();
                        }
                    }
                    return Err(error);
                }
            }
        }

        Ok(unsafe { out.assume_init() })
    }
}

impl<'a, A, const N: usize> Decode<'a> for [A; N]
where
    A: Decode<'a>,
{
    type View = [A::View; N];

    const FIXED_SIZE: Option<usize> = match A::FIXED_SIZE {
        Some(size) => Some(size * N),
        None => None,
    };
    const ALIGNMENT: NonZeroUsize = A::ALIGNMENT;
    const FIELD_ENCODING: FieldEncoding = FieldEncoding::Sequence;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let mut out = core::mem::MaybeUninit::<[A::View; N]>::uninit();
        let out_ptr = out.as_mut_ptr() as *mut A::View;
        let mut initialized = 0usize;

        while initialized < N {
            let mut guard = context.push_index(initialized);
            match A::decode(cursor, &mut *guard) {
                Ok(value) => unsafe {
                    out_ptr.add(initialized).write(value);
                    initialized += 1;
                },
                Err(error) => {
                    for index in 0..initialized {
                        unsafe {
                            out_ptr.add(index).drop_in_place();
                        }
                    }
                    return Err(error);
                }
            }
        }

        Ok(unsafe { out.assume_init() })
    }
}
