#[cfg(feature = "alloc")]
mod container;
mod option;
#[cfg(feature = "alloc")]
pub mod packed;
mod primitive;
mod result;
#[cfg(feature = "alloc")]
mod string;
#[cfg(feature = "alloc")]
pub mod varint;
#[cfg(feature = "alloc")]
pub mod vec;
