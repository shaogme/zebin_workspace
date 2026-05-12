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

    pub const SIZE: usize = 8;
}

pub const MAX_SCHEMA_FIELDS: usize = 128;
