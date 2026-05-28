use core::ops::{Index, IndexMut};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::error::{AccessError, ZebinError};

pub struct Buf<'a> {
    pub(crate) data: &'a [u8],
}

impl<'a> Buf<'a> {
    #[inline]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    #[inline]
    pub fn into_slice(self) -> &'a [u8] {
        self.data
    }
}

impl<'a> core::ops::Deref for Buf<'a> {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.data
    }
}

pub struct BufMut<'a> {
    pub(crate) data: &'a mut [u8],
}

impl<'a> BufMut<'a> {
    #[inline]
    pub fn new(data: &'a mut [u8]) -> Self {
        Self { data }
    }

    #[inline]
    pub fn into_mut_slice(self) -> &'a mut [u8] {
        self.data
    }
}

impl<'a> core::ops::Deref for BufMut<'a> {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<'a> core::ops::DerefMut for BufMut<'a> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data
    }
}

pub trait ChunkSource {
    fn get_buf(&self, pos: usize, len: usize) -> Result<Buf<'_>, ZebinError>;
    fn total_len(&self) -> usize;
}

pub trait ChunkSourceMut: ChunkSource {
    fn get_buf_mut(&mut self, pos: usize, len: usize) -> Result<BufMut<'_>, ZebinError>;
}

// Implement for references
impl<S: ChunkSource + ?Sized> ChunkSource for &S {
    #[inline]
    fn get_buf(&self, pos: usize, len: usize) -> Result<Buf<'_>, ZebinError> {
        (**self).get_buf(pos, len)
    }

    #[inline]
    fn total_len(&self) -> usize {
        (**self).total_len()
    }
}

impl<S: ChunkSource + ?Sized> ChunkSource for &mut S {
    #[inline]
    fn get_buf(&self, pos: usize, len: usize) -> Result<Buf<'_>, ZebinError> {
        (**self).get_buf(pos, len)
    }

    #[inline]
    fn total_len(&self) -> usize {
        (**self).total_len()
    }
}

impl<S: ChunkSourceMut + ?Sized> ChunkSourceMut for &mut S {
    #[inline]
    fn get_buf_mut(&mut self, pos: usize, len: usize) -> Result<BufMut<'_>, ZebinError> {
        (**self).get_buf_mut(pos, len)
    }
}

// Slice of slices: &'a [&'b [u8]]
impl ChunkSource for &[&[u8]] {
    #[inline]
    fn get_buf(&self, pos: usize, len: usize) -> Result<Buf<'_>, ZebinError> {
        let mut current_sum = 0;
        for chunk in self.iter() {
            let next_sum = current_sum + chunk.len();
            if pos >= current_sum && pos + len <= next_sum {
                let start = pos - current_sum;
                return Ok(Buf::new(&chunk[start..start + len]));
            }
            current_sum = next_sum;
        }
        Err(ZebinError::Access(AccessError::ValidationError {
            message: "Requested slice spans across non-contiguous chunks or out of bounds",
            pos,
        }))
    }

    #[inline]
    fn total_len(&self) -> usize {
        self.iter().map(|c| c.len()).sum()
    }
}

// Vec of slices: Vec<&'a [u8]>
#[cfg(feature = "alloc")]
impl ChunkSource for Vec<&[u8]> {
    #[inline]
    fn get_buf(&self, pos: usize, len: usize) -> Result<Buf<'_>, ZebinError> {
        let mut current_sum = 0;
        for chunk in self.iter() {
            let next_sum = current_sum + chunk.len();
            if pos >= current_sum && pos + len <= next_sum {
                let start = pos - current_sum;
                return Ok(Buf::new(&chunk[start..start + len]));
            }
            current_sum = next_sum;
        }
        Err(ZebinError::Access(AccessError::ValidationError {
            message: "Requested slice spans across non-contiguous chunks or out of bounds",
            pos,
        }))
    }

    #[inline]
    fn total_len(&self) -> usize {
        self.iter().map(|c| c.len()).sum()
    }
}

// Slice of mutable slices: &'a mut [&'b mut [u8]]
impl ChunkSource for &mut [&mut [u8]] {
    #[inline]
    fn get_buf(&self, pos: usize, len: usize) -> Result<Buf<'_>, ZebinError> {
        let mut current_sum = 0;
        for chunk in self.iter() {
            let next_sum = current_sum + chunk.len();
            if pos >= current_sum && pos + len <= next_sum {
                let start = pos - current_sum;
                return Ok(Buf::new(&chunk[start..start + len]));
            }
            current_sum = next_sum;
        }
        Err(ZebinError::Access(AccessError::ValidationError {
            message: "Requested slice spans across non-contiguous chunks or out of bounds",
            pos,
        }))
    }

    #[inline]
    fn total_len(&self) -> usize {
        self.iter().map(|c| c.len()).sum()
    }
}

impl ChunkSourceMut for &mut [&mut [u8]] {
    #[inline]
    fn get_buf_mut(&mut self, pos: usize, len: usize) -> Result<BufMut<'_>, ZebinError> {
        let mut current_sum = 0;
        for chunk in self.iter_mut() {
            let next_sum = current_sum + chunk.len();
            if pos >= current_sum && pos + len <= next_sum {
                let start = pos - current_sum;
                return Ok(BufMut::new(&mut chunk[start..start + len]));
            }
            current_sum = next_sum;
        }
        Err(ZebinError::Access(AccessError::ValidationError {
            message: "Requested slice spans across non-contiguous chunks or out of bounds",
            pos,
        }))
    }
}

// Vec of mutable slices: Vec<&'a mut [u8]>
#[cfg(feature = "alloc")]
impl ChunkSource for Vec<&mut [u8]> {
    #[inline]
    fn get_buf(&self, pos: usize, len: usize) -> Result<Buf<'_>, ZebinError> {
        let mut current_sum = 0;
        for chunk in self.iter() {
            let next_sum = current_sum + chunk.len();
            if pos >= current_sum && pos + len <= next_sum {
                let start = pos - current_sum;
                return Ok(Buf::new(&chunk[start..start + len]));
            }
            current_sum = next_sum;
        }
        Err(ZebinError::Access(AccessError::ValidationError {
            message: "Requested slice spans across non-contiguous chunks or out of bounds",
            pos,
        }))
    }

    #[inline]
    fn total_len(&self) -> usize {
        self.iter().map(|c| c.len()).sum()
    }
}

#[cfg(feature = "alloc")]
impl ChunkSourceMut for Vec<&mut [u8]> {
    #[inline]
    fn get_buf_mut(&mut self, pos: usize, len: usize) -> Result<BufMut<'_>, ZebinError> {
        let mut current_sum = 0;
        for chunk in self.iter_mut() {
            let next_sum = current_sum + chunk.len();
            if pos >= current_sum && pos + len <= next_sum {
                let start = pos - current_sum;
                return Ok(BufMut::new(&mut chunk[start..start + len]));
            }
            current_sum = next_sum;
        }
        Err(ZebinError::Access(AccessError::ValidationError {
            message: "Requested slice spans across non-contiguous chunks or out of bounds",
            pos,
        }))
    }
}

// ==========================================
// 3. ChunkedView & ChunkedViewMut
// ==========================================

#[derive(Clone)]
pub struct ChunkedView<S: ?Sized> {
    pub(crate) total_len: usize,
    pub(crate) source: S,
}

impl<S: ChunkSource + ?Sized> ChunkedView<S> {
    pub fn new(source: S) -> Self
    where
        S: Sized,
    {
        let total_len = source.total_len();
        Self { source, total_len }
    }

    pub fn new_ref(source: &S) -> ChunkedView<&S> {
        let total_len = source.total_len();
        ChunkedView { source, total_len }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.total_len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.total_len == 0
    }
}

impl<S: ChunkSource + ?Sized> Index<usize> for ChunkedView<S> {
    type Output = u8;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        let buf = self.source.get_buf(index, 1).expect("Index out of bounds");
        &buf.data[0]
    }
}

#[derive(Clone)]
pub struct ChunkedViewMut<S: ?Sized> {
    pub(crate) total_len: usize,
    pub(crate) source: S,
}

impl<S: ChunkSourceMut + ?Sized> ChunkedViewMut<S> {
    pub fn new(source: S) -> Self
    where
        S: Sized,
    {
        let total_len = source.total_len();
        Self { source, total_len }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.total_len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.total_len == 0
    }
}

impl<S: ChunkSourceMut + ?Sized> Index<usize> for ChunkedViewMut<S> {
    type Output = u8;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        let buf = self.source.get_buf(index, 1).expect("Index out of bounds");
        &buf.data[0]
    }
}

impl<S: ChunkSourceMut + ?Sized> IndexMut<usize> for ChunkedViewMut<S> {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        let buf = self
            .source
            .get_buf_mut(index, 1)
            .expect("Index out of bounds");
        &mut buf.into_mut_slice()[0]
    }
}
