#[cfg(feature = "alloc")]
#[path = "archive/container.rs"]
mod container;

#[path = "archive/option.rs"]
mod option;
pub use option::ArchivedOption;

#[path = "archive/packed.rs"]
mod packed;
pub use packed::{
    ArchivedPackedBoolSlice, ArchivedPackedBoolSliceView, ArchivedPackedU8Slice,
    ArchivedPackedU8SliceView, PackedBoolSlice, PackedSlice, PackedU8Slice,
};

#[cfg(feature = "alloc")]
#[path = "archive/packed_vec.rs"]
mod packed_vec;
#[cfg(feature = "alloc")]
pub use packed_vec::{
    PackedBoolVec, PackedBoolVecSerializer, PackedSequenceSerializer, PackedU8Vec,
    PackedU8VecSerializer, PackedVec,
};

#[path = "archive/primitive.rs"]
mod primitive;

#[path = "archive/result.rs"]
mod result;

#[path = "archive/slice.rs"]
mod slice;

#[cfg(feature = "alloc")]
#[path = "archive/string.rs"]
mod string;

#[path = "archive/varint.rs"]
mod varint;
pub use varint::{VarInt, VarIntView};

#[cfg(feature = "alloc")]
#[path = "archive/varint_vec.rs"]
mod varint_vec;
#[cfg(feature = "alloc")]
pub use varint_vec::{PackedVarIntSlice, VarIntVec};

#[cfg(feature = "alloc")]
#[path = "archive/collections.rs"]
mod collections;

#[path = "archive/iter.rs"]
mod iter;
#[cfg(feature = "alloc")]
pub(crate) use iter::skip_block_index;
pub use iter::{ArchivedIter, ArchivedIterView, IterArchive, IterSerializer};
