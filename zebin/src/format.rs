use core::num::NonZeroU32;

use alloc::{format, string::ToString};

use crate::{ZebinError, utils::num::read_fixed};

/// Archive format constants and header utilities.
pub const ARCHIVE_MAGIC: [u8; 2] = *b"ZB";
pub const ARCHIVE_VERSION: u8 = 1;
pub const ARCHIVE_HEADER_SIZE: usize = 12;

/// Parsed archive header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveHeader {
    pub version: u8,
    pub flags: u8,
    pub layout_offset: NonZeroU32,
    pub root_offset: NonZeroU32,
}

impl ArchiveHeader {
    pub const fn new(
        version: u8,
        flags: u8,
        layout_offset: NonZeroU32,
        root_offset: NonZeroU32,
    ) -> Self {
        Self {
            version,
            flags,
            layout_offset,
            root_offset,
        }
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, ZebinError> {
        if bytes.len() < ARCHIVE_HEADER_SIZE {
            return Err(ZebinError::ValidationError {
                message: "Header too short".to_string(),
                pos: 0,
            });
        }
        if bytes[0..2] != ARCHIVE_MAGIC {
            return Err(ZebinError::ValidationError {
                message: "Invalid magic".to_string(),
                pos: 0,
            });
        }

        let version = bytes[2];
        let flags = bytes[3];
        if version != ARCHIVE_VERSION {
            return Err(ZebinError::ValidationError {
                message: format!("Unsupported archive version {}", version),
                pos: 2,
            });
        }

        let layout_offset = NonZeroU32::new(u32::from_le_bytes(read_fixed::<4>(
            bytes,
            4,
            "Layout offset",
        )?))
        .ok_or_else(|| ZebinError::ValidationError {
            message: "Layout offset cannot be zero".to_string(),
            pos: 4,
        })?;
        let root_offset = NonZeroU32::new(u32::from_le_bytes(read_fixed::<4>(
            bytes,
            8,
            "Root offset",
        )?))
        .ok_or_else(|| ZebinError::ValidationError {
            message: "Root offset cannot be zero".to_string(),
            pos: 8,
        })?;

        Ok(Self::new(version, flags, layout_offset, root_offset))
    }

    pub fn to_bytes(
        flags: u8,
        layout_offset: NonZeroU32,
        root_offset: NonZeroU32,
    ) -> [u8; ARCHIVE_HEADER_SIZE] {
        let mut bytes = [0u8; ARCHIVE_HEADER_SIZE];
        bytes[0..2].copy_from_slice(&ARCHIVE_MAGIC);
        bytes[2] = ARCHIVE_VERSION;
        bytes[3] = flags;
        bytes[4..8].copy_from_slice(&layout_offset.get().to_le_bytes());
        bytes[8..12].copy_from_slice(&root_offset.get().to_le_bytes());
        bytes
    }
}
