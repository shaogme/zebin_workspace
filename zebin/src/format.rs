use core::num::NonZeroU32;

use crate::{
    error::ParseHeaderError,
    traits::{ArchiveHeader as ArchiveHeaderTrait, Layout},
};
use core::num::NonZeroUsize;

/// Archive format constants and header utilities.
pub const ARCHIVE_MAGIC: [u8; 2] = *b"ZB";
pub const ARCHIVE_VERSION: u8 = 1;

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
}

impl Layout for ArchiveHeader {
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(4).unwrap();

    fn size_hint(&self) -> usize {
        Self::SIZE
    }

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        let bytes = Self::to_bytes(archived.flags, archived.layout_offset, archived.root_offset);
        out[..Self::SIZE].copy_from_slice(&bytes);
    }
}

impl ArchiveHeaderTrait for ArchiveHeader {
    type Bytes = [u8; 12];
    const MAGIC: [u8; 2] = ARCHIVE_MAGIC;
    const VERSION: u8 = ARCHIVE_VERSION;
    const SIZE: usize = 12;

    fn parse(bytes: &[u8]) -> Result<Self, ParseHeaderError> {
        if bytes.len() < Self::SIZE {
            return Err(ParseHeaderError::TooShort { pos: 0 });
        }
        if bytes[0..2] != Self::MAGIC {
            return Err(ParseHeaderError::InvalidMagic { pos: 0 });
        }

        let version = bytes[2];
        let flags = bytes[3];
        if version != Self::VERSION {
            return Err(ParseHeaderError::UnsupportedVersion { version, pos: 2 });
        }

        let layout_offset_bytes: [u8; 4] = bytes[4..8].try_into().unwrap();
        let layout_offset = NonZeroU32::new(u32::from_le_bytes(layout_offset_bytes))
            .ok_or(ParseHeaderError::InvalidLayoutOffset { pos: 4 })?;

        let root_offset_bytes: [u8; 4] = bytes[8..12].try_into().unwrap();
        let root_offset = NonZeroU32::new(u32::from_le_bytes(root_offset_bytes))
            .ok_or(ParseHeaderError::InvalidRootOffset { pos: 8 })?;

        Ok(Self::new(version, flags, layout_offset, root_offset))
    }

    fn flags(&self) -> u8 {
        self.flags
    }

    fn encode(&self) -> Self::Bytes {
        Self::to_bytes(self.flags, self.layout_offset, self.root_offset)
    }

    fn create(flags: u8, layout_offset: NonZeroU32, root_offset: NonZeroU32) -> Self {
        Self::new(Self::VERSION, flags, layout_offset, root_offset)
    }

    fn layout_offset(&self) -> NonZeroU32 {
        self.layout_offset
    }

    fn root_offset(&self) -> NonZeroU32 {
        self.root_offset
    }
}

impl ArchiveHeader {
    pub fn to_bytes(flags: u8, layout_offset: NonZeroU32, root_offset: NonZeroU32) -> [u8; 12] {
        let mut bytes = [0u8; 12];
        bytes[0..2].copy_from_slice(&ARCHIVE_MAGIC);
        bytes[2] = ARCHIVE_VERSION;
        bytes[3] = flags;
        bytes[4..8].copy_from_slice(&layout_offset.get().to_le_bytes());
        bytes[8..12].copy_from_slice(&root_offset.get().to_le_bytes());
        bytes
    }
}

impl crate::traits::ArchivedDefault for ArchiveHeader {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchiveHeader = ArchiveHeader {
            version: ARCHIVE_VERSION,
            flags: 0,
            layout_offset: NonZeroU32::new(12).unwrap(),
            root_offset: NonZeroU32::new(16).unwrap(),
        };
        &DEFAULT
    }
}
