use alloc::vec::Vec;
use core::num::NonZeroUsize;

use crate::{
    ZebinError,
    core::schema::{
        LayoutDescriptor, LayoutField, ObjectEncoding, SchemaRevision, StableSchemaKey,
    },
    io::sink::{ByteSink, LayoutSink},
    utils::{byteops, num::usize_to_u32},
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
        encoding: ObjectEncoding,
        layout: &[LayoutField],
    ) -> Result<(), ZebinError> {
        let descriptor = LayoutDescriptor::new(
            stable_schema_key,
            schema_revision,
            encoding,
            layout.to_vec(),
        )?;
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
        encoding: ObjectEncoding,
        layout: &[LayoutField],
    ) -> Result<(), ZebinError> {
        self.layouts
            .register(stable_schema_key, schema_revision, encoding, layout)
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
        encoding: ObjectEncoding,
        layout: &[LayoutField],
    ) -> Result<(), ZebinError> {
        self.layouts
            .register(stable_schema_key, schema_revision, encoding, layout)
    }
}
