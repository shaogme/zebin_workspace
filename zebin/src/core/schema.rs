use crate::{ZebinError, num::u32_to_usize};
use std::num::NonZeroUsize;

/// A single field entry inside a layout descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayoutField {
    pub field_id: u16,
    pub offset: u16,
}

/// An owned layout descriptor used while constructing an archive.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LayoutDescriptor {
    pub fields: Vec<LayoutField>,
}

impl LayoutDescriptor {
    pub fn new(mut fields: Vec<LayoutField>) -> Result<Self, ZebinError> {
        fields.sort_unstable_by_key(|field| field.field_id);
        for pair in fields.windows(2) {
            if pair[0].field_id == pair[1].field_id {
                return Err(ZebinError::LayoutError);
            }
        }
        Ok(Self { fields })
    }

    pub fn field_offset(&self, field_id: u16) -> Option<u16> {
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

    pub fn schema_id(&self) -> u32 {
        let start = self.entry_pos;
        let mut schema_id_bytes = [0u8; 4];
        schema_id_bytes.copy_from_slice(&self.bytes[start..start + 4]);
        u32::from_le_bytes(schema_id_bytes)
    }

    pub fn field_count(&self) -> usize {
        self.field_count
    }

    pub fn fields(&self) -> LayoutFieldIter<'a> {
        LayoutFieldIter {
            bytes: self.bytes,
            cursor: self.entry_pos + 8,
            remaining: self.field_count,
        }
    }

    pub fn field_offset(&self, field_id: u16) -> Option<u16> {
        for field in self.fields() {
            if field.field_id == field_id {
                return Some(field.offset);
            }
        }
        None
    }

    pub fn check_field(&self, field_id: u16, expected: u16) -> Result<(), ZebinError> {
        let actual = self
            .field_offset(field_id)
            .ok_or_else(|| ZebinError::ValidationError {
                message: format!("Missing layout entry for field {}", field_id),
                pos: self.entry_pos,
            })?;
        if actual != expected {
            return Err(ZebinError::ValidationError {
                message: format!(
                    "Layout offset mismatch for field {}: expected {}, found {}",
                    field_id, expected, actual
                ),
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
        let offset = u16::from_le_bytes(
            self.bytes
                .get(self.cursor + 2..self.cursor + 4)?
                .try_into()
                .ok()?,
        );
        self.cursor += 4;
        self.remaining -= 1;
        Some(LayoutField { field_id, offset })
    }
}

/// Borrowed access to the archive layout directory.
#[derive(Clone, Copy)]
pub struct LayoutDirectory<'a> {
    bytes: &'a [u8],
    section_offset: NonZeroUsize,
}

impl<'a> LayoutDirectory<'a> {
    pub fn new(bytes: &'a [u8], section_offset: NonZeroUsize) -> Self {
        Self {
            bytes,
            section_offset,
        }
    }

    pub fn section_offset(&self) -> NonZeroUsize {
        self.section_offset
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn lookup(&self, schema_id: u32) -> Result<LayoutView<'a>, ZebinError> {
        let section_offset = self.section_offset.get();
        let header_end =
            section_offset
                .checked_add(4)
                .ok_or_else(|| ZebinError::ValidationError {
                    message: "Layout section header overflow".to_string(),
                    pos: section_offset,
                })?;
        if header_end > self.bytes.len() {
            return Err(ZebinError::ValidationError {
                message: "Layout section header out of bounds".to_string(),
                pos: section_offset,
            });
        }

        let num_layouts = u32_to_usize(
            u32::from_le_bytes(
                self.bytes
                    .get(section_offset..header_end)
                    .ok_or_else(|| ZebinError::ValidationError {
                        message: "Layout section header out of bounds".to_string(),
                        pos: section_offset,
                    })?
                    .try_into()
                    .map_err(|_| ZebinError::LayoutError)?,
            ),
            || ZebinError::ValidationError {
                message: "Layout section layout count exceeds usize range".to_string(),
                pos: section_offset,
            },
        )?;

        let schema_index = u32_to_usize(schema_id, || ZebinError::ValidationError {
            message: "Layout schema id overflow".to_string(),
            pos: section_offset,
        })?;
        if schema_index >= num_layouts {
            return Err(ZebinError::ValidationError {
                message: "Layout schema id out of range".to_string(),
                pos: schema_index,
            });
        }

        let offsets_pos = header_end;
        let offsets_end =
            offsets_pos
                .checked_add(num_layouts.checked_mul(4).ok_or_else(|| {
                    ZebinError::ValidationError {
                        message: "Layout offset table overflow".to_string(),
                        pos: section_offset,
                    }
                })?)
                .ok_or_else(|| ZebinError::ValidationError {
                    message: "Layout offset table overflow".to_string(),
                    pos: section_offset,
                })?;
        if offsets_end > self.bytes.len() {
            return Err(ZebinError::ValidationError {
                message: "Layout offset table out of bounds".to_string(),
                pos: offsets_pos,
            });
        }

        let offset_pos = offsets_pos + schema_index * 4;
        let layout_rel_offset = u32_to_usize(
            u32::from_le_bytes(
                self.bytes
                    .get(offset_pos..offset_pos + 4)
                    .ok_or_else(|| ZebinError::ValidationError {
                        message: "Layout offset entry out of bounds".to_string(),
                        pos: offset_pos,
                    })?
                    .try_into()
                    .map_err(|_| ZebinError::LayoutError)?,
            ),
            || ZebinError::ValidationError {
                message: "Layout offset entry overflow".to_string(),
                pos: offset_pos,
            },
        )?;

        let entry_pos = section_offset
            .checked_add(layout_rel_offset)
            .ok_or_else(|| ZebinError::ValidationError {
                message: "Layout entry overflow".to_string(),
                pos: offset_pos,
            })?;

        let entry_header_end =
            entry_pos
                .checked_add(8)
                .ok_or_else(|| ZebinError::ValidationError {
                    message: "Layout entry overflow".to_string(),
                    pos: entry_pos,
                })?;
        if entry_header_end > self.bytes.len() {
            return Err(ZebinError::ValidationError {
                message: "Layout entry out of bounds".to_string(),
                pos: entry_pos,
            });
        }

        let stored_schema_id = u32::from_le_bytes(
            self.bytes
                .get(entry_pos..entry_pos + 4)
                .ok_or_else(|| ZebinError::ValidationError {
                    message: "Layout schema id out of bounds".to_string(),
                    pos: entry_pos,
                })?
                .try_into()
                .map_err(|_| ZebinError::LayoutError)?,
        );
        if stored_schema_id != schema_id {
            return Err(ZebinError::ValidationError {
                message: format!(
                    "Layout schema id mismatch: expected {}, found {}",
                    schema_id, stored_schema_id
                ),
                pos: entry_pos,
            });
        }

        let field_count = usize::from(u16::from_le_bytes(
            self.bytes
                .get(entry_pos + 4..entry_pos + 6)
                .ok_or_else(|| ZebinError::ValidationError {
                    message: "Layout field count out of bounds".to_string(),
                    pos: entry_pos,
                })?
                .try_into()
                .map_err(|_| ZebinError::LayoutError)?,
        ));
        let entry_end = entry_pos
            .checked_add(8)
            .and_then(|pos| pos.checked_add(field_count.checked_mul(4)?))
            .ok_or_else(|| ZebinError::ValidationError {
                message: "Layout field table overflow".to_string(),
                pos: entry_pos,
            })?;
        if entry_end > self.bytes.len() {
            return Err(ZebinError::ValidationError {
                message: "Layout entry payload out of bounds".to_string(),
                pos: entry_pos,
            });
        }

        Ok(LayoutView::new(self.bytes, entry_pos, field_count))
    }
}
