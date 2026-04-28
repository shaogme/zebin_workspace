use crate::ZebinError;
use crate::core::schema::{LayoutField, ObjectEncoding, SchemaRevision, StableSchemaKey};
use core::num::NonZeroUsize;

/// Byte-stream sink used by archive state machines.
pub trait ByteSink {
    fn pos(&self) -> usize;

    /// Write as many bytes as possible and return the amount consumed.
    fn write(&mut self, bytes: &[u8]) -> Result<usize, ZebinError>;

    /// Write as many alignment bytes as possible and return the amount consumed.
    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, ZebinError>;
}

/// Layout registration sink used by archive state machines.
pub trait LayoutSink {
    /// Register a layout descriptor for the current object.
    fn register_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
        encoding: ObjectEncoding,
        layout: &[LayoutField],
    ) -> Result<(), ZebinError>;
}
