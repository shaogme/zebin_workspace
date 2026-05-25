#[cfg(feature = "alloc")]
extern crate alloc;

#[path = "archive.rs"]
mod archive_impl;

#[path = "core.rs"]
mod core_impl;

#[path = "error.rs"]
mod error_impl;

#[path = "format.rs"]
mod format_impl;

#[path = "io.rs"]
mod io_impl;

#[path = "read.rs"]
mod read_impl;

#[path = "traits.rs"]
mod traits_impl;

#[path = "validation.rs"]
mod validation_impl;

mod write;

pub mod utils;

pub mod prelude {
    #[cfg(feature = "alloc")]
    pub use crate::archive::{PackedBoolVec, PackedU8Vec, PackedVarIntSlice, PackedVec, VarIntVec};
    #[cfg(feature = "mmap")]
    pub use crate::io::{Mmap, MmapEncoder, MmapMut};
    pub use crate::{
        archive::{
            ArchivedPackedBoolSlice, ArchivedPackedU8Slice, IterArchive, PackedBoolSlice,
            PackedSlice, PackedU8Slice, VarInt, VarIntView,
        },
        error::{ArchiveError, DecodeError, ZebinError},
        io::{
            Archive, ArchiveHeader, ArchiveHeaderTrait, ArchivedDefault, ArchivedLayout, ByteSink,
            Cursor, Decode, Encode, Encoder, FixedLayout, Restore, SchemaAware, SinkProgress,
            Storage, ZebinReader, ZebinWriter, decode, encode_chunked, reader,
        },
        schema::{
            FieldEncoding, FieldEntry, FieldTableReader, ObjectEncoding, SchemaRevision,
            StableSchemaKey,
        },
        validation::{
            ValidationConfig, ValidationContext, ValidationPathStack, Validator, validate,
            validate_detailed, validate_with_config,
        },
    };
    pub use zebin_macros::{ZebinArchive, ZebinEncode};
}

// --- 子模块结构 (方案 2) ---

/// 底层表示与数据存储结构相关的高级 API
pub mod archive {
    pub use crate::archive_impl::{
        iter::{ArchivedIter, IterArchive, IterEncoder},
        packed::{
            ArchivedPackedBoolSlice, ArchivedPackedBoolSliceView, ArchivedPackedU8Slice,
            ArchivedPackedU8SliceView, PackedBoolSlice, PackedSlice, PackedU8Slice,
        },
        varint::{VarInt, VarIntView},
    };
    #[cfg(feature = "alloc")]
    pub use crate::archive_impl::{
        packed_vec::{PackedBoolVec, PackedSequenceEncoder, PackedU8Vec, PackedVec},
        varint_vec::{PackedVarIntSlice, VarIntVec},
    };
}

/// 数据校验控制与上下文
pub mod validation {
    pub use crate::pub_fn::{validate, validate_detailed, validate_with_config};
    pub use crate::validation_impl::{
        context::{ArchivedDepthGuard, ValidationContext},
        path::{ValidationPathSegment, ValidationPathStack},
        validator::{ValidationConfig, Validator},
    };
}

/// 架构（Schema）分析与解析
pub mod schema {
    pub use crate::core_impl::schema::{
        FieldEncoding, FieldEntry, FieldTableReader, MAX_SCHEMA_FIELDS, ObjectEncoding,
        SchemaObjectHeader, SchemaRevision, StableSchemaKey, process_field_table,
        process_trailing_field_table,
    };
}

/// 底层 I/O、流、编码器与自定义头部
pub mod io {
    pub use crate::format_impl::{ARCHIVE_MAGIC, ARCHIVE_VERSION, ArchiveHeader};
    pub use crate::io_impl::storage::Storage;
    #[cfg(feature = "mmap")]
    pub use crate::io_impl::storage::mmap::{Mmap, MmapMut};
    pub use crate::pub_fn::{decode, encode_chunked, reader};
    #[cfg(feature = "alloc")]
    pub use crate::pub_fn::{encode, encode_into};
    pub use crate::read_impl::{Cursor, ZebinReader};
    pub use crate::traits_impl::{
        Archive, ArchiveHeader as ArchiveHeaderTrait, ArchivedDefault, ArchivedLayout, ByteSink,
        Decode, Encode, Encoder, FixedLayout, Restore, SchemaAware, SinkProgress,
    };
    #[cfg(feature = "alloc")]
    pub use crate::traits_impl::{
        BackwardSequenceStrategy, FixedSequenceStrategy, ForwardSequenceStrategy,
        SequenceDecodeStrategy,
    };
    #[cfg(feature = "mmap")]
    pub use crate::write::encoder::MmapEncoder;
    pub use crate::write::{ArchiveWriter, ZebinWriter};
}

/// 详细的解码和序列化错误定义
pub mod error {
    pub use crate::error_impl::*;
}

// --- 根目录暴露的常用核心门面 API ---
pub use crate::error::ZebinError;
pub use crate::read_impl::ZebinReader;
pub use crate::traits_impl::{Archive, Decode, Encode};
pub use crate::write::ZebinWriter;

pub use memoffset;
pub use zebin_macros::*;

pub use pub_fn::*;

mod pub_fn {
    use super::prelude::*;

    /// Measure the body length a value will occupy when serialized without the archive header.
    pub fn measure_serialized_len<T>(value: &T) -> Result<usize, ZebinError>
    where
        T: Encode + Archive + ?Sized,
    {
        crate::write::measure_body_len(value)
    }

    /// Create a reader for the archived root object using the default header.
    pub fn reader<'a, T>(bytes: &'a [u8]) -> Result<ZebinReader<'a, T>, ZebinError>
    where
        T: Archive,
        T::Archived: Decode<'a>,
    {
        ZebinReader::new(bytes, ValidationConfig::default())
    }

    /// Decode and validate the archived root object using the default header directly into T.
    pub fn decode<'a, T>(bytes: &'a [u8]) -> Result<T, ZebinError>
    where
        T: Archive,
        T::Archived: Decode<'a>,
        <T::Archived as Decode<'a>>::View: Restore<T>,
    {
        ZebinReader::<T>::decode(bytes)
    }

    /// Validate an archive without exposing the archived view using the default header.
    pub fn validate<'a, T>(bytes: &'a [u8]) -> Result<(), ZebinError>
    where
        T: Archive,
        T::Archived: Decode<'a>,
    {
        ZebinReader::<T>::validate(bytes, ValidationConfig::default(), None)
    }

    /// Validate an archive with explicit runtime validation limits.
    pub fn validate_with_config<'a, T>(
        bytes: &'a [u8],
        config: ValidationConfig,
        stack: Option<&mut ValidationPathStack>,
    ) -> Result<(), ZebinError>
    where
        T: Archive,
        T::Archived: Decode<'a>,
    {
        ZebinReader::<T>::validate(bytes, config, stack)
    }

    /// Validate an archive and capture the logical field/index path on failure.
    pub fn validate_detailed<'a, T>(
        bytes: &'a [u8],
        stack: &mut ValidationPathStack,
    ) -> Result<(), ZebinError>
    where
        T: Archive,
        T::Archived: Decode<'a>,
    {
        ZebinReader::<T>::validate(bytes, ValidationConfig::default(), Some(stack))
    }

    /// Create a chunked archive writer that can be resumed with caller-provided buffers.
    pub fn encode_chunked<T>(value: &T) -> Result<ZebinWriter<'_, T>, ZebinError>
    where
        T: Encode + Archive,
        T::Archived: ArchivedLayout,
    {
        ZebinWriter::encode_chunked(value)
    }

    #[cfg(feature = "alloc")]
    use alloc::vec::Vec;

    /// Archive a value into a newly allocated byte vector using the default header.
    #[cfg(feature = "alloc")]
    pub fn encode<T>(value: &T) -> Result<Vec<u8>, ZebinError>
    where
        T: Encode + Archive,
        T::Archived: ArchivedLayout,
    {
        ZebinWriter::encode(value)
    }

    /// Archive a value into an existing vector using the default header.
    #[cfg(feature = "alloc")]
    pub fn encode_into<T>(value: &T, buf: &mut Vec<u8>) -> Result<(), ZebinError>
    where
        T: Encode + Archive,
        T::Archived: ArchivedLayout,
    {
        ZebinWriter::encode_into(value, buf)
    }
}
