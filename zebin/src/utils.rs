#[doc(hidden)]
pub mod byteops;

#[doc(hidden)]
pub mod macros_helpers;

pub(crate) fn padding_for_alignment(pos: usize, alignment: core::num::NonZeroUsize) -> usize {
    let alignment = alignment.get();
    let remainder = pos % alignment;
    if remainder == 0 {
        0
    } else {
        alignment - remainder
    }
}
