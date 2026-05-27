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

pub mod utils;
mod write;

pub mod prelude {
    #[cfg(feature = "alloc")]
    pub use crate::archive::{PackedBoolVec, PackedU8Vec, PackedVarIntSlice, PackedVec, VarIntVec};
    #[cfg(feature = "alloc")]
    pub use crate::io::VecEncoder;
    #[cfg(feature = "mmap")]
    pub use crate::io::{Mmap, MmapEncoder, MmapMut};
    pub use crate::{
        archive::{
            ArchivedPackedBoolSlice, ArchivedPackedU8Slice, IterArchive, PackedBoolSlice,
            PackedSlice, PackedU8Slice, VarInt, VarIntView,
        },
        error::{AccessError, ArchiveError, ZebinError},
        io::{
            Access, Archive, ArchiveHeader, ArchiveHeaderTrait, ArchivedDefault, ArchivedField,
            ArchivedLayout, Cursor, Deserialize, Encode, Encoder, FixedLayout, MeasureBody,
            NoSharder, SchemaAware, Sharder, SinkProgress, SliceEncoder, StaticMode, Storage,
            StorageMode, StorageMut, StreamMode, ZebinReader, ZebinWriter, deserialize, reader,
            writer,
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
        ArchivedIter, ArchivedOption, ArchivedPackedBoolSlice, ArchivedPackedBoolSliceView,
        ArchivedPackedU8Slice, ArchivedPackedU8SliceView, IterArchive, IterEncoder,
        PackedBoolSlice, PackedSlice, PackedU8Slice, VarInt, VarIntView,
    };
    #[cfg(feature = "alloc")]
    pub use crate::archive_impl::{
        PackedBoolVec, PackedBoolVecEncoder, PackedSequenceEncoder, PackedU8Vec,
        PackedU8VecEncoder, PackedVarIntSlice, PackedVec, VarIntVec,
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
    pub use crate::io_impl::storage::SliceEncoder;
    #[cfg(feature = "alloc")]
    pub use crate::io_impl::storage::VecEncoder;
    #[cfg(feature = "mmap")]
    pub use crate::io_impl::storage::mmap::MmapEncoder;
    #[cfg(feature = "mmap")]
    pub use crate::io_impl::storage::mmap::{Mmap, MmapMut};
    pub use crate::io_impl::storage::{
        NoSharder, Sharder, StaticMode, Storage, StorageMode, StorageMut, StreamMode,
    };
    pub use crate::pub_fn::{deserialize, reader, writer};
    #[cfg(feature = "alloc")]
    pub use crate::pub_fn::{encode, encode_into};
    pub use crate::read_impl::ZebinReader;
    pub use crate::traits_impl::{
        Access, Archive, ArchiveHeader as ArchiveHeaderTrait, ArchivedDefault, ArchivedField,
        ArchivedLayout, Deserialize, Encode, Encoder, FixedLayout, MeasureBody, SchemaAware,
        SinkProgress,
    };
    #[cfg(feature = "alloc")]
    pub use crate::traits_impl::{
        BackwardSequenceStrategy, FixedSequenceStrategy, ForwardSequenceStrategy,
        SequenceAccessStrategy,
    };
    pub use crate::utils::cursor::Cursor;
    pub use crate::write::{ArchiveWriter, ZebinWriter};
}

/// 详细的解码和序列化错误定义
pub mod error {
    pub use crate::error_impl::*;
}

pub use crate::archive_impl::ArchivedOption;
pub use crate::error::ZebinError;
pub use crate::read_impl::ZebinReader;
pub use crate::traits_impl::{Access, Archive, Encode, MeasureBody};
pub use crate::write::ZebinWriter;

pub use memoffset;
pub use zebin_macros::*;

pub use pub_fn::*;

mod pub_fn {
    use super::prelude::*;
    /// Create a reader for the archived root object using the default header.
    pub fn reader<'a, T, S>(storage: S) -> Result<ZebinReader<'a, T, S>, ZebinError>
    where
        T: Archive,
        S: Storage,
        T::Archived: Access,
    {
        ZebinReader::new(storage, ValidationConfig::default())
    }

    /// Access the archived root object using the default header.
    pub fn access<'a, T, S>(storage: &'a S) -> Result<<T::Archived as Access>::View<'a>, ZebinError>
    where
        T: Archive + 'a,
        S: Storage<Mode = StaticMode> + 'a,
        T::Archived: Access,
    {
        ZebinReader::<T, S>::access(storage, ValidationConfig::default())
    }

    /// Decode and validate the archived root object using the default header directly into T.
    pub fn deserialize<'a, T, S>(storage: S) -> Result<T, ZebinError>
    where
        T: Archive + 'a,
        S: Storage + 'a,
        T::Archived: Access + 'a,
        for<'b> <T::Archived as Access>::View<'b>: Deserialize<T>,
    {
        ZebinReader::<T, S>::deserialize(storage)
    }

    /// Validate an archive without exposing the archived view using the default header.
    pub fn validate<'a, T, S>(storage: &'a S) -> Result<(), ZebinError>
    where
        T: Archive,
        S: Storage + 'a,
        T::Archived: Access,
    {
        ZebinReader::<T, S>::validate(storage, ValidationConfig::default(), None)
    }

    /// Validate an archive with explicit runtime validation limits.
    pub fn validate_with_config<'a, T, S>(
        storage: &'a S,
        config: ValidationConfig,
        stack: Option<&mut ValidationPathStack>,
    ) -> Result<(), ZebinError>
    where
        T: Archive,
        S: Storage + 'a,
        T::Archived: Access,
    {
        ZebinReader::<T, S>::validate(storage, config, stack)
    }

    /// Validate an archive and capture the logical field/index path on failure.
    pub fn validate_detailed<'a, T, S>(
        storage: &'a S,
        stack: &mut ValidationPathStack,
    ) -> Result<(), ZebinError>
    where
        T: Archive,
        S: Storage + 'a,
        T::Archived: Access,
    {
        ZebinReader::<T, S>::validate(storage, ValidationConfig::default(), Some(stack))
    }

    /// Create a chunked archive writer that can be resumed with caller-provided buffers.
    pub fn writer<'a, T, S>(sink: S) -> Result<ZebinWriter<'a, T, S>, ZebinError>
    where
        T: Encode + Archive + 'a,
        S: StorageMut,
        T::Archived: ArchivedLayout,
    {
        ZebinWriter::new(sink)
    }

    #[cfg(feature = "alloc")]
    use alloc::vec::Vec;

    /// Archive a value into a newly allocated byte vector using the default header.
    #[cfg(feature = "alloc")]
    pub fn encode<'a, T>(value: T) -> Result<Vec<u8>, ZebinError>
    where
        T: Encode + Archive + 'a,
        T::Archived: ArchivedLayout,
        T: Encode<Input<'a> = T>,
    {
        ZebinWriter::<'a, T, VecEncoder>::encode(value)
    }

    /// Archive a value into an existing vector using the default header.
    #[cfg(feature = "alloc")]
    pub fn encode_into<'a, T>(value: T, buf: &mut Vec<u8>) -> Result<(), ZebinError>
    where
        T: Encode + Archive + 'a,
        T::Archived: ArchivedLayout,
        T: Encode<Input<'a> = T>,
    {
        ZebinWriter::<'a, T, VecEncoder>::encode_into(value, buf)
    }
}
