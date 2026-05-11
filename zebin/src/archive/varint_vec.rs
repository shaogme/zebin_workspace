use alloc::vec::Vec;
use core::{marker::PhantomData, num::NonZeroUsize, task::Poll};

use crate::{
    archive::varint::{VarIntNumber, encode_u64, encoded_len_u64},
    core::rel_ptr::RelPtr,
    error::{AccessError, ArchiveError, ZebinError},
    read::{Cursor, ResolvedLayout},
    traits::{
        Access, Archive, ArchiveHeader, ArchivedDefault, ByteSink, Layout, LayoutSink, Restore,
        RestoreFromView, Serialize, SerializeState,
    },
    utils::num::usize_add_signed,
    validation::context::ValidationContext,
};

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

impl<T: 'static> ArchivedDefault for ArchivedVarIntVec<T> {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchivedVarIntVec<()> = ArchivedVarIntVec {
            data_ptr: None,
            offsets_ptr: None,
            len: 0,
            _marker: core::marker::PhantomData,
        };
        unsafe { &*(&DEFAULT as *const ArchivedVarIntVec<()> as *const ArchivedVarIntVec<T>) }
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

impl<'a, T: VarIntNumber + 'a> Access<'a> for ArchivedVarIntVec<T> {
    type View = &'a Self;

    unsafe fn access<H, C>(
        cursor: &mut Cursor<'a>,
        context: &mut C,
    ) -> Result<(Self::View, usize), AccessError>
    where
        H: ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let mut guard = context.guard()?;
        let pos = cursor.pos();
        guard.check_alignment(pos, Self::ALIGNMENT)?;
        guard.check_range(pos, core::mem::size_of::<Self>())?;

        let archived = unsafe { &*(cursor.bytes().as_ptr().add(pos) as *const Self) };
        if archived.len > 0 {
            let data_rel = archived.data_ptr.as_ref().ok_or_else(|| {
                guard.validation_error("Null data pointer in non-empty ArchivedVarIntVec", pos)
            })?;
            let offsets_rel = archived.offsets_ptr.as_ref().ok_or_else(|| {
                guard.validation_error("Null offsets pointer in non-empty ArchivedVarIntVec", pos)
            })?;

            let data_pos = usize_add_signed(pos, data_rel.offset(), || {
                guard.validation_error("ArchivedVarIntVec data pointer overflow", pos)
            })?;
            let offsets_pos = usize_add_signed(pos + 8, offsets_rel.offset(), || {
                guard.validation_error("ArchivedVarIntVec offsets pointer overflow", pos)
            })?;

            let offsets_len = (archived.len as usize + 1) * core::mem::size_of::<u32>();
            guard.check_range(offsets_pos, offsets_len)?;
            let offsets = &cursor.bytes()[offsets_pos..offsets_pos + offsets_len];
            let total_data_len = u32::from_le_bytes(
                offsets[offsets_len - 4..offsets_len]
                    .try_into()
                    .expect("offset table length validated"),
            ) as usize;
            guard.check_range(data_pos, total_data_len)?;
        }
        Ok((archived, core::mem::size_of::<Self>()))
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
    ) -> Result<Self::Archived, ArchiveError> {
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

pub struct VarIntVecBuilderState<'a, T: VarIntNumber> {
    values: &'a [T],
    index: usize,
    phase: VarIntVecBuilderPhase,
    data_pos: Option<usize>,
    offsets_pos: Option<usize>,
    cursor: usize,
    current_val_buf: [u8; 10],
    current_val_len: usize,
    current_total_offset: u32,
    offsets: Vec<u32>,
}

enum VarIntVecBuilderPhase {
    Data,
    Offsets,
    Done,
}

impl<'a, T: VarIntNumber> VarIntVecBuilderState<'a, T> {
    pub(crate) fn new(values: &'a [T]) -> Self {
        Self {
            values,
            index: 0,
            phase: VarIntVecBuilderPhase::Data,
            data_pos: None,
            offsets_pos: None,
            cursor: 0,
            current_val_buf: [0u8; 10],
            current_val_len: 0,
            current_total_offset: 0,
            offsets: Vec::with_capacity(values.len() + 1),
        }
    }
}

impl<'a, T: VarIntNumber> SerializeState<'a> for VarIntVecBuilderState<'a, T> {
    type Resolver = VarIntVecResolver;

    fn poll<E: ByteSink + LayoutSink<'a> + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        loop {
            match self.phase {
                VarIntVecBuilderPhase::Data => {
                    if self.data_pos.is_none() {
                        self.data_pos = Some(encoder.pos());
                    }

                    while self.index < self.values.len() {
                        if self.current_val_len == 0 {
                            self.offsets.push(self.current_total_offset);
                            let val = self.values[self.index].to_u64();
                            self.current_val_len = encoded_len_u64(val);
                            encode_u64(val, &mut self.current_val_buf[..self.current_val_len]);
                            self.cursor = 0;
                        }

                        let written = encoder
                            .write(&self.current_val_buf[self.cursor..self.current_val_len])?;
                        self.cursor += written;
                        if self.cursor < self.current_val_len {
                            return Ok(Poll::Pending);
                        }

                        self.current_total_offset += self.current_val_len as u32;
                        self.current_val_len = 0;
                        self.index += 1;
                    }

                    self.offsets.push(self.current_total_offset);
                    self.phase = VarIntVecBuilderPhase::Offsets;
                    self.cursor = 0;
                    self.index = 0;
                }
                VarIntVecBuilderPhase::Offsets => {
                    encoder.align(NonZeroUsize::new(4).unwrap())?;
                    if self.offsets_pos.is_none() {
                        self.offsets_pos = Some(encoder.pos());
                    }

                    while self.index < self.offsets.len() {
                        let bytes = self.offsets[self.index].to_le_bytes();
                        let written = encoder.write(&bytes[self.cursor..])?;
                        self.cursor += written;
                        if self.cursor < 4 {
                            return Ok(Poll::Pending);
                        }
                        self.cursor = 0;
                        self.index += 1;
                    }
                    self.phase = VarIntVecBuilderPhase::Done;
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
        = VarIntVecBuilderState<'a, T>
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
    ) -> Result<Self::Archived, ArchiveError> {
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
        = VarIntVecBuilderState<'b, T>
    where
        Self: 'b;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(VarIntVecBuilderState::new(self.values))
    }
}

pub struct VarIntArchiveState<T: VarIntNumber> {
    bytes: [u8; 10],
    len: u8,
    cursor: u8,
    _marker: PhantomData<T>,
}

impl<T: VarIntNumber> VarIntArchiveState<T> {
    pub(crate) fn new(value: T) -> Self {
        let val = value.to_u64();
        let len = encoded_len_u64(val);
        let mut bytes = [0u8; 10];
        encode_u64(val, &mut bytes[..len]);
        Self {
            bytes,
            len: len as u8,
            cursor: 0,
            _marker: PhantomData,
        }
    }
}

impl<'a, T: VarIntNumber> SerializeState<'a> for VarIntArchiveState<T> {
    type Resolver = ();

    fn poll<E: ByteSink + LayoutSink<'a> + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        let written = encoder.write(&self.bytes[self.cursor as usize..self.len as usize])?;
        self.cursor += written as u8;
        if self.cursor < self.len {
            Ok(Poll::Pending)
        } else {
            Ok(Poll::Ready(()))
        }
    }
}

impl<T> Serialize for crate::archive::varint::VarInt<T>
where
    T: VarIntNumber,
{
    type State<'a>
        = VarIntArchiveState<T>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(VarIntArchiveState::new(self.get()))
    }
}

impl<T: VarIntNumber> Restore<Vec<T>> for ArchivedVarIntVec<T> {
    fn restore(&self) -> Result<Vec<T>, ZebinError> {
        let mut out = Vec::with_capacity(self.len());
        for val in self.iter() {
            out.push(val);
        }
        Ok(out)
    }
}

impl<'a, T: VarIntNumber, H: ArchiveHeader> RestoreFromView<'a, Vec<T>, H>
    for ArchivedVarIntVec<T>
{
    fn restore_from_view(&self, _layout: &ResolvedLayout<'a, H>) -> Result<Vec<T>, ZebinError> {
        self.restore()
    }
}

impl<T: VarIntNumber> Restore<VarIntVec<T>> for ArchivedVarIntVec<T> {
    fn restore(&self) -> Result<VarIntVec<T>, ZebinError> {
        Ok(VarIntVec {
            values: self.restore()?,
        })
    }
}

impl<'a, T: VarIntNumber, H: ArchiveHeader> RestoreFromView<'a, VarIntVec<T>, H>
    for ArchivedVarIntVec<T>
{
    fn restore_from_view(
        &self,
        layout: &ResolvedLayout<'a, H>,
    ) -> Result<VarIntVec<T>, ZebinError> {
        Ok(VarIntVec {
            values: self.restore_from_view(layout)?,
        })
    }
}

impl<T, U> Restore<VarIntVec<U>> for VarIntVec<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<VarIntVec<U>, ZebinError> {
        Ok(VarIntVec {
            values: self.values.restore()?,
        })
    }
}

impl<'a, T, U, H: ArchiveHeader> RestoreFromView<'a, VarIntVec<U>, H> for VarIntVec<T>
where
    T: RestoreFromView<'a, U, H>,
{
    fn restore_from_view(
        &self,
        layout: &ResolvedLayout<'a, H>,
    ) -> Result<VarIntVec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.values.len());
        for item in &self.values {
            out.push(item.restore_from_view(layout)?);
        }
        Ok(VarIntVec { values: out })
    }
}

impl<'a, T, U> Restore<VarIntVec<U>> for PackedVarIntSlice<'a, T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<VarIntVec<U>, ZebinError> {
        Ok(VarIntVec {
            values: self.values.restore()?,
        })
    }
}

impl<'a, T, U, H: ArchiveHeader> RestoreFromView<'a, VarIntVec<U>, H> for PackedVarIntSlice<'a, T>
where
    T: RestoreFromView<'a, U, H>,
{
    fn restore_from_view(
        &self,
        layout: &ResolvedLayout<'a, H>,
    ) -> Result<VarIntVec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.values.len());
        for item in self.values {
            out.push(item.restore_from_view(layout)?);
        }
        Ok(VarIntVec { values: out })
    }
}
