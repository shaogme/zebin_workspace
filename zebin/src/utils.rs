#[doc(hidden)]
pub mod byteops;

pub(crate) fn padding_for_alignment(pos: usize, alignment: core::num::NonZeroUsize) -> usize {
    let alignment = alignment.get();
    debug_assert!(
        alignment.is_power_of_two(),
        "Alignment must be a power of two"
    );
    let mask = alignment - 1;
    alignment.wrapping_sub(pos) & mask
}
