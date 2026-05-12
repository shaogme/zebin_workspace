use crate::{error::DecodeError, read::Cursor, validation::context::ValidationContext};

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

    pub fn decode<'a, C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let entry_pos = cursor.pos();

        let field_id = cursor.read_u16(context)?;
        let encoding_byte = cursor.read_u8(context)?;
        let reserved = cursor.read_u8(context)?;
        let payload_len = cursor.read_u32(context)?;

        let encoding = FieldEncoding::from_byte(encoding_byte)
            .ok_or_else(|| context.validation_error("Unknown field encoding", entry_pos + 2))?;

        if reserved != Self::RESERVED {
            return Err(context.error(DecodeError::InvalidFieldTable { pos: entry_pos + 3 }));
        }

        Ok(Self {
            field_id,
            encoding,
            payload_len,
        })
    }
}

pub const MAX_SCHEMA_FIELDS: usize = 128;

/// Common header for schema-aware objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SchemaObjectHeader {
    pub stable_schema_key: StableSchemaKey,
    pub schema_revision: SchemaRevision,
    pub field_count: u16,
}

impl SchemaObjectHeader {
    pub fn decode_and_verify<'a, C>(
        cursor: &mut Cursor<'a>,
        context: &mut C,
        expected_key: StableSchemaKey,
    ) -> Result<Self, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let object_start = cursor.pos();
        let stable_schema_key = cursor.read_u32(context)?;
        if stable_schema_key != expected_key {
            return Err(context.validation_error("Stable schema key mismatch", object_start));
        }

        let schema_revision = cursor.read_u32(context)?;
        let field_count = cursor.read_u16(context)?;
        let reserved_pos = cursor.pos();
        let reserved = cursor.read_u16(context)?;

        if reserved != 0 {
            return Err(context.error(DecodeError::InvalidFieldTable { pos: reserved_pos }));
        }

        if field_count as usize > MAX_SCHEMA_FIELDS {
            return Err(context.error(DecodeError::InvalidFieldTable { pos: object_start }));
        }

        Ok(Self {
            stable_schema_key,
            schema_revision,
            field_count,
        })
    }
}
