#[cfg(feature = "simd")]
use wide::u8x16;

#[inline]
pub fn fill(dst: &mut [u8], value: u8) {
    if dst.is_empty() {
        return;
    }

    #[cfg(feature = "simd")]
    {
        let block = u8x16::splat(value).to_array();
        let mut chunks = dst.chunks_exact_mut(16);
        for chunk in &mut chunks {
            chunk.copy_from_slice(&block);
        }

        let remainder = chunks.into_remainder();
        if !remainder.is_empty() {
            remainder.fill(value);
        }
    }

    #[cfg(not(feature = "simd"))]
    {
        dst.fill(value);
    }
}

#[inline]
pub fn copy_exact(dst: &mut [u8], src: &[u8]) {
    debug_assert_eq!(dst.len(), src.len());
    if dst.is_empty() {
        return;
    }

    #[cfg(feature = "simd")]
    {
        let mut dst_chunks = dst.chunks_exact_mut(16);
        let mut src_chunks = src.chunks_exact(16);

        for (dst_chunk, src_chunk) in dst_chunks.by_ref().zip(src_chunks.by_ref()) {
            let mut block = [0u8; 16];
            block.copy_from_slice(src_chunk);
            dst_chunk.copy_from_slice(&u8x16::new(block).to_array());
        }

        let dst_remainder = dst_chunks.into_remainder();
        let src_remainder = src_chunks.remainder();
        if !dst_remainder.is_empty() {
            dst_remainder.copy_from_slice(src_remainder);
        }
    }

    #[cfg(not(feature = "simd"))]
    {
        dst.copy_from_slice(src);
    }
}
