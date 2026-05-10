#[cfg(feature = "alloc")]
mod container;
mod option;
pub mod packed;
#[cfg(feature = "alloc")]
pub mod packed_vec;
mod primitive;
mod result;
pub mod slice;
#[cfg(feature = "alloc")]
mod string;
pub mod varint;
#[cfg(feature = "alloc")]
pub mod varint_vec;
#[cfg(feature = "alloc")]
pub mod vec;
