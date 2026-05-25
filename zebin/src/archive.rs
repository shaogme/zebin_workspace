#[cfg(feature = "alloc")]
#[path = "archive/container.rs"]
mod container;

#[path = "archive/option.rs"]
mod option;

#[path = "archive/packed.rs"]
pub mod packed;

#[cfg(feature = "alloc")]
#[path = "archive/packed_vec.rs"]
pub mod packed_vec;

#[path = "archive/primitive.rs"]
mod primitive;

#[path = "archive/result.rs"]
mod result;

#[path = "archive/slice.rs"]
pub mod slice;

#[cfg(feature = "alloc")]
#[path = "archive/string.rs"]
mod string;

#[path = "archive/varint.rs"]
pub mod varint;

#[cfg(feature = "alloc")]
#[path = "archive/varint_vec.rs"]
pub mod varint_vec;

#[cfg(feature = "alloc")]
#[path = "archive/vec.rs"]
pub mod vec;

#[path = "archive/iter.rs"]
pub mod iter;
