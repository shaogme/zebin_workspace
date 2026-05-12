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
        let bytes = cursor.read_array::<{ Self::SIZE }, C>(context)?;

        let field_id = u16::from_le_bytes([bytes[0], bytes[1]]);
        let encoding_byte = bytes[2];
        let reserved = bytes[3];
        let payload_len = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);

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

    pub fn check_decodable<C>(
        &self,
        entry_pos: usize,
        expected_encoding: FieldEncoding,
        already_seen: bool,
        context: &mut C,
    ) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        if self.encoding != expected_encoding {
            return Err(context.error(DecodeError::UnexpectedFieldEncoding {
                field_id: self.field_id,
                expected: expected_encoding,
                actual: self.encoding,
                pos: entry_pos,
            }));
        }
        if already_seen {
            return Err(context.error(DecodeError::DuplicateField {
                field_id: self.field_id,
                pos: entry_pos,
            }));
        }
        Ok(())
    }

    pub fn check_payload_len<C>(
        &self,
        entry_pos: usize,
        consumed: usize,
        context: &mut C,
    ) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        if consumed != self.payload_len as usize {
            return Err(context.error(DecodeError::FieldLengthMismatch {
                field_id: self.field_id,
                expected: self.payload_len as usize,
                actual: consumed,
                pos: entry_pos,
            }));
        }
        Ok(())
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
        let bytes = cursor.read_array::<12, C>(context)?;

        let stable_schema_key = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if stable_schema_key != expected_key {
            return Err(context.validation_error("Stable schema key mismatch", object_start));
        }

        let schema_revision = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let field_count = u16::from_le_bytes([bytes[8], bytes[9]]);
        let reserved = u16::from_le_bytes([bytes[10], bytes[11]]);

        if reserved != 0 {
            return Err(context.error(DecodeError::InvalidFieldTable {
                pos: object_start + 10,
            }));
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

/// Helper for iterating over schema-aware object field tables and their payloads.
pub struct FieldTableReader<'a> {
    table_cursor: Cursor<'a>,
    pub payload_cursor: Cursor<'a>,
    remaining: usize,
}

impl<'a> FieldTableReader<'a> {
    pub fn new<C>(
        cursor: &mut Cursor<'a>,
        field_count: usize,
        context: &mut C,
    ) -> Result<Self, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let table_cursor = *cursor;
        let table_len = field_count
            .checked_mul(FieldEntry::SIZE)
            .ok_or_else(|| DecodeError::InvalidFieldTable { pos: cursor.pos() })?;
        cursor.advance(table_len, context)?;
        let payload_cursor = *cursor;
        Ok(Self {
            table_cursor,
            payload_cursor,
            remaining: field_count,
        })
    }

    pub fn next<C>(
        &mut self,
        context: &mut C,
    ) -> Result<Option<(FieldEntry, usize, &'a [u8])>, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        if self.remaining == 0 {
            return Ok(None);
        }
        let entry_pos = self.table_cursor.pos();
        let entry = FieldEntry::decode(&mut self.table_cursor, context)?;
        let payload = self
            .payload_cursor
            .read_exact(entry.payload_len as usize, context)?;
        self.remaining -= 1;
        Ok(Some((entry, entry_pos, payload)))
    }
}

/// Helper for processing schema-aware object field tables.
#[doc(hidden)]
pub fn process_field_table<'a, C, F>(
    cursor: &mut Cursor<'a>,
    field_count: usize,
    context: &mut C,
    mut handler: F,
) -> Result<(), DecodeError>
where
    C: ValidationContext + ?Sized,
    F: FnMut(FieldEntry, usize, &'a [u8], &mut C) -> Result<(), DecodeError>,
{
    let mut reader = FieldTableReader::new(cursor, field_count, context)?;
    while let Some((entry, entry_pos, payload)) = reader.next(context)? {
        handler(entry, entry_pos, payload, context)?;
    }
    *cursor = reader.payload_cursor;
    Ok(())
}
