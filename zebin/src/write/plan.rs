use crate::{
    error::{ValidateError, ZebinError},
    format::ArchiveHeader,
    traits::{Archive, ArchiveHeader as ArchiveHeaderTrait, Layout},
    utils::num::{u32_to_usize, usize_to_nonzero_u32},
    write::{
        encoder::{LayoutRegistry, MeasureEncoder},
        state::Serialize,
        state::SerializeState,
    },
};

pub(crate) struct EncodePlan<'a, H: ArchiveHeaderTrait = ArchiveHeader> {
    pub root_pos: usize,
    pub layout_pos: usize,
    pub total_len: usize,
    pub header: H,
    pub layouts: LayoutRegistry<'a>,
}

pub(crate) fn measure_plan<'a, T, H>(value: &'a T) -> Result<EncodePlan<'a, H>, ZebinError>
where
    T: Serialize + Archive,
    H: ArchiveHeaderTrait,
{
    use crate::io::sink::ByteSink;

    let mut encoder = MeasureEncoder::new(H::SIZE);
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
        || ValidateError::ValidationError {
            message: "Root offset exceeds u32 range",
            pos: root_offset,
            path: Default::default(),
        },
        || ValidateError::ValidationError {
            message: "Root offset cannot be zero",
            pos: root_offset,
            path: Default::default(),
        },
    )?;

    let root_pos = u32_to_usize(root_offset.get(), || ZebinError::ArithmeticOverflow {
        pos: root_offset.get() as usize,
    })?;
    let archived = value.resolve(root_pos, resolver)?;
    let size = archived.size_hint();
    encoder.skip(size)?;

    let layout_offset = encoder.pos();
    let layout_offset = usize_to_nonzero_u32(
        layout_offset,
        || ValidateError::ValidationError {
            message: "Layout section offset exceeds u32 range",
            pos: layout_offset,
            path: Default::default(),
        },
        || ValidateError::ValidationError {
            message: "Layout section offset cannot be zero",
            pos: layout_offset,
            path: Default::default(),
        },
    )?;

    let layouts = encoder.layouts_moved();
    let section_len = layout_section_len_registry(&layouts)?;
    let layout_pos = u32_to_usize(layout_offset.get(), || ZebinError::ArithmeticOverflow {
        pos: layout_offset.get() as usize,
    })?;
    let total_len = layout_pos
        .checked_add(section_len)
        .ok_or(ZebinError::ArithmeticOverflow { pos: layout_pos })?;

    Ok(EncodePlan {
        root_pos,
        layout_pos,
        total_len,
        header: H::create(
            <T::Archived as Layout>::ENCODING as u8,
            layout_offset,
            root_offset,
        ),
        layouts,
    })
}

pub(crate) fn fill_layout_section_chunk(
    registry: &LayoutRegistry<'_>,
    start_offset: usize,
    out: &mut [u8],
) -> Result<(), ZebinError> {
    let out_len = out.len();
    if out_len == 0 {
        return Ok(());
    }

    let total_len = layout_section_len_registry(registry)?;
    if start_offset >= total_len {
        return Ok(());
    }

    let end_offset = start_offset + out_len;
    let count = registry.count();
    let table_start = 4;
    let descriptors_start = table_start + count * 4;

    // 1. Layout count (4 bytes)
    if start_offset < 4 {
        let count_bytes = (count as u32).to_le_bytes();
        let overlap_start = start_offset;
        let overlap_end = end_offset.min(4);
        let len = overlap_end - overlap_start;
        out[0..len].copy_from_slice(&count_bytes[overlap_start..overlap_end]);
    }

    // 2. Offset table (count * 4 bytes)
    if end_offset > table_start && start_offset < descriptors_start {
        let mut current_descriptor_pos = descriptors_start;
        for i in 0..count {
            let entry_start = table_start + i * 4;
            let entry_end = entry_start + 4;

            if start_offset < entry_end && end_offset > entry_start {
                let overlap_start = start_offset.max(entry_start);
                let overlap_end = end_offset.min(entry_end);

                let out_pos = overlap_start - start_offset;
                let data_pos = overlap_start - entry_start;
                let len = overlap_end - overlap_start;

                let bytes = (current_descriptor_pos as u32).to_le_bytes();
                out[out_pos..out_pos + len].copy_from_slice(&bytes[data_pos..data_pos + len]);
            }

            let layout = registry.get_layout(i).unwrap();
            current_descriptor_pos += 16 + layout.fields.len() * 8;
        }
    }

    // 3. Descriptors (variable length)
    if end_offset > descriptors_start {
        let mut current_pos = descriptors_start;
        for i in 0..count {
            let layout = registry.get_layout(i).unwrap();
            let layout_len = 16 + layout.fields.len() * 8;
            let layout_end = current_pos + layout_len;

            if start_offset < layout_end && end_offset > current_pos {
                // Header (16 bytes)
                let header_end = current_pos + 16;
                if start_offset < header_end && end_offset > current_pos {
                    let mut header = [0u8; 16];
                    header[0..4].copy_from_slice(&layout.stable_schema_key.to_le_bytes());
                    header[4..8].copy_from_slice(&layout.schema_revision.to_le_bytes());
                    header[8..10].copy_from_slice(&(layout.fields.len() as u16).to_le_bytes());
                    header[10] = layout.encoding as u8;

                    let overlap_start = start_offset.max(current_pos);
                    let overlap_end = end_offset.min(header_end);
                    let out_pos = overlap_start - start_offset;
                    let data_pos = overlap_start - current_pos;
                    let len = overlap_end - overlap_start;
                    out[out_pos..out_pos + len].copy_from_slice(&header[data_pos..data_pos + len]);
                }

                // Fields (8 bytes per field)
                let fields_start = current_pos + 16;
                if end_offset > fields_start && start_offset < layout_end {
                    for (f_idx, field) in layout.fields.iter().enumerate() {
                        let field_start = fields_start + f_idx * 8;
                        let field_end = field_start + 8;

                        if start_offset < field_end && end_offset > field_start {
                            let mut field_bytes = [0u8; 8];
                            field_bytes[0..2].copy_from_slice(&field.field_id.to_le_bytes());
                            field_bytes[2..6].copy_from_slice(&field.offset.to_le_bytes());
                            field_bytes[6] = field.encoding as u8;

                            let overlap_start = start_offset.max(field_start);
                            let overlap_end = end_offset.min(field_end);
                            let out_pos = overlap_start - start_offset;
                            let data_pos = overlap_start - field_start;
                            let len = overlap_end - overlap_start;
                            out[out_pos..out_pos + len]
                                .copy_from_slice(&field_bytes[data_pos..data_pos + len]);
                        }
                    }
                }
            }

            current_pos = layout_end;
        }
    }
    Ok(())
}

pub(crate) fn layout_section_len_registry(
    registry: &LayoutRegistry<'_>,
) -> Result<usize, ZebinError> {
    let mut len = 4usize
        .checked_add(
            registry
                .count()
                .checked_mul(4)
                .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?,
        )
        .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
    for i in 0..registry.count() {
        let layout = registry.get_layout(i).unwrap();
        len = len
            .checked_add(16)
            .and_then(|v| v.checked_add(layout.fields.len().checked_mul(8)?))
            .ok_or(ZebinError::ArithmeticOverflow { pos: len })?;
    }
    Ok(len)
}
