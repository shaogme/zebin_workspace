use core::num::NonZeroUsize;

use crate::{
    ZebinError,
    core::schema::{
        LayoutDescriptor, LayoutField, ObjectEncoding, SchemaRevision, StableSchemaKey,
    },
    traits::{ByteSink, LayoutSink},
    utils::byteops,
};

/// Shared layout registry used by both the measuring and emitting encoders.
pub(crate) struct LayoutRegistry<'a> {
    layouts: [Option<LayoutDescriptor<'a>>; 32],
    count: usize,
}

impl<'a> Default for LayoutRegistry<'a> {
    fn default() -> Self {
        const NONE: Option<LayoutDescriptor<'static>> = None;
        Self {
            layouts: [NONE; 32],
            count: 0,
        }
    }
}

impl<'a> LayoutRegistry<'a> {
    pub fn register(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
        encoding: ObjectEncoding,
        layout: &'a [LayoutField],
    ) -> Result<(), ZebinError> {
        for i in 0..self.count {
            let existing = self.layouts[i].as_ref().unwrap();
            if existing.stable_schema_key == stable_schema_key
                && existing.schema_revision == schema_revision
            {
                if existing.encoding != encoding || existing.fields != layout {
                    return Err(ZebinError::LayoutCollision {
                        key: stable_schema_key,
                        revision: schema_revision,
                    });
                }
                return Ok(());
            }
        }

        if self.count >= self.layouts.len() {
            return Err(ZebinError::LayoutRegistryFull);
        }

        let descriptor =
            LayoutDescriptor::new(stable_schema_key, schema_revision, encoding, layout)?;
        self.layouts[self.count] = Some(descriptor);
        self.count += 1;
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn get_layout(&self, index: usize) -> Option<&LayoutDescriptor<'a>> {
        self.layouts.get(index).and_then(|o| o.as_ref())
    }
}

/// Measuring encoder that simulates writes while collecting layouts.
pub(crate) struct MeasureEncoder<'a> {
    pos: usize,
    layouts: LayoutRegistry<'a>,
}

impl<'a> MeasureEncoder<'a> {
    pub fn new(pos: usize) -> Self {
        Self {
            pos,
            layouts: LayoutRegistry::default(),
        }
    }

    pub fn layouts_moved(self) -> LayoutRegistry<'a> {
        self.layouts
    }
}

impl ByteSink for MeasureEncoder<'_> {
    fn pos(&self) -> usize {
        self.pos
    }

    fn write(&mut self, bytes: &[u8]) -> Result<usize, ZebinError> {
        self.pos = self
            .pos
            .checked_add(bytes.len())
            .ok_or(ZebinError::ArithmeticOverflow { pos: self.pos })?;
        Ok(bytes.len())
    }

    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, ZebinError> {
        let alignment = alignment.get();
        let pos = self.pos;
        let padding = (alignment - (pos % alignment)) % alignment;
        self.pos = self
            .pos
            .checked_add(padding)
            .ok_or(ZebinError::ArithmeticOverflow { pos: self.pos })?;
        Ok(padding)
    }

    fn skip(&mut self, len: usize) -> Result<usize, ZebinError> {
        self.pos = self
            .pos
            .checked_add(len)
            .ok_or(ZebinError::ArithmeticOverflow { pos: self.pos })?;
        Ok(len)
    }
}

impl<'a> LayoutSink<'a> for MeasureEncoder<'a> {
    fn register_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
        encoding: ObjectEncoding,
        layout: &'a [LayoutField],
    ) -> Result<(), ZebinError> {
        self.layouts
            .register(stable_schema_key, schema_revision, encoding, layout)
    }
}

/// Chunked encoder that writes into a caller-provided buffer slice.
pub(crate) struct SliceEncoder<'a, 'b> {
    buf: &'a mut [u8],
    written: usize,
    archive_pos: usize,
    layouts: &'a mut LayoutRegistry<'b>,
}

impl<'a, 'b> SliceEncoder<'a, 'b> {
    pub fn new(buf: &'a mut [u8], archive_pos: usize, layouts: &'a mut LayoutRegistry<'b>) -> Self {
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

    pub fn layouts(&self) -> &LayoutRegistry<'b> {
        self.layouts
    }
}

impl<'a, 'b> ByteSink for SliceEncoder<'a, 'b> {
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
        self.archive_pos =
            self.archive_pos
                .checked_add(written)
                .ok_or(ZebinError::ArithmeticOverflow {
                    pos: self.archive_pos,
                })?;
        Ok(written)
    }

    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, ZebinError> {
        let alignment = alignment.get();
        let padding = (alignment - (self.archive_pos % alignment)) % alignment;
        self.skip(padding)
    }

    fn skip(&mut self, len: usize) -> Result<usize, ZebinError> {
        if len == 0 {
            return Ok(0);
        }

        let remaining = self.buf.len().saturating_sub(self.written);
        let written = remaining.min(len);
        if written > 0 {
            byteops::fill(&mut self.buf[self.written..self.written + written], 0);
            self.written += written;
        }
        self.archive_pos =
            self.archive_pos
                .checked_add(written)
                .ok_or(ZebinError::ArithmeticOverflow {
                    pos: self.archive_pos,
                })?;
        Ok(written)
    }
}

impl<'a, 'b> LayoutSink<'b> for SliceEncoder<'a, 'b> {
    fn register_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
        encoding: ObjectEncoding,
        layout: &'b [LayoutField],
    ) -> Result<(), ZebinError> {
        self.layouts
            .register(stable_schema_key, schema_revision, encoding, layout)
    }
}
