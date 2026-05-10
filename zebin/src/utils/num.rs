use core::convert::TryFrom;
use core::num::{NonZeroU32, NonZeroUsize};

use crate::error::ValidateError;

pub(crate) fn read_fixed<const N: usize>(
    bytes: &[u8],
    pos: usize,
    field: &'static str,
) -> Result<[u8; N], ValidateError> {
    let end = pos
        .checked_add(N)
        .ok_or_else(|| ValidateError::FieldOverflow {
            field,
            pos,
            path: Default::default(),
        })?;
    let slice = bytes
        .get(pos..end)
        .ok_or_else(|| ValidateError::FieldOutOfBounds {
            field,
            pos,
            path: Default::default(),
        })?;
    let mut out = [0u8; N];
    out.copy_from_slice(slice);
    Ok(out)
}

pub(crate) fn usize_to_u32<E>(value: usize, on_err: impl FnOnce() -> E) -> Result<u32, E> {
    u32::try_from(value).map_err(|_| on_err())
}

pub(crate) fn usize_to_nonzero_u32<E>(
    value: usize,
    on_range_err: impl FnOnce() -> E,
    on_zero_err: impl FnOnce() -> E,
) -> Result<NonZeroU32, E> {
    NonZeroU32::new(usize_to_u32(value, on_range_err)?).ok_or_else(on_zero_err)
}

pub(crate) fn u32_to_usize<E>(value: u32, on_err: impl FnOnce() -> E) -> Result<usize, E> {
    usize::try_from(value).map_err(|_| on_err())
}

pub(crate) fn u32_to_nonzero_usize<E>(
    value: u32,
    on_range_err: impl FnOnce() -> E,
    on_zero_err: impl FnOnce() -> E,
) -> Result<NonZeroUsize, E> {
    NonZeroUsize::new(u32_to_usize(value, on_range_err)?).ok_or_else(on_zero_err)
}
