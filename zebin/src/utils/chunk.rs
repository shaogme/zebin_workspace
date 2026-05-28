use core::cell::Cell;
use core::num::NonZeroUsize;
use core::ops::{Index, IndexMut};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::error::ZebinError;
use crate::traits_impl::SinkProgress;
use crate::utils::{byteops, padding_for_alignment};

pub trait ChunkSource {
    /// 获取分块总数
    fn chunk_count(&self) -> usize;
    /// 获取指定索引的只读分块
    fn get_chunk(&self, idx: usize) -> Option<&[u8]>;

    /// 计算所有分块的总长度
    fn total_len(&self) -> usize {
        let mut len = 0;
        let mut idx = 0;
        while let Some(chunk) = self.get_chunk(idx) {
            len += chunk.len();
            idx += 1;
        }
        len
    }
}

pub trait ChunkSourceMut: ChunkSource {
    /// 获取指定索引的可写分块
    fn get_chunk_mut(&mut self, idx: usize) -> Option<&mut [u8]>;

    /// 在指定位置写入数据，默认实现会利用 get_chunk_mut 写入。支持扩容的源可以重写此方法。
    fn write_at(&mut self, pos: usize, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        if bytes.is_empty() {
            return Ok(SinkProgress::Complete);
        }
        let total_len = self.total_len();
        if pos >= total_len {
            return Ok(SinkProgress::Blocked);
        }
        let available = total_len - pos;
        let to_write = bytes.len().min(available);
        if to_write == 0 {
            return Ok(SinkProgress::Blocked);
        }

        let mut current_sum = 0;
        let mut bytes_written = 0;
        for idx in 0..self.chunk_count() {
            if let Some(chunk_len) = self.get_chunk(idx).map(|c| c.len()) {
                let next_sum = current_sum + chunk_len;
                if pos + bytes_written >= current_sum && pos + bytes_written < next_sum {
                    let chunk_offset = (pos + bytes_written) - current_sum;
                    let chunk_avail = chunk_len - chunk_offset;
                    let write_len = (to_write - bytes_written).min(chunk_avail);
                    if write_len > 0 {
                        if let Some(chunk_mut) = self.get_chunk_mut(idx) {
                            byteops::copy_exact(
                                &mut chunk_mut[chunk_offset..chunk_offset + write_len],
                                &bytes[bytes_written..bytes_written + write_len],
                            );
                        }
                        bytes_written += write_len;
                    }
                }
                current_sum = next_sum;
            }
            if bytes_written >= to_write {
                break;
            }
        }
        Ok(SinkProgress::from_accepted(bytes.len(), bytes_written))
    }

    fn align_at(
        &mut self,
        pos: usize,
        alignment: NonZeroUsize,
    ) -> Result<SinkProgress, ZebinError> {
        let padding = padding_for_alignment(pos, alignment);
        self.skip_at(pos, padding)
    }

    fn skip_at(&mut self, pos: usize, len: usize) -> Result<SinkProgress, ZebinError> {
        if len == 0 {
            return Ok(SinkProgress::Complete);
        }
        let total_len = self.total_len();
        if pos >= total_len {
            return Ok(SinkProgress::Blocked);
        }
        let available = total_len - pos;
        let to_skip = len.min(available);
        if to_skip == 0 {
            return Ok(SinkProgress::Blocked);
        }

        let mut current_sum = 0;
        let mut bytes_skipped = 0;
        for idx in 0..self.chunk_count() {
            if let Some(chunk_len) = self.get_chunk(idx).map(|c| c.len()) {
                let next_sum = current_sum + chunk_len;
                if pos + bytes_skipped >= current_sum && pos + bytes_skipped < next_sum {
                    let chunk_offset = (pos + bytes_skipped) - current_sum;
                    let chunk_avail = chunk_len - chunk_offset;
                    let skip_len = (to_skip - bytes_skipped).min(chunk_avail);
                    if skip_len > 0 {
                        if let Some(chunk_mut) = self.get_chunk_mut(idx) {
                            byteops::fill(&mut chunk_mut[chunk_offset..chunk_offset + skip_len], 0);
                        }
                        bytes_skipped += skip_len;
                    }
                }
                current_sum = next_sum;
            }
            if bytes_skipped >= to_skip {
                break;
            }
        }
        Ok(SinkProgress::from_accepted(len, bytes_skipped))
    }
}

// Implement for references
impl<S: ChunkSource + ?Sized> ChunkSource for &S {
    #[inline]
    fn chunk_count(&self) -> usize {
        (**self).chunk_count()
    }

    #[inline]
    fn get_chunk(&self, idx: usize) -> Option<&[u8]> {
        (**self).get_chunk(idx)
    }
}

impl<S: ChunkSource + ?Sized> ChunkSource for &mut S {
    #[inline]
    fn chunk_count(&self) -> usize {
        (**self).chunk_count()
    }

    #[inline]
    fn get_chunk(&self, idx: usize) -> Option<&[u8]> {
        (**self).get_chunk(idx)
    }
}

impl<S: ChunkSourceMut + ?Sized> ChunkSourceMut for &mut S {
    #[inline]
    fn get_chunk_mut(&mut self, idx: usize) -> Option<&mut [u8]> {
        (**self).get_chunk_mut(idx)
    }

    #[inline]
    fn write_at(&mut self, pos: usize, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        (**self).write_at(pos, bytes)
    }

    #[inline]
    fn align_at(
        &mut self,
        pos: usize,
        alignment: NonZeroUsize,
    ) -> Result<SinkProgress, ZebinError> {
        (**self).align_at(pos, alignment)
    }

    #[inline]
    fn skip_at(&mut self, pos: usize, len: usize) -> Result<SinkProgress, ZebinError> {
        (**self).skip_at(pos, len)
    }
}

// Slice of slices: &'a [&'b [u8]]
impl ChunkSource for &[&[u8]] {
    #[inline]
    fn chunk_count(&self) -> usize {
        self.len()
    }

    #[inline]
    fn get_chunk(&self, idx: usize) -> Option<&[u8]> {
        self.get(idx).copied()
    }
}

// Vec of slices: Vec<&'a [u8]>
#[cfg(feature = "alloc")]
impl ChunkSource for Vec<&[u8]> {
    #[inline]
    fn chunk_count(&self) -> usize {
        self.len()
    }

    #[inline]
    fn get_chunk(&self, idx: usize) -> Option<&[u8]> {
        self.get(idx).copied()
    }
}

// Slice of mutable slices: &'a mut [&'b mut [u8]]
impl ChunkSource for &mut [&mut [u8]] {
    #[inline]
    fn chunk_count(&self) -> usize {
        self.len()
    }

    #[inline]
    fn get_chunk(&self, idx: usize) -> Option<&[u8]> {
        self.get(idx).map(|s| &**s)
    }
}

impl ChunkSourceMut for &mut [&mut [u8]] {
    #[inline]
    fn get_chunk_mut(&mut self, idx: usize) -> Option<&mut [u8]> {
        self.get_mut(idx).map(|s| &mut **s)
    }
}

// Vec of mutable slices: Vec<&'a mut [u8]>
#[cfg(feature = "alloc")]
impl ChunkSource for Vec<&mut [u8]> {
    #[inline]
    fn chunk_count(&self) -> usize {
        self.len()
    }

    #[inline]
    fn get_chunk(&self, idx: usize) -> Option<&[u8]> {
        self.get(idx).map(|s| &**s)
    }
}

#[cfg(feature = "alloc")]
impl ChunkSourceMut for Vec<&mut [u8]> {
    #[inline]
    fn get_chunk_mut(&mut self, idx: usize) -> Option<&mut [u8]> {
        self.get_mut(idx).map(|s| &mut **s)
    }
}

// ==========================================
// 3. ChunkedView & ChunkedViewMut
// ==========================================

#[derive(Clone)]
pub struct ChunkedView<S: ?Sized> {
    pub(crate) total_len: usize,
    pub(crate) last_chunk: Cell<usize>,
    pub(crate) source: S,
}

impl<S: ChunkSource + ?Sized> ChunkedView<S> {
    pub fn new(source: S) -> Self
    where
        S: Sized,
    {
        let total_len = source.total_len();
        Self {
            source,
            total_len,
            last_chunk: Cell::new(0),
        }
    }

    pub fn new_ref(source: &S) -> ChunkedView<&S> {
        let total_len = source.total_len();
        ChunkedView {
            source,
            total_len,
            last_chunk: Cell::new(0),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.total_len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.total_len == 0
    }

    #[inline]
    pub fn translate_address(&self, global_idx: usize) -> Option<(usize, usize)> {
        if global_idx >= self.total_len {
            return None;
        }

        // 1. 快速路径：使用缓存加速连续访问
        let last = self.last_chunk.get();
        if last < self.source.chunk_count() {
            let mut chunk_start = 0;
            for i in 0..last {
                if let Some(chunk) = self.source.get_chunk(i) {
                    chunk_start += chunk.len();
                }
            }
            if let Some(chunk) = self.source.get_chunk(last) {
                let chunk_end = chunk_start + chunk.len();
                if global_idx >= chunk_start && global_idx < chunk_end {
                    return Some((last, global_idx - chunk_start));
                }
            }
        }

        // 2. 慢速路径：从头遍历累加定位分块
        let mut current_sum = 0;
        for idx in 0..self.source.chunk_count() {
            if let Some(chunk) = self.source.get_chunk(idx) {
                let next_sum = current_sum + chunk.len();
                if global_idx >= current_sum && global_idx < next_sum {
                    self.last_chunk.set(idx);
                    return Some((idx, global_idx - current_sum));
                }
                current_sum = next_sum;
            }
        }

        None
    }
}

impl<S: ChunkSource + ?Sized> Index<usize> for ChunkedView<S> {
    type Output = u8;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        let (chunk_idx, local_idx) = self.translate_address(index).expect("Index out of bounds");
        &self.source.get_chunk(chunk_idx).unwrap()[local_idx]
    }
}

#[derive(Clone)]
pub struct ChunkedViewMut<S: ?Sized> {
    pub(crate) total_len: usize,
    pub(crate) last_chunk: Cell<usize>,
    pub(crate) source: S,
}

impl<S: ChunkSourceMut + ?Sized> ChunkedViewMut<S> {
    pub fn new(source: S) -> Self
    where
        S: Sized,
    {
        let total_len = source.total_len();
        Self {
            source,
            total_len,
            last_chunk: Cell::new(0),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.total_len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.total_len == 0
    }

    #[inline]
    fn translate_address(&self, global_idx: usize) -> Option<(usize, usize)> {
        if global_idx >= self.total_len {
            return None;
        }

        let last = self.last_chunk.get();
        if last < self.source.chunk_count() {
            let mut chunk_start = 0;
            for i in 0..last {
                if let Some(chunk) = self.source.get_chunk(i) {
                    chunk_start += chunk.len();
                }
            }
            if let Some(chunk) = self.source.get_chunk(last) {
                let chunk_end = chunk_start + chunk.len();
                if global_idx >= chunk_start && global_idx < chunk_end {
                    return Some((last, global_idx - chunk_start));
                }
            }
        }

        let mut current_sum = 0;
        for idx in 0..self.source.chunk_count() {
            if let Some(chunk) = self.source.get_chunk(idx) {
                let next_sum = current_sum + chunk.len();
                if global_idx >= current_sum && global_idx < next_sum {
                    self.last_chunk.set(idx);
                    return Some((idx, global_idx - current_sum));
                }
                current_sum = next_sum;
            }
        }

        None
    }
}

impl<S: ChunkSourceMut + ?Sized> Index<usize> for ChunkedViewMut<S> {
    type Output = u8;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        let (chunk_idx, local_idx) = self.translate_address(index).expect("Index out of bounds");
        &self.source.get_chunk(chunk_idx).unwrap()[local_idx]
    }
}

impl<S: ChunkSourceMut + ?Sized> IndexMut<usize> for ChunkedViewMut<S> {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        let (chunk_idx, local_idx) = self.translate_address(index).expect("Index out of bounds");
        &mut self.source.get_chunk_mut(chunk_idx).unwrap()[local_idx]
    }
}
