use alloc::{string::ToString, vec::Vec};
use core::convert::TryFrom;

use crate::{
    core::schema::LayoutDescriptor,
    error::ZebinError,
    format::{ARCHIVE_HEADER_SIZE, ArchiveHeader},
    traits::{Archive, Layout},
    utils::num::{u32_to_usize, usize_to_nonzero_u32, usize_to_u32},
    write::{encoder::MeasureEncoder, state::Serialize, state::SerializeState},
};

pub(crate) struct EncodePlan {
    pub root_pos: usize,
    pub layout_pos: usize,
    pub total_len: usize,
    pub header_bytes: [u8; ARCHIVE_HEADER_SIZE],
    pub layout_bytes: Vec<u8>,
    pub layouts: Vec<LayoutDescriptor>,
}

pub(crate) fn measure_plan<T>(value: &T) -> Result<EncodePlan, ZebinError>
where
    T: Serialize + Archive,
{
    use crate::io::sink::ByteSink;

    let mut encoder = MeasureEncoder::new(ARCHIVE_HEADER_SIZE);
    let mut state = value.begin_serialize()?;
    let resolver = loop {
        match state.poll(&mut encoder)? {
            ::core::task::Poll::Pending => continue,
            ::core::task::Poll::Ready(resolver) => break resolver,
        }
    };

    encoder.align(<T::Archived as Layout>::ALIGNMENT)?;
    let root_offset = encoder.pos();
    let root_offset = usize_to_nonzero_u32(
        root_offset,
        || ZebinError::ValidationError {
            message: "Root offset exceeds u32 range".to_string(),
            pos: root_offset,
        },
        || ZebinError::ValidationError {
            message: "Root offset cannot be zero".to_string(),
            pos: root_offset,
        },
    )?;

    let root_pos = u32_to_usize(root_offset.get(), || ZebinError::WriteError)?;
    let archived = value.resolve(root_pos, resolver)?;
    let archived_bytes = crate::traits::archived_bytes(&archived);
    encoder.write(&archived_bytes)?;

    let layout_offset = encoder.pos();
    let layout_offset = usize_to_nonzero_u32(
        layout_offset,
        || ZebinError::ValidationError {
            message: "Layout section offset exceeds u32 range".to_string(),
            pos: layout_offset,
        },
        || ZebinError::ValidationError {
            message: "Layout section offset cannot be zero".to_string(),
            pos: layout_offset,
        },
    )?;

    let layouts = encoder.into_layouts();
    let layout_bytes = build_layout_section_bytes(&layouts)?;
    let layout_pos = u32_to_usize(layout_offset.get(), || ZebinError::WriteError)?;
    let total_len = layout_pos
        .checked_add(layout_bytes.len())
        .ok_or(ZebinError::WriteError)?;

    Ok(EncodePlan {
        root_pos,
        layout_pos,
        total_len,
        header_bytes: ArchiveHeader::to_bytes(
            <T::Archived as Layout>::ENCODING as u8,
            layout_offset,
            root_offset,
        ),
        layout_bytes,
        layouts,
    })
}

fn layout_section_len(layouts: &[LayoutDescriptor]) -> Result<usize, ZebinError> {
    let mut len = 4usize
        .checked_add(layouts.len().checked_mul(4).ok_or(ZebinError::WriteError)?)
        .ok_or(ZebinError::WriteError)?;
    for layout in layouts {
        len = len
            .checked_add(16)
            .and_then(|v| v.checked_add(layout.fields.len().checked_mul(8)?))
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
            .checked_add(16)
            .and_then(|v| v.checked_add(layout.fields.len().checked_mul(8)?))
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
        bytes.push(layout.encoding as u8);
        bytes.push(0);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        for field in &layout.fields {
            bytes.extend_from_slice(&field.field_id.to_le_bytes());
            bytes.extend_from_slice(&field.offset.to_le_bytes());
            bytes.push(field.encoding as u8);
            bytes.push(0);
        }
    }

    debug_assert_eq!(bytes.len(), total_len);
    Ok(bytes)
}
