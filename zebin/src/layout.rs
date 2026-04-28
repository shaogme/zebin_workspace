use std::collections::HashMap;
use std::convert::TryFrom;
use std::num::NonZeroUsize;

use crate::{
    ZebinError,
    core::schema::{LayoutDescriptor, LayoutField},
    num::usize_to_u32,
    traits::Encoder,
};

/// Shared layout registry used by both the measuring and emitting encoders.
#[derive(Default)]
pub(crate) struct LayoutRegistry {
    layouts: Vec<LayoutDescriptor>,
    layout_map: HashMap<LayoutDescriptor, u32>,
}

impl LayoutRegistry {
    pub fn register(&mut self, layout: &[LayoutField]) -> Result<u32, ZebinError> {
        let descriptor = LayoutDescriptor::new(layout.to_vec())?;
        if let Some(&id) = self.layout_map.get(&descriptor) {
            return Ok(id);
        }

        let id = usize_to_u32(self.layouts.len(), || ZebinError::LayoutError)?;
        self.layouts.push(descriptor.clone());
        self.layout_map.insert(descriptor, id);
        Ok(id)
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

impl Encoder for MeasureEncoder {
    type Error = ZebinError;

    fn pos(&self) -> usize {
        self.pos
    }

    fn write(&mut self, bytes: &[u8]) -> Result<usize, Self::Error> {
        self.pos = self
            .pos
            .checked_add(bytes.len())
            .ok_or(ZebinError::WriteError)?;
        Ok(bytes.len())
    }

    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, Self::Error> {
        let alignment = alignment.get();
        let pos = self.pos;
        let padding = (alignment - (pos % alignment)) % alignment;
        self.pos = self
            .pos
            .checked_add(padding)
            .ok_or(ZebinError::WriteError)?;
        Ok(padding)
    }

    fn register_layout(&mut self, layout: &[LayoutField]) -> Result<u32, Self::Error> {
        self.layouts.register(layout)
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

impl<'a> Encoder for SliceEncoder<'a> {
    type Error = ZebinError;

    fn pos(&self) -> usize {
        self.archive_pos
    }

    fn write(&mut self, bytes: &[u8]) -> Result<usize, Self::Error> {
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

    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, Self::Error> {
        let alignment = alignment.get();
        let padding = (alignment - (self.archive_pos % alignment)) % alignment;
        if padding == 0 {
            return Ok(0);
        }

        let remaining = self.buf.len().saturating_sub(self.written);
        let written = remaining.min(padding);
        if written > 0 {
            self.buf[self.written..self.written + written].fill(0);
            self.written += written;
            self.archive_pos = self
                .archive_pos
                .checked_add(written)
                .ok_or(ZebinError::WriteError)?;
        }
        Ok(written)
    }

    fn register_layout(&mut self, layout: &[LayoutField]) -> Result<u32, Self::Error> {
        self.layouts.register(layout)
    }
}

fn layout_section_len(layouts: &[LayoutDescriptor]) -> Result<usize, ZebinError> {
    let mut len = 4usize
        .checked_add(layouts.len().checked_mul(4).ok_or(ZebinError::WriteError)?)
        .ok_or(ZebinError::WriteError)?;
    for layout in layouts {
        len = len
            .checked_add(8)
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
            .checked_add(8)
            .and_then(|v| v.checked_add(layout.fields.len().checked_mul(4)?))
            .ok_or(ZebinError::WriteError)?;
    }

    for offset in offsets {
        bytes.extend_from_slice(&offset.to_le_bytes());
    }

    for (schema_id, layout) in layouts.iter().enumerate() {
        let schema_id = usize_to_u32(schema_id, || ZebinError::WriteError)?;
        bytes.extend_from_slice(&schema_id.to_le_bytes());
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
