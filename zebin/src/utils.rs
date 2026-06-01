#[doc(hidden)]
pub mod byteops;

#[doc(hidden)]
pub mod macro_helpers;

pub mod cursor;

pub fn padding_for_alignment(pos: usize, alignment: core::num::NonZeroUsize) -> usize {
    let alignment = alignment.get();
    let remainder = pos % alignment;
    if remainder == 0 {
        0
    } else {
        alignment - remainder
    }
}
