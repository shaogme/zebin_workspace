#[cfg(feature = "alloc")]
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
    pub use crate::archive::{
        packed::{
            ArchivedPackedBoolSlice, ArchivedPackedU8Slice, PackedBoolSlice, PackedSlice,
            PackedU8Slice,
        },
        varint::{VarInt, VarIntView},
    };
    #[cfg(feature = "alloc")]
    pub use crate::archive::{
        packed_vec::{PackedBoolVec, PackedU8Vec, PackedVec},
        varint_vec::{PackedVarIntSlice, VarIntVec},
    };
    pub use crate::core::rel_ptr::RelPtr;
    pub use crate::core::schema::{
        FieldEncoding, LayoutDescriptor, LayoutDirectory, LayoutField, LayoutView, ObjectEncoding,
    };
    pub use crate::error::{AccessError, ArchiveError, ValidateError, ZebinError};
    pub use crate::format::ArchiveHeader;
    #[cfg(feature = "mmap")]
    pub use crate::io::storage::mmap::Mmap;
    pub use crate::traits::{
        Access, Archive, ArchiveHeader as ArchiveHeaderTrait, ArchivedDefault, ByteSink, Layout,
        LayoutSink, OptRestorer, OptRestorerFallback, OptRestorerOption, Restore, RestoreFromView,
        SchemaAware, Serialize, SerializeState, Validate,
    };
    pub use crate::validation::context::{ArchivedDepthGuard, ValidationContext};
    pub use crate::{
        ResolvedLayout, Storage, Validator, View, ZebinReader, ZebinWriter, decode, encode_chunked,
        reader, validate,
    };
    pub use zebin_macros::{ZebinArchive, ZebinSerialize};
}

pub use crate::archive::{
    packed::{
        ArchivedPackedBoolSlice, ArchivedPackedU8Slice, PackedBoolSlice, PackedSlice, PackedU8Slice,
    },
    varint::{VarInt, VarIntView},
};
#[cfg(feature = "alloc")]
pub use crate::archive::{
    packed_vec::{PackedBoolVec, PackedSequenceState, PackedU8Vec, PackedVec},
    varint_vec::PackedVarIntSlice,
    vec::archived_bytes,
};
pub use crate::core::rel_ptr::RelPtr;
pub use crate::core::schema::{
    FieldEncoding, LayoutDescriptor, LayoutDirectory, LayoutField, LayoutView, ObjectEncoding,
    SchemaRevision, StableSchemaKey,
};
pub use crate::error::{AccessError, ArchiveError, ValidateError, ZebinError};
pub use crate::format::{ARCHIVE_MAGIC, ARCHIVE_VERSION, ArchiveHeader};
pub use crate::io::storage::Storage;
#[cfg(feature = "mmap")]
pub use crate::io::storage::mmap::Mmap;
pub use crate::read::{ResolvedLayout, View, ZebinReader, get_nested_layout};
pub use crate::traits::{
    Access, Archive, ArchiveHeader as ArchiveHeaderTrait, ArchivedDefault, ByteSink, Layout,
    LayoutSink, OptRestorer, OptRestorerFallback, OptRestorerOption, Restore, RestoreFromView,
    SchemaAware, Serialize, SerializeState, Validate,
};
pub use crate::validation::context::{ArchivedDepthGuard, ValidationContext};
pub use crate::validation::validator::{ValidationPathSegment, ValidationPathStack, Validator};
pub use crate::write::{ArchiveWriter, ZebinWriter};

pub use memoffset;
pub use zebin_macros::*;

use ::core::ops::Deref;

/// Create a reader for the archived root object using the default header.
pub fn reader<'a, T>(bytes: &'a [u8]) -> Result<ZebinReader<'a, T>, ZebinError>
where
    T: Archive,
    T::Archived: Layout + Validate + Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
{
    ZebinReader::new(bytes)
}

/// Decode and validate the archived root object using the default header directly into T.
pub fn decode<'a, T>(bytes: &'a [u8]) -> Result<T, ZebinError>
where
    T: Archive,
    T::Archived: Layout + Validate + Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
    View<'a, <T::Archived as Access<'a>>::View, ArchiveHeader>: Restore<T>,
{
    ZebinReader::<T>::decode(bytes)
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
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Archive a value into a newly allocated byte vector using the default header.
#[cfg(feature = "alloc")]
pub fn encode<T>(value: &T) -> Result<Vec<u8>, ZebinError>
where
    T: Serialize + Archive,
{
    ZebinWriter::encode(value)
}

/// Archive a value into an existing vector using the default header.
#[cfg(feature = "alloc")]
pub fn encode_into<T>(value: &T, buf: &mut Vec<u8>) -> Result<(), ZebinError>
where
    T: Serialize + Archive,
{
    ZebinWriter::encode_into(value, buf)
}
