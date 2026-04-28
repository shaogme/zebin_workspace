use core::convert::TryFrom;
use core::num::NonZeroUsize;

use alloc::vec::Vec;

use crate::{
    byteops,
    ZebinError,
    core::schema::{LayoutDescriptor, LayoutField, SchemaRevision, StableSchemaKey},
    num::usize_to_u32,
    traits::{ByteSink, LayoutSink},
};

#[cfg(feature = "no_std")]
pub type HashMap<K, V> = hashbrown::HashMap<K, V, core::hash::BuildHasherDefault<ahash::AHasher>>;

#[cfg(feature = "std")]
pub type HashMap<K, V> = std::collections::HashMap<K, V>;

/// Shared layout registry used by both the measuring and emitting encoders.
#[derive(Default)]
pub(crate) struct LayoutRegistry {
    layouts: Vec<LayoutDescriptor>,
    layout_map: HashMap<(StableSchemaKey, SchemaRevision), u32>,
}

impl LayoutRegistry {
    pub fn register(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
        layout: &[LayoutField],
    ) -> Result<(), ZebinError> {
        let descriptor =
            LayoutDescriptor::new(stable_schema_key, schema_revision, layout.to_vec())?;
        let key = (stable_schema_key, schema_revision);
        if let Some(&id) = self.layout_map.get(&key) {
            let existing = self
                .layouts
                .get(id as usize)
                .ok_or(ZebinError::LayoutError)?;
            if existing != &descriptor {
                return Err(ZebinError::LayoutError);
            }
            return Ok(());
        }

        let id = usize_to_u32(self.layouts.len(), || ZebinError::LayoutError)?;
        self.layouts.push(descriptor);
        self.layout_map.insert(key, id);
        Ok(())
    }

    pub fn layouts(&self) -> &[LayoutDescriptor] {
        &self.layouts
    }

    pub fn into_layouts(self) -> Vec<LayoutDescriptor> {
        self.layouts
    }
}

/// Measuring encoder that simulates writes while collecting layouts.
pub(crate) struct MeasureEncoder {
    pos: usize,
    layouts: LayoutRegistry,
}

impl MeasureEncoder {
    pub fn new(pos: usize) -> Self {
        Self {
            pos,
            layouts: LayoutRegistry::default(),
        }
    }

    pub fn into_layouts(self) -> Vec<LayoutDescriptor> {
        self.layouts.into_layouts()
    }
}

impl ByteSink for MeasureEncoder {
    fn pos(&self) -> usize {
        self.pos
    }

    fn write(&mut self, bytes: &[u8]) -> Result<usize, ZebinError> {
        self.pos = self
            .pos
            .checked_add(bytes.len())
            .ok_or(ZebinError::WriteError)?;
        Ok(bytes.len())
    }

    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, ZebinError> {
        let alignment = alignment.get();
        let pos = self.pos;
        let padding = (alignment - (pos % alignment)) % alignment;
        self.pos = self
            .pos
            .checked_add(padding)
            .ok_or(ZebinError::WriteError)?;
        Ok(padding)
    }
}

impl LayoutSink for MeasureEncoder {
    fn register_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
        layout: &[LayoutField],
    ) -> Result<(), ZebinError> {
        self.layouts
            .register(stable_schema_key, schema_revision, layout)
    }
}

/// Chunked encoder that writes into a caller-provided buffer slice.
pub(crate) struct SliceEncoder<'a> {
    buf: &'a mut [u8],
    written: usize,
    archive_pos: usize,
    layouts: &'a mut LayoutRegistry,
}

impl<'a> SliceEncoder<'a> {
    pub fn new(buf: &'a mut [u8], archive_pos: usize, layouts: &'a mut LayoutRegistry) -> Self {
        Self {
            buf,
            written: 0,
            archive_pos,
            layouts,
        }
    }

    pub fn written(&self) -> usize {
        self.written
    }

    pub fn layouts(&self) -> &[LayoutDescriptor] {
        self.layouts.layouts()
    }
}

impl<'a> ByteSink for SliceEncoder<'a> {
    fn pos(&self) -> usize {
        self.archive_pos
    }

    fn write(&mut self, bytes: &[u8]) -> Result<usize, ZebinError> {
        let remaining = self.buf.len().saturating_sub(self.written);
        if remaining == 0 || bytes.is_empty() {
            return Ok(0);
        }

        let written = remaining.min(bytes.len());
        self.buf[self.written..self.written + written].copy_from_slice(&bytes[..written]);
        self.written += written;
        self.archive_pos = self
            .archive_pos
            .checked_add(written)
            .ok_or(ZebinError::WriteError)?;
        Ok(written)
    }

    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, ZebinError> {
        let alignment = alignment.get();
        let padding = (alignment - (self.archive_pos % alignment)) % alignment;
        if padding == 0 {
            return Ok(0);
        }

        let remaining = self.buf.len().saturating_sub(self.written);
        let written = remaining.min(padding);
        if written > 0 {
            byteops::fill(&mut self.buf[self.written..self.written + written], 0);
            self.written += written;
            self.archive_pos = self
                .archive_pos
                .checked_add(written)
                .ok_or(ZebinError::WriteError)?;
        }
        Ok(written)
    }
}

impl<'a> LayoutSink for SliceEncoder<'a> {
    fn register_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
        layout: &[LayoutField],
    ) -> Result<(), ZebinError> {
        self.layouts
            .register(stable_schema_key, schema_revision, layout)
    }
}

fn layout_section_len(layouts: &[LayoutDescriptor]) -> Result<usize, ZebinError> {
    let mut len = 4usize
        .checked_add(layouts.len().checked_mul(4).ok_or(ZebinError::WriteError)?)
        .ok_or(ZebinError::WriteError)?;
    for layout in layouts {
        len = len
            .checked_add(12)
            .and_then(|v| v.checked_add(layout.fields.len().checked_mul(4)?))
            .ok_or(ZebinError::WriteError)?;
    }
    Ok(len)
}

pub(crate) fn build_layout_section_bytes(
    layouts: &[LayoutDescriptor],
) -> Result<Vec<u8>, ZebinError> {
    let total_len = layout_section_len(layouts)?;
    let mut bytes = Vec::with_capacity(total_len);
    let layout_count = usize_to_u32(layouts.len(), || ZebinError::WriteError)?;
    bytes.extend_from_slice(&layout_count.to_le_bytes());

    let mut offsets = Vec::with_capacity(layouts.len());
    let mut cursor = 4usize
        .checked_add(layouts.len().checked_mul(4).ok_or(ZebinError::WriteError)?)
        .ok_or(ZebinError::WriteError)?;
    for layout in layouts {
        offsets.push(usize_to_u32(cursor, || ZebinError::WriteError)?);
        cursor = cursor
            .checked_add(12)
            .and_then(|v| v.checked_add(layout.fields.len().checked_mul(4)?))
            .ok_or(ZebinError::WriteError)?;
    }

    for offset in offsets {
        bytes.extend_from_slice(&offset.to_le_bytes());
    }

    for layout in layouts {
        bytes.extend_from_slice(&layout.stable_schema_key.to_le_bytes());
        bytes.extend_from_slice(&layout.schema_revision.to_le_bytes());
        let field_count = u16::try_from(layout.fields.len()).map_err(|_| ZebinError::WriteError)?;
        bytes.extend_from_slice(&field_count.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        for field in &layout.fields {
            bytes.extend_from_slice(&field.field_id.to_le_bytes());
            bytes.extend_from_slice(&field.offset.to_le_bytes());
        }
    }

    debug_assert_eq!(bytes.len(), total_len);
    Ok(bytes)
}
