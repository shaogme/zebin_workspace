use core::convert::TryFrom;
use core::num::{NonZeroU32, NonZeroUsize};

use crate::traits::ZebinError;

pub(crate) fn usize_to_u32(
    value: usize,
    on_err: impl FnOnce() -> ZebinError,
) -> Result<u32, ZebinError> {
    u32::try_from(value).map_err(|_| on_err())
}

pub(crate) fn usize_to_nonzero_u32(
    value: usize,
    on_range_err: impl FnOnce() -> ZebinError,
    on_zero_err: impl FnOnce() -> ZebinError,
) -> Result<NonZeroU32, ZebinError> {
    NonZeroU32::new(usize_to_u32(value, on_range_err)?).ok_or_else(on_zero_err)
}

pub(crate) fn u32_to_usize(
    value: u32,
    on_err: impl FnOnce() -> ZebinError,
) -> Result<usize, ZebinError> {
    usize::try_from(value).map_err(|_| on_err())
}

pub(crate) fn u32_to_nonzero_usize(
    value: u32,
    on_range_err: impl FnOnce() -> ZebinError,
    on_zero_err: impl FnOnce() -> ZebinError,
) -> Result<NonZeroUsize, ZebinError> {
    NonZeroUsize::new(u32_to_usize(value, on_range_err)?).ok_or_else(on_zero_err)
}
