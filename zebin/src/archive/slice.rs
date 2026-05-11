use crate::{
    core::schema::FieldEncoding,
    error::{AccessError, ZebinError},
    read::Cursor,
    traits::{Archive, ArchivedDefault, Decode, Restore},
    validation::context::ValidationContext,
};

#[cfg(feature = "alloc")]
use crate::alloc::vec::Vec;

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

/// Decoded archived vector view.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedVec<'a, T> {
    items: Vec<T>,
    _marker: core::marker::PhantomData<&'a ()>,
}

#[cfg(feature = "alloc")]
impl<'a, T> ArchivedVec<'a, T> {
    pub fn new(items: Vec<T>) -> Self {
        Self {
            items,
            _marker: core::marker::PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub unsafe fn as_slice(&self) -> &[T] {
        self.items.as_slice()
    }

    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.items.iter()
    }
}

#[cfg(feature = "alloc")]
impl<'a, T: 'static> ArchivedDefault for ArchivedVec<'a, T> {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchivedVec<'static, ()> = ArchivedVec {
            items: Vec::new(),
            _marker: core::marker::PhantomData,
        };
        unsafe { &*(&DEFAULT as *const ArchivedVec<'static, ()> as *const Self) }
    }
}

#[cfg(feature = "alloc")]
impl<'marker, 'a, A> Decode<'a> for ArchivedVec<'marker, A>
where
    A: Decode<'a>,
{
    type View = ArchivedVec<'a, A::View>;

    const FIELD_ENCODING: FieldEncoding = FieldEncoding::Sequence;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let len = cursor.read_u32(context)? as usize;
        let mut items = Vec::with_capacity(len);

        if let Some(_fixed_size) = A::FIXED_SIZE {
            cursor.align(A::ALIGNMENT, context)?;
            for index in 0..len {
                let mut guard = context.push_index(index);
                items.push(A::decode(cursor, &mut *guard)?);
            }
        } else {
            for index in 0..len {
                let mut guard = context.push_index(index);
                items.push(A::decode(cursor, &mut *guard)?);
            }
        }

        Ok(ArchivedVec::new(items))
    }
}

#[cfg(feature = "alloc")]
impl<T, U> Restore<Vec<U>> for ArchivedVec<'_, T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<Vec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.items.len());
        for item in &self.items {
            out.push(item.restore()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<T, U> Restore<Vec<U>> for Vec<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<Vec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.len());
        for item in self {
            out.push(item.restore()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<T> Archive for [T]
where
    T: Archive,
{
    type Archived = ArchivedVec<'static, T::Archived>;
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
