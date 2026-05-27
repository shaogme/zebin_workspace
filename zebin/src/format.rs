use core::num::NonZeroUsize;

use crate::{error_impl::ParseHeaderError, prelude::*, utils::byteops};

/// Archive format constants and header utilities.
pub const ARCHIVE_MAGIC: [u8; 2] = *b"ZB";
pub const ARCHIVE_VERSION: u8 = 1;

/// Parsed archive header.
///
/// The root object starts immediately after this fixed header. The flags byte
/// is descriptive metadata, not a pointer to another archive section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveHeader {
    pub version: u8,
    pub flags: u8,
}

impl ArchiveHeader {
    pub const fn new(version: u8, flags: u8) -> Self {
        Self { version, flags }
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, ParseHeaderError> {
        <Self as ArchiveHeaderTrait>::parse(bytes)
    }

    pub fn to_bytes(flags: u8) -> [u8; 4] {
        let mut bytes = [0u8; 4];
        byteops::copy_exact(&mut bytes[0..2], &ARCHIVE_MAGIC);
        bytes[2] = ARCHIVE_VERSION;
        bytes[3] = flags;
        bytes
    }

    pub fn object_encoding(&self) -> Result<ObjectEncoding, ParseHeaderError> {
        ObjectEncoding::from_byte(self.flags).ok_or(ParseHeaderError::InvalidObjectEncoding {
            flags: self.flags,
            pos: 3,
        })
    }
}

impl FixedLayout for ArchiveHeader {
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();
    const SIZE: usize = 4;

    fn write_fixed(archived: &Self, out: &mut [u8]) {
        byteops::copy_exact(
            &mut out[..<Self as FixedLayout>::SIZE],
            &Self::to_bytes(archived.flags),
        );
    }
}

impl ArchiveHeaderTrait for ArchiveHeader {
    type Bytes = [u8; 4];
    const MAGIC: [u8; 2] = ARCHIVE_MAGIC;
    const VERSION: u8 = ARCHIVE_VERSION;
    const SIZE: usize = 4;

    fn parse(bytes: &[u8]) -> Result<Self, ParseHeaderError> {
        if bytes.len() < <Self as ArchiveHeaderTrait>::SIZE {
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

        let header = Self::new(version, flags);
        header.object_encoding()?;
        Ok(header)
    }

    fn flags(&self) -> u8 {
        self.flags
    }

    fn serialize(&self) -> Self::Bytes {
        Self::to_bytes(self.flags)
    }

    fn create(flags: u8) -> Self {
        Self::new(Self::VERSION, flags)
    }
}

impl ArchivedDefault for ArchiveHeader {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchiveHeader = ArchiveHeader {
            version: ARCHIVE_VERSION,
            flags: 0,
        };
        &DEFAULT
    }
}
