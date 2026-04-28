#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(any(feature = "no_std", feature = "std")))]
compile_error!(
    "Please enable at least one of the features: no_std or std. 
	Use --no-default-features flag to disable default features when you need no_std."
);

pub extern crate alloc;

mod archive;
mod core;
mod error;
mod format;
mod io;
mod read;
mod traits;
pub mod utils;
mod validation;
mod write;

pub mod prelude {
    pub use crate::archive::packed::{
        ArchivedPackedBoolSlice, ArchivedPackedU8Slice, PackedBoolSlice, PackedBoolVec,
        PackedSlice, PackedU8Slice, PackedU8Vec, PackedVec,
    };
    pub use crate::archive::varint::{PackedVarIntSlice, VarInt, VarIntVec, VarIntView};
    pub use crate::core::rel_ptr::RelPtr;
    pub use crate::core::schema::{
        FieldEncoding, LayoutDescriptor, LayoutDirectory, LayoutField, LayoutView, ObjectEncoding,
    };
    pub use crate::error::ZebinError;
    pub use crate::format::{ARCHIVE_HEADER_SIZE, ARCHIVE_MAGIC, ARCHIVE_VERSION, ArchiveHeader};
    pub use crate::io::sink::{ByteSink, LayoutSink};
    #[cfg(feature = "mmap")]
    pub use crate::io::storage::mmap::Mmap;
    pub use crate::traits::{
        Access, Archive, Layout, Serialize, SerializeState, Validate, archived_bytes,
    };
    pub use crate::validation::context::{
        ArchivedDepthGuard, ValidationContext, ValidationPathSegment,
    };
    pub use crate::{
        ArchiveView, ArchiveWriter, ResolvedLayout, Storage, Validator, decode, encode,
        encode_chunked, encode_into, validate,
    };
    pub use zebin_macros::{ZebinArchive, ZebinSerialize};
}

pub use crate::archive::packed::{
    ArchivedPackedBoolSlice, ArchivedPackedU8Slice, PackedBoolSlice, PackedBoolVec, PackedSlice,
    PackedU8Slice, PackedU8Vec, PackedVec,
};
pub use crate::archive::varint::{PackedVarIntSlice, VarInt, VarIntVec, VarIntView};
pub use crate::core::rel_ptr::RelPtr;
pub use crate::core::schema::{
    FieldEncoding, LayoutDescriptor, LayoutDirectory, LayoutField, LayoutView, ObjectEncoding,
    SchemaRevision, StableSchemaKey,
};
pub use crate::error::ZebinError;
pub use crate::format::{ARCHIVE_HEADER_SIZE, ARCHIVE_MAGIC, ARCHIVE_VERSION, ArchiveHeader};
pub use crate::io::sink::{ByteSink, LayoutSink};
pub use crate::io::storage::Storage;
#[cfg(feature = "mmap")]
pub use crate::io::storage::mmap::Mmap;
pub use crate::read::view::{ArchiveView, ResolvedLayout};
pub use crate::read::{decode, validate};
pub use crate::traits::{
    Access, Archive, Layout, Serialize, SerializeState, Validate, archived_bytes,
};
pub use crate::validation::context::{
    ArchivedDepthGuard, ValidationContext, ValidationPathSegment,
};
pub use crate::validation::validator::Validator;
pub use crate::write::{ArchiveWriter, encode, encode_chunked, encode_into};

pub use memoffset;
pub use zebin_macros::*;
