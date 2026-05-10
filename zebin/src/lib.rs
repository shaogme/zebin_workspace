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
    pub use crate::format::ArchiveHeader;
    pub use crate::io::sink::{ByteSink, LayoutSink};
    #[cfg(feature = "mmap")]
    pub use crate::io::storage::mmap::Mmap;
    pub use crate::traits::{
        Access, Archive, ArchiveHeader as ArchiveHeaderTrait, Layout, Serialize, SerializeState,
        Validate, archived_bytes,
    };
    pub use crate::validation::context::{
        ArchivedDepthGuard, ValidationContext, ValidationPathSegment,
    };
    pub use crate::{
        ZebinReader, ResolvedLayout, Storage, Validator, ZebinWriter, decode, encode,
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
pub use crate::format::{ARCHIVE_MAGIC, ARCHIVE_VERSION, ArchiveHeader};
pub use crate::io::sink::{ByteSink, LayoutSink};
pub use crate::io::storage::Storage;
#[cfg(feature = "mmap")]
pub use crate::io::storage::mmap::Mmap;
pub use crate::read::{ZebinReader, ResolvedLayout};
pub use crate::traits::{
    Access, Archive, ArchiveHeader as ArchiveHeaderTrait, Layout, Serialize, SerializeState,
    Validate, archived_bytes,
};
pub use crate::validation::context::{
    ArchivedDepthGuard, ValidationContext, ValidationPathSegment,
};
pub use crate::validation::validator::Validator;
pub use crate::write::{ArchiveWriter, ZebinWriter};

pub use memoffset;
pub use zebin_macros::*;

use ::core::ops::Deref;
use alloc::vec::Vec;

/// Decode and validate the archived root object using the default header.
pub fn decode<'a, T>(bytes: &'a [u8]) -> Result<ZebinReader<'a, T>, ZebinError>
where
    T: Archive,
    T::Archived: Layout + Validate + Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
{
    ZebinReader::decode(bytes)
}

/// Validate an archive without exposing the archived view using the default header.
pub fn validate<'a, T>(bytes: &'a [u8]) -> Result<(), ZebinError>
where
    T: Archive,
    T::Archived: 'a,
    T::Archived: Layout + Validate + Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
{
    ZebinReader::<T>::validate(bytes)
}

/// Create a chunked archive writer that can be resumed with caller-provided buffers.
pub fn encode_chunked<T>(value: &T) -> Result<ZebinWriter<'_, T>, ZebinError>
where
    T: Serialize + Archive,
{
    ZebinWriter::encode_chunked(value)
}

/// Archive a value into a newly allocated byte vector using the default header.
pub fn encode<T>(value: &T) -> Result<Vec<u8>, ZebinError>
where
    T: Serialize + Archive,
{
    ZebinWriter::encode(value)
}

/// Archive a value into an existing vector using the default header.
pub fn encode_into<T>(value: &T, buf: &mut Vec<u8>) -> Result<(), ZebinError>
where
    T: Serialize + Archive,
{
    ZebinWriter::encode_into(value, buf)
}
