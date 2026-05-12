use crate::{error::AccessError, read::Cursor, validation::context::ValidationContext};

/// Stable identifier used to refer to a schema across archive revisions.
pub type StableSchemaKey = u32;

/// Monotonic schema revision for a stable schema key.
pub type SchemaRevision = u32;

/// Object-level encoding family stored in the archive header flags.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ObjectEncoding {
    #[default]
    Fixed = 0,
    SchemaAware = 1,
    VarInt = 2,
    Packed = 3,
    Sequence = 4,
}

impl ObjectEncoding {
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Fixed),
            1 => Some(Self::SchemaAware),
            2 => Some(Self::VarInt),
            3 => Some(Self::Packed),
            4 => Some(Self::Sequence),
            _ => None,
        }
    }
}

/// Field-level encoding family stored in schema-aware object field entries.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FieldEncoding {
    #[default]
    Fixed = 0,
    VarInt = 1,
    PackedBits = 2,
    LengthPrefixed = 3,
    SchemaAware = 4,
    Sequence = 5,
}

impl FieldEncoding {
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Fixed),
            1 => Some(Self::VarInt),
            2 => Some(Self::PackedBits),
            3 => Some(Self::LengthPrefixed),
            4 => Some(Self::SchemaAware),
            5 => Some(Self::Sequence),
            _ => None,
        }
    }
}

/// One self-describing field entry inside a schema-aware object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FieldEntry {
    pub field_id: u16,
    pub encoding: FieldEncoding,
    pub payload_len: u32,
}

impl FieldEntry {
    pub const EMPTY: Self = Self {
        field_id: 0,
        encoding: FieldEncoding::Fixed,
        payload_len: 0,
    };

    /// Wire layout: `field_id: u16 LE`, `encoding: u8`, `reserved: u8`,
    /// `payload_len: u32 LE`.
    pub const SIZE: usize = 8;
    pub const RESERVED: u8 = 0;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut bytes = [0u8; Self::SIZE];
        bytes[0..2].copy_from_slice(&self.field_id.to_le_bytes());
        bytes[2] = self.encoding as u8;
        bytes[3] = Self::RESERVED;
        bytes[4..8].copy_from_slice(&self.payload_len.to_le_bytes());
        bytes
    }

    pub fn decode<'a, C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let field_id = cursor.read_u16(context)?;
        let encoding_pos = cursor.pos();
        let encoding_byte = cursor.read_u8(context)?;
        let encoding = FieldEncoding::from_byte(encoding_byte)
            .ok_or_else(|| context.validation_error("Unknown field encoding", encoding_pos))?;
        let reserved_pos = cursor.pos();
        let reserved = cursor.read_u8(context)?;
        if reserved != Self::RESERVED {
            return Err(AccessError::InvalidFieldTable { pos: reserved_pos });
        }
        let payload_len = cursor.read_u32(context)?;
        Ok(Self {
            field_id,
            encoding,
            payload_len,
        })
    }
}

pub const MAX_SCHEMA_FIELDS: usize = 128;
