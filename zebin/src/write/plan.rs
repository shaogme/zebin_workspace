use crate::{
    error::ZebinError,
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
        || ZebinError::ValidationError {
            message: "Root offset exceeds u32 range",
            pos: root_offset,
            path: Default::default(),
        },
        || ZebinError::ValidationError {
            message: "Root offset cannot be zero",
            pos: root_offset,
            path: Default::default(),
        },
    )?;

    let root_pos = u32_to_usize(root_offset.get(), || ZebinError::WriteError)?;
    let archived = value.resolve(root_pos, resolver)?;
    let size = archived.size_hint();
    encoder.skip(size)?;

    let layout_offset = encoder.pos();
    let layout_offset = usize_to_nonzero_u32(
        layout_offset,
        || ZebinError::ValidationError {
            message: "Layout section offset exceeds u32 range",
            pos: layout_offset,
            path: Default::default(),
        },
        || ZebinError::ValidationError {
            message: "Layout section offset cannot be zero",
            pos: layout_offset,
            path: Default::default(),
        },
    )?;

    let layouts = encoder.layouts_moved();
    let section_len = layout_section_len_registry(&layouts)?;
    let layout_pos = u32_to_usize(layout_offset.get(), || ZebinError::WriteError)?;
    let total_len = layout_pos
        .checked_add(section_len)
        .ok_or(ZebinError::WriteError)?;

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
    mut offset: usize,
    out: &mut [u8],
) -> Result<(), ZebinError> {
    let total_len = layout_section_len_registry(registry)?;
    let mut written = 0;

    while written < out.len() && offset < total_len {
        let byte = if offset < 4 {
            // Layout count
            let count = registry.count() as u32;
            count.to_le_bytes()[offset]
        } else if offset < 4 + registry.count() * 4 {
            // Offset table
            let table_offset = offset - 4;
            let layout_idx = table_offset / 4;
            let byte_idx = table_offset % 4;

            let mut current_pos = 4 + registry.count() * 4;
            for i in 0..layout_idx {
                let layout = registry.get_layout(i).unwrap();
                current_pos += 16 + layout.fields.len() * 8;
            }
            (current_pos as u32).to_le_bytes()[byte_idx]
        } else {
            // Descriptors
            let mut current_offset = 4 + registry.count() * 4;
            let mut found_byte = None;

            for i in 0..registry.count() {
                let layout = registry.get_layout(i).unwrap();
                let layout_len = 16 + layout.fields.len() * 8;

                if offset >= current_offset && offset < current_offset + layout_len {
                    let rel_offset = offset - current_offset;
                    if rel_offset < 4 {
                        found_byte = Some(layout.stable_schema_key.to_le_bytes()[rel_offset]);
                    } else if rel_offset < 8 {
                        found_byte = Some(layout.schema_revision.to_le_bytes()[rel_offset - 4]);
                    } else if rel_offset < 10 {
                        let field_count = layout.fields.len() as u16;
                        found_byte = Some(field_count.to_le_bytes()[rel_offset - 8]);
                    } else if rel_offset == 10 {
                        found_byte = Some(layout.encoding as u8);
                    } else if rel_offset < 16 {
                        found_byte = Some(0);
                    } else {
                        let field_rel = rel_offset - 16;
                        let field_idx = field_rel / 8;
                        let field_byte = field_rel % 8;
                        let field = &layout.fields[field_idx];
                        if field_byte < 2 {
                            found_byte = Some(field.field_id.to_le_bytes()[field_byte]);
                        } else if field_byte < 6 {
                            found_byte = Some(field.offset.to_le_bytes()[field_byte - 2]);
                        } else if field_byte == 6 {
                            found_byte = Some(field.encoding as u8);
                        } else {
                            found_byte = Some(0);
                        }
                    }
                    break;
                }
                current_offset += layout_len;
            }
            found_byte.unwrap_or(0)
        };

        out[written] = byte;
        written += 1;
        offset += 1;
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
                .ok_or(ZebinError::WriteError)?,
        )
        .ok_or(ZebinError::WriteError)?;
    for i in 0..registry.count() {
        let layout = registry.get_layout(i).unwrap();
        len = len
            .checked_add(16)
            .and_then(|v| v.checked_add(layout.fields.len().checked_mul(8)?))
            .ok_or(ZebinError::WriteError)?;
    }
    Ok(len)
}
