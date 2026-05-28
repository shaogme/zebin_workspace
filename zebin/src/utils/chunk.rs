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
    fn total_len(&self) -> usize;
}

pub trait ChunkSourceMut: ChunkSource {
    /// 获取指定索引的可写分块
    fn get_chunk_mut(&mut self, idx: usize) -> Option<&mut [u8]>;

    /// 在指定位置写入数据，默认实现会利用 get_chunk_mut 写入。支持扩容的源可以重写此方法。
    fn write_at(&mut self, pos: usize, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        mutate_at(
            self,
            pos,
            bytes.len(),
            |chunk_slice, bytes_written, step_len| {
                byteops::copy_exact(chunk_slice, &bytes[bytes_written..bytes_written + step_len]);
            },
        )
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
        mutate_at(self, pos, len, |chunk_slice, _, _| {
            byteops::fill(chunk_slice, 0);
        })
    }
}

fn mutate_at<S: ChunkSourceMut + ?Sized>(
    source: &mut S,
    pos: usize,
    len: usize,
    mut f: impl FnMut(&mut [u8], usize, usize),
) -> Result<SinkProgress, ZebinError> {
    if len == 0 {
        return Ok(SinkProgress::Complete);
    }
    let total_len = source.total_len();
    if pos >= total_len {
        return Ok(SinkProgress::Blocked);
    }
    let available = total_len - pos;
    let target_len = len.min(available);
    if target_len == 0 {
        return Ok(SinkProgress::Blocked);
    }

    let mut current_sum = 0;
    let mut bytes_processed = 0;
    for idx in 0..source.chunk_count() {
        if let Some(chunk_len) = source.get_chunk(idx).map(|c| c.len()) {
            let next_sum = current_sum + chunk_len;
            if pos + bytes_processed >= current_sum && pos + bytes_processed < next_sum {
                let chunk_offset = (pos + bytes_processed) - current_sum;
                let chunk_avail = chunk_len - chunk_offset;
                let step_len = (target_len - bytes_processed).min(chunk_avail);
                if step_len > 0 {
                    if let Some(chunk_mut) = source.get_chunk_mut(idx) {
                        f(
                            &mut chunk_mut[chunk_offset..chunk_offset + step_len],
                            bytes_processed,
                            step_len,
                        );
                    }
                    bytes_processed += step_len;
                }
            }
            current_sum = next_sum;
        }
        if bytes_processed >= target_len {
            break;
        }
    }
    Ok(SinkProgress::from_accepted(len, bytes_processed))
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

    #[inline]
    fn total_len(&self) -> usize {
        (**self).total_len()
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

    #[inline]
    fn total_len(&self) -> usize {
        (**self).total_len()
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

    #[inline]
    fn total_len(&self) -> usize {
        self.iter().map(|c| c.len()).sum()
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

    #[inline]
    fn total_len(&self) -> usize {
        self.iter().map(|c| c.len()).sum()
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

    #[inline]
    fn total_len(&self) -> usize {
        self.iter().map(|c| c.len()).sum()
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

    #[inline]
    fn total_len(&self) -> usize {
        self.iter().map(|c| c.len()).sum()
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
        translate_address_impl(&self.source, self.total_len, &self.last_chunk, global_idx)
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
        translate_address_impl(&self.source, self.total_len, &self.last_chunk, global_idx)
    }
}

#[inline]
fn translate_address_impl<S: ChunkSource + ?Sized>(
    source: &S,
    total_len: usize,
    last_chunk: &Cell<usize>,
    global_idx: usize,
) -> Option<(usize, usize)> {
    if global_idx >= total_len {
        return None;
    }

    // 1. 快速路径：使用缓存加速连续访问
    let last = last_chunk.get();
    if last < source.chunk_count() {
        let mut chunk_start = 0;
        for i in 0..last {
            if let Some(chunk) = source.get_chunk(i) {
                chunk_start += chunk.len();
            }
        }
        if let Some(chunk) = source.get_chunk(last) {
            let chunk_end = chunk_start + chunk.len();
            if global_idx >= chunk_start && global_idx < chunk_end {
                return Some((last, global_idx - chunk_start));
            }
        }
    }

    // 2. 慢速路径：从头遍历累加定位分块
    let mut current_sum = 0;
    for idx in 0..source.chunk_count() {
        if let Some(chunk) = source.get_chunk(idx) {
            let next_sum = current_sum + chunk.len();
            if global_idx >= current_sum && global_idx < next_sum {
                last_chunk.set(idx);
                return Some((idx, global_idx - current_sum));
            }
            current_sum = next_sum;
        }
    }

    None
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
