use crate::io::Storage;
use crate::prelude::*;

pub type ZebinReader<'a, T, S = [u8], H = ArchiveHeader> = ArchiveReader<'a, T, S, H>;

/// Safe access layer output that keeps the validated byte slice alive.
pub struct ArchiveReader<'a, T, S = [u8], H = ArchiveHeader>
where
    T: Archive,
    T::Archived: Decode<'a>,
    S: Storage + ?Sized,
    H: ArchiveHeaderTrait,
{
    storage: &'a S,
    offset: usize,
    config: ValidationConfig,
    current_view: Option<<T::Archived as Decode<'a>>::View>,
    _phantom: core::marker::PhantomData<(T, H)>,
}

impl<'a, T, S, H> ArchiveReader<'a, T, S, H>
where
    T: Archive,
    T::Archived: Decode<'a>,
    S: Storage + ?Sized,
    H: ArchiveHeaderTrait,
{
    pub fn new(storage: &'a S, config: ValidationConfig) -> Result<Self, ZebinError> {
        Ok(Self {
            storage,
            offset: 0,
            config,
            current_view: None,
            _phantom: core::marker::PhantomData,
        })
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn is_finished(&self) -> bool {
        self.offset >= self.storage.as_slice().len()
    }

    pub fn read(&mut self) -> Result<&<T::Archived as Decode<'a>>::View, ZebinError> {
        let bytes: &'a [u8] = self.storage.as_slice();
        if self.offset >= bytes.len() {
            return Err(ZebinError::BufferTooSmall {
                pos: self.offset,
                required: 1,
            });
        }
        let remaining = &bytes[self.offset..];
        let header = H::parse(remaining)?;
        validate_root_object_encoding::<T, H>(&header)?;

        let mut validator = Validator::with_config(remaining, self.config, None);
        let mut cursor = Cursor::new(remaining, H::SIZE);
        let root = T::Archived::decode(&mut cursor, &mut validator)?;
        self.offset += cursor.pos();
        self.current_view = Some(root);
        Ok(self.current_view.as_ref().unwrap())
    }

    pub fn decode(storage: &'a S) -> Result<T, ZebinError>
    where
        <T::Archived as Decode<'a>>::View: Restore<T>,
    {
        let mut reader = Self::new(storage, ValidationConfig::default())?;
        let view = reader.read()?;
        view.restore()
    }

    pub fn validate(
        storage: &'a S,
        config: ValidationConfig,
        stack: Option<&mut ValidationPathStack>,
    ) -> Result<(), ZebinError> {
        let bytes = storage.as_slice();
        let header = H::parse(bytes)?;
        validate_root_object_encoding::<T, H>(&header)?;
        validate_root::<T>(bytes, H::SIZE, config, stack)
    }
}

fn validate_root<'a, T>(
    bytes: &'a [u8],
    root_pos: usize,
    config: ValidationConfig,
    mut stack: Option<&mut ValidationPathStack>,
) -> Result<(), ZebinError>
where
    T: Archive,
    T::Archived: Decode<'a>,
{
    let mut cursor = Cursor::new(bytes, root_pos);
    let (result, error_path) = {
        let mut validator = Validator::with_config(bytes, config, stack.as_deref_mut());
        let res = T::Archived::validate(&mut cursor, &mut validator).and_then(|()| {
            if cursor.pos() != bytes.len() {
                Err(validator.validation_error("Trailing bytes after root object", cursor.pos()))
            } else {
                Ok(())
            }
        });
        (res, validator.last_error_path().cloned())
    };

    if let (Some(s), Some(ep)) = (stack, error_path) {
        *s = ep;
    }

    result.map_err(Into::into)
}

fn validate_root_object_encoding<'a, T, H>(header: &H) -> Result<(), ZebinError>
where
    T: Archive,
    H: ArchiveHeaderTrait,
    T::Archived: Decode<'a>,
{
    let actual = ObjectEncoding::from_byte(header.flags()).ok_or(
        crate::error::ParseHeaderError::InvalidObjectEncoding {
            flags: header.flags(),
            pos: H::SIZE.saturating_sub(1),
        },
    )?;
    let expected = <T::Archived as ArchivedLayout>::OBJECT_ENCODING;
    if actual != expected {
        return Err(DecodeError::UnexpectedObjectEncoding {
            expected,
            actual,
            pos: H::SIZE.saturating_sub(1),
        }
        .into());
    }
    Ok(())
}
