use core::ops::{Index, IndexMut};

use crate::error::ZebinError;

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
    fn is_eof(&self, pos: usize) -> bool;
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
    fn is_eof(&self, pos: usize) -> bool {
        (**self).is_eof(pos)
    }
}

impl<S: ChunkSource + ?Sized> ChunkSource for &mut S {
    #[inline]
    fn get_buf(&self, pos: usize, len: usize) -> Result<Buf<'_>, ZebinError> {
        (**self).get_buf(pos, len)
    }

    #[inline]
    fn is_eof(&self, pos: usize) -> bool {
        (**self).is_eof(pos)
    }
}

impl<S: ChunkSourceMut + ?Sized> ChunkSourceMut for &mut S {
    #[inline]
    fn get_buf_mut(&mut self, pos: usize, len: usize) -> Result<BufMut<'_>, ZebinError> {
        (**self).get_buf_mut(pos, len)
    }
}

// ==========================================
// 3. ChunkedView & ChunkedViewMut
// ==========================================

#[derive(Clone)]
pub struct ChunkedView<S: ?Sized> {
    pub(crate) source: S,
}

impl<S: ChunkSource + ?Sized> ChunkedView<S> {
    pub fn new(source: S) -> Self
    where
        S: Sized,
    {
        Self { source }
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
    pub(crate) source: S,
}

impl<S: ChunkSourceMut + ?Sized> ChunkedViewMut<S> {
    pub fn new(source: S) -> Self
    where
        S: Sized,
    {
        Self { source }
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
