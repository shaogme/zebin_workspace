use core::num::NonZeroUsize;
use core::ops::Deref;

use alloc::{format, string::ToString};

use crate::{
    core::{schema::LayoutDirectory, validator::Validator},
    format::{ARCHIVE_HEADER_SIZE, ArchiveHeader},
    num::{u32_to_nonzero_usize, u32_to_usize},
    traits::{Archive, Validate, ZebinError},
};

fn read_fixed<const N: usize>(
    bytes: &[u8],
    pos: usize,
    field: &'static str,
) -> Result<[u8; N], ZebinError> {
    let end = pos
        .checked_add(N)
        .ok_or_else(|| ZebinError::ValidationError {
            message: format!("{field} overflow"),
            pos,
        })?;
    let slice = bytes
        .get(pos..end)
        .ok_or_else(|| ZebinError::ValidationError {
            message: format!("{field} out of bounds"),
            pos,
        })?;
    let mut out = [0u8; N];
    out.copy_from_slice(slice);
    Ok(out)
}

/// Safe access layer output that keeps the validated byte slice alive.
pub struct ArchiveView<'a, T: Archive> {
    bytes: &'a [u8],
    header: ArchiveHeader,
    root: &'a T::Archived,
}

impl<'a, T: Archive> ArchiveView<'a, T> {
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn header(&self) -> ArchiveHeader {
        self.header
    }

    pub fn root(&self) -> &'a T::Archived {
        self.root
    }
}

impl<'a, T: Archive> Deref for ArchiveView<'a, T> {
    type Target = T::Archived;

    fn deref(&self) -> &Self::Target {
        self.root
    }
}

/// Decode and validate the archived root object.
pub fn decode<'a, T>(bytes: &'a [u8]) -> Result<ArchiveView<'a, T>, ZebinError>
where
    T: Archive,
    T::Archived: Validate<Validator<'a>>,
{
    check_archive(bytes)
}

fn validate_layout_section(bytes: &[u8], layout_offset: NonZeroUsize) -> Result<(), ZebinError> {
    let layout_offset = layout_offset.get();

    let header_end = layout_offset
        .checked_add(4)
        .ok_or_else(|| ZebinError::ValidationError {
            message: "Layout section header overflow".to_string(),
            pos: layout_offset,
        })?;
    if header_end > bytes.len() {
        return Err(ZebinError::ValidationError {
            message: "Layout section header out of bounds".to_string(),
            pos: layout_offset,
        });
    }

    let num_layouts = u32_to_usize(
        u32::from_le_bytes(read_fixed::<4>(
            bytes,
            layout_offset,
            "Layout section header",
        )?),
        || ZebinError::ValidationError {
            message: "Layout section layout count exceeds usize range".to_string(),
            pos: layout_offset,
        },
    )?;
    let offsets_pos = header_end;
    let offsets_end = offsets_pos
        .checked_add(
            num_layouts
                .checked_mul(4)
                .ok_or_else(|| ZebinError::ValidationError {
                    message: "Layout offset table overflow".to_string(),
                    pos: layout_offset,
                })?,
        )
        .ok_or_else(|| ZebinError::ValidationError {
            message: "Layout offset table overflow".to_string(),
            pos: layout_offset,
        })?;

    if offsets_end > bytes.len() {
        return Err(ZebinError::ValidationError {
            message: "Layout offset table out of bounds".to_string(),
            pos: offsets_pos,
        });
    }

    for layout_idx in 0..num_layouts {
        let offset_pos = offsets_pos + layout_idx * 4;
        let layout_rel_offset = u32_to_usize(
            u32::from_le_bytes(read_fixed::<4>(bytes, offset_pos, "Layout offset entry")?),
            || ZebinError::ValidationError {
                message: "Layout offset entry exceeds usize range".to_string(),
                pos: offset_pos,
            },
        )?;
        let layout_pos = layout_offset
            .checked_add(layout_rel_offset)
            .ok_or_else(|| ZebinError::ValidationError {
                message: "Layout position overflow".to_string(),
                pos: offset_pos,
            })?;
        let entry_header_end =
            layout_pos
                .checked_add(8)
                .ok_or_else(|| ZebinError::ValidationError {
                    message: "Layout entry overflow".to_string(),
                    pos: layout_pos,
                })?;
        if entry_header_end > bytes.len() {
            return Err(ZebinError::ValidationError {
                message: "Layout entry out of bounds".to_string(),
                pos: layout_pos,
            });
        }

        let schema_id = u32_to_usize(
            u32::from_le_bytes(read_fixed::<4>(bytes, layout_pos, "Layout schema id")?),
            || ZebinError::ValidationError {
                message: "Layout schema id exceeds usize range".to_string(),
                pos: layout_pos,
            },
        )?;
        if schema_id != layout_idx {
            return Err(ZebinError::ValidationError {
                message: format!(
                    "Layout schema id mismatch: expected {}, found {}",
                    layout_idx, schema_id
                ),
                pos: layout_pos,
            });
        }

        let field_count = usize::from(u16::from_le_bytes(read_fixed::<2>(
            bytes,
            layout_pos + 4,
            "Layout field count",
        )?));
        let entry_size =
            8usize
                .checked_add(field_count.checked_mul(4).ok_or_else(|| {
                    ZebinError::ValidationError {
                        message: "Layout field table overflow".to_string(),
                        pos: layout_pos,
                    }
                })?)
                .ok_or_else(|| ZebinError::ValidationError {
                    message: "Layout field table overflow".to_string(),
                    pos: layout_pos,
                })?;
        let entry_end =
            layout_pos
                .checked_add(entry_size)
                .ok_or_else(|| ZebinError::ValidationError {
                    message: "Layout entry overflow".to_string(),
                    pos: layout_pos,
                })?;
        if entry_end > bytes.len() {
            return Err(ZebinError::ValidationError {
                message: "Layout entry payload out of bounds".to_string(),
                pos: layout_pos,
            });
        }
    }

    Ok(())
}

/// Check the archive for safety and return a validated archive view.
fn check_archive<'a, T>(bytes: &'a [u8]) -> Result<ArchiveView<'a, T>, ZebinError>
where
    T: Archive,
    T::Archived: Validate<Validator<'a>>,
{
    let header = ArchiveHeader::parse(bytes)?;
    let root_pos = u32_to_usize(header.root_offset.get(), || ZebinError::ValidationError {
        message: "Root offset exceeds usize range".to_string(),
        pos: 8,
    })?;
    let root_end = root_pos
        .checked_add(core::mem::size_of::<T::Archived>())
        .ok_or_else(|| ZebinError::ValidationError {
            message: "Root range overflow".to_string(),
            pos: root_pos,
        })?;
    if root_pos < ARCHIVE_HEADER_SIZE {
        return Err(ZebinError::ValidationError {
            message: "Root overlaps archive header".to_string(),
            pos: root_pos,
        });
    }
    if root_end > bytes.len() {
        return Err(ZebinError::ValidationError {
            message: "Root out of bounds".to_string(),
            pos: root_pos,
        });
    }
    if root_pos % T::ALIGNMENT.get() != 0 {
        return Err(ZebinError::AlignmentError {
            expected: T::ALIGNMENT,
            actual: unsafe { NonZeroUsize::new_unchecked(root_pos % T::ALIGNMENT.get()) },
            pos: root_pos,
        });
    }

    let layout_offset = u32_to_usize(header.layout_offset.get(), || ZebinError::ValidationError {
        message: "Layout offset exceeds usize range".to_string(),
        pos: 4,
    })?;
    if layout_offset < root_end {
        return Err(ZebinError::ValidationError {
            message: "Layout section overlaps root".to_string(),
            pos: layout_offset,
        });
    }
    validate_layout_section(
        bytes,
        u32_to_nonzero_usize(
            header.layout_offset.get(),
            || ZebinError::ValidationError {
                message: "Layout offset exceeds usize range".to_string(),
                pos: 4,
            },
            || ZebinError::ValidationError {
                message: "Layout offset cannot be zero".to_string(),
                pos: 4,
            },
        )?,
    )?;

    let layout_dir = LayoutDirectory::new(
        bytes,
        u32_to_nonzero_usize(
            header.layout_offset.get(),
            || ZebinError::ValidationError {
                message: "Layout offset exceeds usize range".to_string(),
                pos: 4,
            },
            || ZebinError::ValidationError {
                message: "Layout offset cannot be zero".to_string(),
                pos: 4,
            },
        )?,
    );
    let mut validator = Validator::with_layouts(bytes, layout_dir);
    let root_ptr = unsafe { bytes.as_ptr().add(root_pos) as *const T::Archived };

    unsafe {
        T::Archived::validate(root_ptr, &mut validator)?;
    }

    Ok(ArchiveView {
        bytes,
        header,
        root: unsafe { &*root_ptr },
    })
}

/// Validate an archive without exposing the archived view.
pub fn validate<'a, T>(bytes: &'a [u8]) -> Result<(), ZebinError>
where
    T: Archive,
    T::Archived: 'a,
    T::Archived: Validate<Validator<'a>>,
{
    decode::<T>(bytes).map(|_| ())
}
