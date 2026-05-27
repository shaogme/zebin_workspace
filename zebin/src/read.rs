use crate::io::{Sharder, Storage};
use crate::prelude::*;

/// Safe access layer output that keeps the validated byte slice alive.
pub struct ZebinReader<'a, T, S, H = ArchiveHeader>
where
    S: Storage,
    H: ArchiveHeaderTrait,
{
    storage: S,
    offset: usize,
    config: ValidationConfig,
    _phantom: core::marker::PhantomData<(T, H, &'a S)>,
}

impl<'a, T, S, H> ZebinReader<'a, T, S, H>
where
    S: Storage,
    H: ArchiveHeaderTrait,
{
    pub fn new(storage: S, config: ValidationConfig) -> Result<Self, ZebinError> {
        Ok(Self {
            storage,
            offset: 0,
            config,
            _phantom: core::marker::PhantomData,
        })
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn is_finished(&self) -> bool {
        self.offset >= self.storage.as_slice().len()
    }

    pub fn access(
        storage: &'a S,
        config: ValidationConfig,
    ) -> Result<<T::Archived as Access>::View<'a>, ZebinError>
    where
        T: Archive,
        T::Archived: Access,
        S: Storage<Mode = StaticMode>,
    {
        let bytes = storage.as_slice();
        let header = H::parse(bytes)?;
        validate_root_object_encoding::<T, H>(&header)?;

        let mut validator = Validator::with_config(bytes, config, None);
        let mut cursor = Cursor::new(bytes, H::SIZE);
        let view = T::Archived::access(&mut cursor, &mut validator)?;
        Ok(view)
    }

    pub fn read(&mut self) -> Result<<T::Archived as Access>::View<'_>, ZebinError>
    where
        T: Archive,
        T::Archived: Access,
    {
        let len = self.storage.as_slice().len();
        if self.offset >= len {
            self.storage.sharder().advance()?;
            self.offset = 0;
        }
        let bytes = self.storage.as_slice();
        let remaining = &bytes[self.offset..];
        let header = H::parse(remaining)?;
        validate_root_object_encoding::<T, H>(&header)?;

        let mut validator = Validator::with_config(remaining, self.config, None);
        let mut cursor = Cursor::new(remaining, H::SIZE);
        let view = T::Archived::access(&mut cursor, &mut validator)?;
        self.offset += cursor.pos();
        Ok(view)
    }

    pub fn deserialize(storage: S) -> Result<T, ZebinError>
    where
        T: Archive,
        T::Archived: Access,
        for<'b> <T::Archived as Access>::View<'b>: Deserialize<T>,
    {
        let mut reader = Self::new(storage, ValidationConfig::default())?;
        let view = reader.read()?;
        view.deserialize()
    }

    pub fn validate(
        storage: &'a S,
        config: ValidationConfig,
        stack: Option<&mut ValidationPathStack>,
    ) -> Result<(), ZebinError>
    where
        T: Archive,
        T::Archived: Access,
    {
        let bytes = storage.as_slice();
        let header = H::parse(bytes)?;
        validate_root_object_encoding::<T, H>(&header)?;
        validate_root::<T>(bytes, H::SIZE, config, stack)
    }
}

fn validate_root<T>(
    bytes: &[u8],
    root_pos: usize,
    config: ValidationConfig,
    mut stack: Option<&mut ValidationPathStack>,
) -> Result<(), ZebinError>
where
    T: Archive,
    T::Archived: Access,
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

fn validate_root_object_encoding<T, H>(header: &H) -> Result<(), ZebinError>
where
    T: Archive,
    H: ArchiveHeaderTrait,
    T::Archived: Access,
{
    let actual = ObjectEncoding::from_byte(header.flags()).ok_or(
        crate::error::ParseHeaderError::InvalidObjectEncoding {
            flags: header.flags(),
            pos: H::SIZE.saturating_sub(1),
        },
    )?;
    let expected = <T::Archived as ArchivedLayout>::OBJECT_ENCODING;
    if actual != expected {
        return Err(AccessError::UnexpectedObjectEncoding {
            expected,
            actual,
            pos: H::SIZE.saturating_sub(1),
        }
        .into());
    }
    Ok(())
}
