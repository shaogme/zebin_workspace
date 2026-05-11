use crate::{
    error::AccessError,
    utils::num::{read_fixed, u32_to_usize},
};
use core::num::NonZeroUsize;

/// Stable identifier used to refer to a schema across archive revisions.
pub type StableSchemaKey = u32;

/// Monotonic schema revision for a stable schema key.
pub type SchemaRevision = u32;

/// Object-level encoding family stored in layout metadata.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ObjectEncoding {
    #[default]
    Fixed = 0,
    SchemaAware = 1,
    VarInt = 2,
    Packed = 3,
}

/// Field-level encoding family stored in layout metadata.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FieldEncoding {
    #[default]
    Fixed = 0,
    VarInt = 1,
    PackedBits = 2,
    PackedLen = 3,
    RelPtr = 4,
}

/// A single field entry inside a layout descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayoutField {
    pub field_id: u16,
    pub offset: u32,
    pub encoding: FieldEncoding,
}

/// An owned layout descriptor used while constructing an archive.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LayoutDescriptor<'a> {
    pub stable_schema_key: StableSchemaKey,
    pub schema_revision: SchemaRevision,
    pub encoding: ObjectEncoding,
    pub fields: &'a [LayoutField],
}

impl<'a> LayoutDescriptor<'a> {
    pub fn new(
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
        encoding: ObjectEncoding,
        fields: &'a [LayoutField],
    ) -> Result<Self, AccessError> {
        for pair in fields.windows(2) {
            if pair[0].field_id >= pair[1].field_id {
                return Err(AccessError::InvalidLayout { pos: 0 });
            }
        }
        Ok(Self {
            stable_schema_key,
            schema_revision,
            encoding,
            fields,
        })
    }

    pub fn field_offset(&self, field_id: u16) -> Option<u32> {
        self.fields
            .binary_search_by_key(&field_id, |field| field.field_id)
            .ok()
            .map(|idx| self.fields[idx].offset)
    }

    pub fn field_count(&self) -> usize {
        self.fields.len()
    }
}

/// A borrowed layout descriptor read from an archive.
#[derive(Clone, Copy)]
pub struct LayoutView<'a> {
    bytes: &'a [u8],
    entry_pos: usize,
    field_count: usize,
}

impl<'a> LayoutView<'a> {
    pub(crate) fn new(bytes: &'a [u8], entry_pos: usize, field_count: usize) -> Self {
        Self {
            bytes,
            entry_pos,
            field_count,
        }
    }

    pub fn stable_schema_key(&self) -> StableSchemaKey {
        let start = self.entry_pos;
        let mut key_bytes = [0u8; 4];
        key_bytes.copy_from_slice(&self.bytes[start..start + 4]);
        u32::from_le_bytes(key_bytes)
    }

    pub fn schema_revision(&self) -> SchemaRevision {
        let start = self.entry_pos + 4;
        let mut revision_bytes = [0u8; 4];
        revision_bytes.copy_from_slice(&self.bytes[start..start + 4]);
        u32::from_le_bytes(revision_bytes)
    }

    pub fn encoding(&self) -> ObjectEncoding {
        match self
            .bytes
            .get(self.entry_pos + 10)
            .copied()
            .unwrap_or_default()
        {
            1 => ObjectEncoding::SchemaAware,
            2 => ObjectEncoding::VarInt,
            3 => ObjectEncoding::Packed,
            _ => ObjectEncoding::Fixed,
        }
    }

    pub fn field_count(&self) -> usize {
        self.field_count
    }

    pub fn fields(&self) -> LayoutFieldIter<'a> {
        LayoutFieldIter {
            bytes: self.bytes,
            cursor: self.entry_pos + 16,
            remaining: self.field_count,
        }
    }

    pub fn field_offset(&self, field_id: u16) -> Option<u32> {
        let mut low = 0;
        let mut high = self.field_count;

        while low < high {
            let mid = low + (high - low) / 2;
            let field_pos = self.entry_pos + 16 + mid * 8;

            let mid_id = u16::from_le_bytes(
                self.bytes[field_pos..field_pos + 2]
                    .try_into()
                    .expect("Layout field id out of bounds"),
            );

            if mid_id < field_id {
                low = mid + 1;
            } else if mid_id > field_id {
                high = mid;
            } else {
                return Some(u32::from_le_bytes(
                    self.bytes[field_pos + 2..field_pos + 6]
                        .try_into()
                        .expect("Layout field offset out of bounds"),
                ));
            }
        }
        None
    }

    pub fn check_field(&self, field_id: u16, expected: u32) -> Result<(), AccessError> {
        let actual = self
            .field_offset(field_id)
            .ok_or(AccessError::MissingLayoutField {
                field_id,
                pos: self.entry_pos,
            })?;
        if actual != expected {
            return Err(AccessError::LayoutOffsetMismatch {
                field_id,
                expected,
                actual,
                pos: self.entry_pos,
            });
        }
        Ok(())
    }
}

/// Iterator over a borrowed layout's fields.
pub struct LayoutFieldIter<'a> {
    bytes: &'a [u8],
    cursor: usize,
    remaining: usize,
}

impl<'a> Iterator for LayoutFieldIter<'a> {
    type Item = LayoutField;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let field_id = u16::from_le_bytes(
            self.bytes
                .get(self.cursor..self.cursor + 2)?
                .try_into()
                .ok()?,
        );
        let offset = u32::from_le_bytes(
            self.bytes
                .get(self.cursor + 2..self.cursor + 6)?
                .try_into()
                .ok()?,
        );
        let encoding = match self.bytes.get(self.cursor + 6).copied().unwrap_or_default() {
            1 => FieldEncoding::VarInt,
            2 => FieldEncoding::PackedBits,
            3 => FieldEncoding::PackedLen,
            4 => FieldEncoding::RelPtr,
            _ => FieldEncoding::Fixed,
        };
        self.cursor += 8;
        self.remaining -= 1;
        Some(LayoutField {
            field_id,
            offset,
            encoding,
        })
    }
}

/// Borrowed access to the archive layout directory.
#[derive(Clone, Copy)]
pub struct LayoutDirectory<'a> {
    bytes: &'a [u8],
    section_offset: NonZeroUsize,
    parsed: ParsedLayoutSection<'a>,
}

impl<'a> LayoutDirectory<'a> {
    pub fn new(bytes: &'a [u8], section_offset: NonZeroUsize) -> Result<Self, AccessError> {
        let parsed = parse_layout_section(bytes, section_offset)?;
        Ok(Self {
            bytes,
            section_offset,
            parsed,
        })
    }

    pub fn section_offset(&self) -> NonZeroUsize {
        self.section_offset
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn lookup(
        &self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
    ) -> Result<LayoutView<'a>, AccessError> {
        self.parsed.lookup(stable_schema_key, schema_revision)
    }

    pub fn section_end(&self) -> usize {
        self.parsed.section_end
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ParsedLayoutSection<'a> {
    bytes: &'a [u8],
    section_offset: usize,
    num_layouts: usize,
    section_end: usize,
}

impl<'a> ParsedLayoutSection<'a> {
    fn lookup(
        &self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
    ) -> Result<LayoutView<'a>, AccessError> {
        let section_offset = self.section_offset;
        let offsets_pos = section_offset + 4;

        let mut found_entry: Option<(usize, usize)> = None;
        for layout_index in 0..self.num_layouts {
            let offset_pos = offsets_pos + layout_index * 4;
            let layout_rel_offset = u32_to_usize(
                u32::from_le_bytes(read_fixed::<4>(
                    self.bytes,
                    offset_pos,
                    "Layout offset entry",
                )?),
                || AccessError::ValidationError {
                    message: "Layout offset entry overflow",
                    pos: offset_pos,
                },
            )?;

            let entry_pos = section_offset.checked_add(layout_rel_offset).ok_or(
                AccessError::ValidationError {
                    message: "Layout entry overflow",
                    pos: offset_pos,
                },
            )?;

            let entry_header_end =
                entry_pos
                    .checked_add(16)
                    .ok_or(AccessError::ValidationError {
                        message: "Layout entry overflow",
                        pos: entry_pos,
                    })?;
            if entry_header_end > self.bytes.len() {
                return Err(AccessError::ValidationError {
                    message: "Layout entry out of bounds",
                    pos: entry_pos,
                });
            }

            let stored_key = u32::from_le_bytes(read_fixed::<4>(
                self.bytes,
                entry_pos,
                "Layout stable schema key",
            )?);
            let stored_revision = u32::from_le_bytes(read_fixed::<4>(
                self.bytes,
                entry_pos + 4,
                "Layout schema revision",
            )?);
            if stored_key == stable_schema_key && stored_revision == schema_revision {
                found_entry = Some((entry_pos, layout_index));
                break;
            }
        }

        let (entry_pos, _) = found_entry.ok_or(AccessError::MissingLayoutRevision {
            key: stable_schema_key,
            revision: schema_revision,
            pos: section_offset,
        })?;

        let field_count = usize::from(u16::from_le_bytes(read_fixed::<2>(
            self.bytes,
            entry_pos + 8,
            "Layout field count",
        )?));
        let entry_end = entry_pos
            .checked_add(16)
            .and_then(|pos| pos.checked_add(field_count.checked_mul(8)?))
            .ok_or(AccessError::ValidationError {
                message: "Layout field table overflow",
                pos: entry_pos,
            })?;
        if entry_end > self.bytes.len() {
            return Err(AccessError::ValidationError {
                message: "Layout entry payload out of bounds",
                pos: entry_pos,
            });
        }

        Ok(LayoutView::new(self.bytes, entry_pos, field_count))
    }
}

fn parse_layout_section<'a>(
    bytes: &'a [u8],
    section_offset: NonZeroUsize,
) -> Result<ParsedLayoutSection<'a>, AccessError> {
    let section_offset = section_offset.get();

    let header_end = section_offset
        .checked_add(4)
        .ok_or(AccessError::ValidationError {
            message: "Layout section header overflow",
            pos: section_offset,
        })?;
    if header_end > bytes.len() {
        return Err(AccessError::ValidationError {
            message: "Layout section header out of bounds",
            pos: section_offset,
        });
    }

    let num_layouts = u32_to_usize(
        u32::from_le_bytes(read_fixed::<4>(
            bytes,
            section_offset,
            "Layout section header",
        )?),
        || AccessError::ValidationError {
            message: "Layout section layout count exceeds usize range",
            pos: section_offset,
        },
    )?;

    let offsets_pos = header_end;
    let offsets_end = offsets_pos
        .checked_add(num_layouts.checked_mul(4).ok_or({
            AccessError::ValidationError {
                message: "Layout offset table overflow",
                pos: section_offset,
            }
        })?)
        .ok_or(AccessError::ValidationError {
            message: "Layout offset table overflow",
            pos: section_offset,
        })?;
    if offsets_end > bytes.len() {
        return Err(AccessError::ValidationError {
            message: "Layout offset table out of bounds",
            pos: offsets_pos,
        });
    }

    let mut section_end = offsets_end;
    for layout_index in 0..num_layouts {
        let offset_pos = offsets_pos + layout_index * 4;
        let layout_rel_offset = u32_to_usize(
            u32::from_le_bytes(read_fixed::<4>(bytes, offset_pos, "Layout offset entry")?),
            || AccessError::ValidationError {
                message: "Layout offset entry overflow",
                pos: offset_pos,
            },
        )?;

        let entry_pos =
            section_offset
                .checked_add(layout_rel_offset)
                .ok_or(AccessError::ValidationError {
                    message: "Layout entry overflow",
                    pos: offset_pos,
                })?;

        let entry_header_end = entry_pos
            .checked_add(16)
            .ok_or(AccessError::ValidationError {
                message: "Layout entry overflow",
                pos: entry_pos,
            })?;
        if entry_header_end > bytes.len() {
            return Err(AccessError::ValidationError {
                message: "Layout entry out of bounds",
                pos: entry_pos,
            });
        }

        let field_count = usize::from(u16::from_le_bytes(read_fixed::<2>(
            bytes,
            entry_pos + 8,
            "Layout field count",
        )?));
        let entry_end = entry_pos
            .checked_add(16)
            .and_then(|pos| pos.checked_add(field_count.checked_mul(8)?))
            .ok_or(AccessError::ValidationError {
                message: "Layout field table overflow",
                pos: entry_pos,
            })?;
        if entry_end > bytes.len() {
            return Err(AccessError::ValidationError {
                message: "Layout entry payload out of bounds",
                pos: entry_pos,
            });
        }
        if entry_end > section_end {
            section_end = entry_end;
        }
    }

    Ok(ParsedLayoutSection {
        bytes,
        section_offset,
        num_layouts,
        section_end,
    })
}
