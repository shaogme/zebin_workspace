use alloc::{string::ToString, vec, vec::Vec};
use core::{marker::PhantomData, num::NonZeroUsize, ops::Deref, task::Poll};

use crate::{
    core::rel_ptr::RelPtr,
    core::schema::ObjectEncoding,
    error::ZebinError,
    io::sink::{ByteSink, LayoutSink},
    traits::{Access, Archive, Layout, Serialize, SerializeState, Validate},
    validation::context::ValidationContext,
};

/// Unsigned integers that are serialized with a variable-length encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VarInt<T> {
    value: T,
}

impl<T> VarInt<T> {
    pub fn new(value: T) -> Self {
        Self { value }
    }

    pub fn get(&self) -> T
    where
        T: Copy,
    {
        self.value
    }

    pub fn into_inner(self) -> T {
        self.value
    }
}

/// Borrowed view over a decoded varint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VarIntView<T> {
    value: T,
}

impl<T> VarIntView<T> {
    pub fn get(&self) -> T
    where
        T: Copy,
    {
        self.value
    }
}

impl<T> Deref for VarIntView<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

pub trait VarIntNumber: Copy {
    type Archived: Layout + Validate + Copy + Send + Sync + 'static;
    const MAX_BYTES: usize;

    fn to_u64(self) -> u64;
    fn try_from_u64(value: u64) -> Result<Self, ZebinError>;
    fn from_archived(archived: Self::Archived) -> Self;
    fn to_archived(self) -> Self::Archived;
}

macro_rules! impl_varint_number {
    ($t:ty, $archived:ident, $max_bytes:expr) => {
        #[repr(C)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $archived {
            bytes: [u8; $max_bytes],
        }

        impl $archived {
            pub fn get(self) -> $t {
                let mut value = 0u64;
                let mut shift = 0u32;
                for &byte in &self.bytes {
                    let payload = u64::from(byte & 0x7F);
                    value |= payload << shift;
                    if byte & 0x80 == 0 {
                        break;
                    }
                    shift += 7;
                }
                value as $t
            }
        }

        impl Layout for $archived {
            const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();
            const ENCODING: ObjectEncoding = ObjectEncoding::VarInt;

            fn size_hint(&self) -> usize {
                encoded_len_u64(self.get() as u64)
            }

            fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
                encode_u64(archived.get() as u64, out);
            }
        }

        impl Validate for $archived {
            unsafe fn validate<H, C>(ptr: *const Self, context: &mut C) -> Result<(), ZebinError>
            where
                H: crate::traits::ArchiveHeader,
                C: ValidationContext<H> + ?Sized,
            {
                let mut guard = context.guard()?;
                guard.check_range(ptr as *const u8, $max_bytes)?;
                Ok(())
            }
        }

        impl<'a> Access<'a> for $archived {
            type View = &'a Self;

            unsafe fn access<H, C>(
                ptr: *const u8,
                context: &mut C,
            ) -> Result<(Self::View, usize), ZebinError>
            where
                H: crate::traits::ArchiveHeader,
                C: ValidationContext<H> + ?Sized,
            {
                let typed_ptr = ptr as *const Self;
                unsafe {
                    <Self as Validate>::validate::<H, C>(typed_ptr, context)?;
                }
                Ok((unsafe { &*typed_ptr }, $max_bytes))
            }
        }

        impl VarIntNumber for $t {
            type Archived = $archived;
            const MAX_BYTES: usize = $max_bytes;

            fn to_u64(self) -> u64 {
                self as u64
            }

            fn try_from_u64(value: u64) -> Result<Self, ZebinError> {
                <$t>::try_from(value).map_err(|_| ZebinError::ValidationError {
                    message: "VarInt value out of range".to_string(),
                    pos: 0,
                })
            }

            fn from_archived(archived: Self::Archived) -> Self {
                archived.get()
            }

            fn to_archived(self) -> Self::Archived {
                let mut bytes = [0u8; $max_bytes];
                encode_u64(self as u64, &mut bytes);
                $archived { bytes }
            }
        }
    };
}

impl_varint_number!(u8, ArchivedVarIntU8, 2);
impl_varint_number!(u16, ArchivedVarIntU16, 3);
impl_varint_number!(u32, ArchivedVarIntU32, 5);
impl_varint_number!(u64, ArchivedVarIntU64, 10);
impl_varint_number!(usize, ArchivedVarIntUsize, 10);

fn encoded_len_u64(value: u64) -> usize {
    let mut value = value;
    let mut len = 1usize;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

fn encode_u64(mut value: u64, out: &mut [u8]) {
    let mut cursor = 0usize;
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out[cursor] = byte;
        cursor += 1;
        if value == 0 {
            break;
        }
        if cursor >= out.len() {
            break;
        }
    }
}

fn decode_u64<T, H, C>(bytes: *const u8, context: &mut C) -> Result<(T, usize), ZebinError>
where
    T: VarIntNumber,
    H: crate::traits::ArchiveHeader,
    C: ValidationContext<H> + ?Sized,
{
    let mut guard = context.guard()?;
    let mut value = 0u64;
    let mut shift = 0u32;
    let mut consumed = 0usize;
    loop {
        if consumed >= T::MAX_BYTES {
            return Err(ZebinError::ValidationError {
                message: "VarInt exceeds maximum length".to_string(),
                pos: bytes as usize,
            });
        }
        let byte_ptr = unsafe { bytes.add(consumed) };
        guard.check_range(byte_ptr, 1)?;
        let byte = unsafe { *byte_ptr };
        let payload = u64::from(byte & 0x7F);
        value |= payload << shift;
        consumed += 1;
        if byte & 0x80 == 0 {
            let value = T::try_from_u64(value)?;
            return Ok((value, consumed));
        }
        shift += 7;
        if shift >= 64 {
            return Err(ZebinError::ValidationError {
                message: "VarInt shift overflow".to_string(),
                pos: bytes as usize,
            });
        }
    }
}

impl<T> Layout for VarInt<T>
where
    T: VarIntNumber,
{
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();
    const ENCODING: ObjectEncoding = ObjectEncoding::VarInt;

    fn size_hint(&self) -> usize {
        encoded_len_u64(self.value.to_u64())
    }

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        encode_u64(archived.value.to_u64(), out);
    }
}

impl<T> Validate for VarInt<T>
where
    T: VarIntNumber,
{
    unsafe fn validate<H, C>(ptr: *const Self, context: &mut C) -> Result<(), ZebinError>
    where
        H: crate::traits::ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let (view, _) = unsafe { <Self as Access>::access::<H, C>(ptr as *const u8, context)? };
        let _ = view.get();
        Ok(())
    }
}

impl<'a, T: VarIntNumber + 'a> Access<'a> for VarInt<T>
where
    T: VarIntNumber,
{
    type View = VarIntView<T>;

    unsafe fn access<H, C>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), ZebinError>
    where
        H: crate::traits::ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let (value, consumed) = decode_u64::<T, H, C>(ptr, context)?;
        Ok((VarIntView { value }, consumed))
    }
}

impl<T> Archive for VarInt<T>
where
    T: VarIntNumber,
{
    type Archived = T::Archived;
    type Resolver = ();

    fn resolve(
        &self,
        _archive_pos: usize,
        _resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        Ok(self.value.to_archived())
    }
}

pub struct VarIntArchiveState<T: VarIntNumber> {
    bytes: Vec<u8>,
    cursor: usize,
    _marker: PhantomData<T>,
}

impl<T: VarIntNumber> VarIntArchiveState<T> {
    fn new(value: T) -> Self {
        let mut bytes = vec![0u8; encoded_len_u64(value.to_u64())];
        encode_u64(value.to_u64(), &mut bytes);
        Self {
            bytes,
            cursor: 0,
            _marker: PhantomData,
        }
    }
}

impl<T> Serialize for VarInt<T>
where
    T: VarIntNumber,
{
    type State<'a>
        = VarIntArchiveState<T>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(VarIntArchiveState::new(self.value))
    }
}

impl<T: VarIntNumber> SerializeState for VarIntArchiveState<T> {
    type Resolver = ();

    fn poll<E: ByteSink + LayoutSink + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        let written = encoder.write(&self.bytes[self.cursor..])?;
        self.cursor += written;
        if self.cursor < self.bytes.len() {
            Ok(Poll::Pending)
        } else {
            Ok(Poll::Ready(()))
        }
    }
}

/// A compact vector of VarInts that uses an offset table for O(1) access.
pub struct VarIntVec<T> {
    pub values: Vec<T>,
}

impl<T> VarIntVec<T> {
    pub fn new(values: Vec<T>) -> Self {
        Self { values }
    }
}

impl<T> From<Vec<T>> for VarIntVec<T> {
    fn from(values: Vec<T>) -> Self {
        Self::new(values)
    }
}

/// A compact slice of VarInts.
pub struct PackedVarIntSlice<'a, T> {
    pub values: &'a [T],
}

impl<'a, T> PackedVarIntSlice<'a, T> {
    pub fn new(values: &'a [T]) -> Self {
        Self { values }
    }
}

/// Archived version of VarIntVec using an offset table for O(1) access.
#[repr(C)]
pub struct ArchivedVarIntVec<T> {
    data_ptr: Option<RelPtr<u8>>,
    offsets_ptr: Option<RelPtr<u32>>,
    len: u32,
    _marker: PhantomData<T>,
}

impl<T: VarIntNumber> ArchivedVarIntVec<T> {
    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> Option<T> {
        if index >= self.len() {
            return None;
        }

        let offsets_ptr = self.offsets_ptr.as_ref()?;
        let data_ptr = self.data_ptr.as_ref()?;

        unsafe {
            let offsets = core::slice::from_raw_parts(offsets_ptr.as_ptr(), self.len() + 1);
            let start = offsets[index] as usize;
            let end = offsets[index + 1] as usize;

            // We need a context for decoding, but since we've already validated...
            // decode_u64 requires a context. We can use a dummy one or a safe decoder.
            let mut value = 0u64;
            let mut shift = 0u32;
            let data = data_ptr.as_ptr().add(start);
            for i in 0..(end - start) {
                let byte = *data.add(i);
                let payload = u64::from(byte & 0x7F);
                value |= payload << shift;
                if byte & 0x80 == 0 {
                    break;
                }
                shift += 7;
            }
            T::try_from_u64(value).ok()
        }
    }

    pub fn iter(&self) -> VarIntVecIter<'_, T> {
        VarIntVecIter {
            vec: self,
            index: 0,
        }
    }
}

pub struct VarIntVecIter<'a, T: VarIntNumber> {
    vec: &'a ArchivedVarIntVec<T>,
    index: usize,
}

impl<'a, T: VarIntNumber> Iterator for VarIntVecIter<'a, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        let val = self.vec.get(self.index)?;
        self.index += 1;
        Some(val)
    }
}

impl<T: VarIntNumber> Layout for ArchivedVarIntVec<T> {
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(8).unwrap();

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        crate::utils::byteops::fill(out, 0);
        if let Some(ptr) = &archived.data_ptr {
            out[0..8].copy_from_slice(&ptr.offset().to_le_bytes());
        }
        if let Some(ptr) = &archived.offsets_ptr {
            out[8..16].copy_from_slice(&ptr.offset().to_le_bytes());
        }
        out[16..20].copy_from_slice(&archived.len.to_le_bytes());
    }
}

impl<T: VarIntNumber> Validate for ArchivedVarIntVec<T> {
    unsafe fn validate<H, C>(ptr: *const Self, context: &mut C) -> Result<(), ZebinError>
    where
        H: crate::traits::ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let mut guard = context.guard()?;
        guard.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        guard.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;

        let archived = unsafe { &*ptr };
        if archived.len > 0 {
            let data_ptr = archived.data_ptr.as_ref().ok_or(ZebinError::WriteError)?;
            let offsets_ptr = archived
                .offsets_ptr
                .as_ref()
                .ok_or(ZebinError::WriteError)?;

            unsafe {
                guard.check_range(
                    offsets_ptr.as_ptr() as *const u8,
                    (archived.len as usize + 1) * 4,
                )?;

                let offsets =
                    core::slice::from_raw_parts(offsets_ptr.as_ptr(), archived.len as usize + 1);
                let total_data_len = offsets[archived.len as usize] as usize;

                guard.check_range(data_ptr.as_ptr(), total_data_len)?;
            }
        }
        Ok(())
    }
}

impl<'a, T: VarIntNumber + 'a> Access<'a> for ArchivedVarIntVec<T> {
    type View = &'a Self;

    unsafe fn access<H, C>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), ZebinError>
    where
        H: crate::traits::ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let typed_ptr = ptr as *const Self;
        unsafe {
            <Self as Validate>::validate::<H, C>(typed_ptr, context)?;
        }
        Ok((unsafe { &*typed_ptr }, core::mem::size_of::<Self>()))
    }
}

pub struct VarIntVecResolver {
    data_pos: usize,
    offsets_pos: usize,
}

impl<T: VarIntNumber> Archive for VarIntVec<T> {
    type Archived = ArchivedVarIntVec<T>;
    type Resolver = VarIntVecResolver;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        Ok(ArchivedVarIntVec {
            data_ptr: if self.values.is_empty() {
                None
            } else {
                Some(RelPtr::new(archive_pos, resolver.data_pos)?)
            },
            offsets_ptr: if self.values.is_empty() {
                None
            } else {
                Some(RelPtr::new(archive_pos + 8, resolver.offsets_pos)?)
            },
            len: self.values.len() as u32,
            _marker: PhantomData,
        })
    }
}

pub struct VarIntVecBuilderState<T: VarIntNumber> {
    data: Vec<u8>,
    offsets: Vec<u32>,
    phase: VarIntVecBuilderPhase,
    data_pos: Option<usize>,
    offsets_pos: Option<usize>,
    cursor: usize,
    _marker: PhantomData<T>,
}

enum VarIntVecBuilderPhase {
    Data,
    Offsets,
    Done,
}

impl<T: VarIntNumber> VarIntVecBuilderState<T> {
    fn new(values: &[T]) -> Self {
        let mut data = Vec::new();
        let mut offsets = Vec::with_capacity(values.len() + 1);
        let mut current_offset = 0u32;

        for &val in values {
            offsets.push(current_offset);
            let val_u64 = val.to_u64();
            let len = encoded_len_u64(val_u64);
            let mut buf = vec![0u8; len];
            encode_u64(val_u64, &mut buf);
            data.extend_from_slice(&buf);
            current_offset += len as u32;
        }
        offsets.push(current_offset);

        Self {
            data,
            offsets,
            phase: VarIntVecBuilderPhase::Data,
            data_pos: None,
            offsets_pos: None,
            cursor: 0,
            _marker: PhantomData,
        }
    }
}

impl<T: VarIntNumber> SerializeState for VarIntVecBuilderState<T> {
    type Resolver = VarIntVecResolver;

    fn poll<E: ByteSink + LayoutSink + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        loop {
            match self.phase {
                VarIntVecBuilderPhase::Data => {
                    if self.data_pos.is_none() {
                        self.data_pos = Some(encoder.pos());
                    }
                    let written = encoder.write(&self.data[self.cursor..])?;
                    self.cursor += written;
                    if self.cursor >= self.data.len() {
                        self.phase = VarIntVecBuilderPhase::Offsets;
                        self.cursor = 0;
                    } else {
                        return Ok(Poll::Pending);
                    }
                }
                VarIntVecBuilderPhase::Offsets => {
                    encoder.align(NonZeroUsize::new(4).unwrap())?;
                    if self.offsets_pos.is_none() {
                        self.offsets_pos = Some(encoder.pos());
                    }

                    let mut offset_bytes = Vec::with_capacity(self.offsets.len() * 4);
                    for &off in &self.offsets {
                        offset_bytes.extend_from_slice(&off.to_le_bytes());
                    }

                    let written = encoder.write(&offset_bytes[self.cursor..])?;
                    self.cursor += written;
                    if self.cursor >= offset_bytes.len() {
                        self.phase = VarIntVecBuilderPhase::Done;
                    } else {
                        return Ok(Poll::Pending);
                    }
                }
                VarIntVecBuilderPhase::Done => {
                    return Ok(Poll::Ready(VarIntVecResolver {
                        data_pos: self.data_pos.expect("data pos set"),
                        offsets_pos: self.offsets_pos.expect("offsets pos set"),
                    }));
                }
            }
        }
    }
}

impl<T: VarIntNumber> Serialize for VarIntVec<T> {
    type State<'a>
        = VarIntVecBuilderState<T>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(VarIntVecBuilderState::new(&self.values))
    }
}

impl<'a, T: VarIntNumber> Archive for PackedVarIntSlice<'a, T> {
    type Archived = ArchivedVarIntVec<T>;
    type Resolver = VarIntVecResolver;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        Ok(ArchivedVarIntVec {
            data_ptr: if self.values.is_empty() {
                None
            } else {
                Some(RelPtr::new(archive_pos, resolver.data_pos)?)
            },
            offsets_ptr: if self.values.is_empty() {
                None
            } else {
                Some(RelPtr::new(archive_pos + 8, resolver.offsets_pos)?)
            },
            len: self.values.len() as u32,
            _marker: PhantomData,
        })
    }
}

impl<'a, T: VarIntNumber> Serialize for PackedVarIntSlice<'a, T> {
    type State<'b>
        = VarIntVecBuilderState<T>
    where
        Self: 'b;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(VarIntVecBuilderState::new(self.values))
    }
}
