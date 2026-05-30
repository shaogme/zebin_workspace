use crate::error::ParseHeaderError;
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
        self.storage.cursor(self.offset).is_eof()
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
        let mut validator = Validator::new(config, None);
        let mut cursor = storage.cursor(0);
        let header_bytes = cursor.peek_exact(H::SIZE, &mut validator)?;
        let header = H::parse(header_bytes)?;
        validate_root_object_encoding::<T, H>(&header)?;

        cursor.advance(H::SIZE, &mut validator)?;
        let view = T::Archived::access(&mut cursor, &mut validator)?;
        Ok(view)
    }

    pub fn read(&mut self) -> Result<<T::Archived as Access>::View<'_>, ZebinError>
    where
        T: Archive,
        T::Archived: Access,
    {
        if self.is_finished() {
            self.storage.sharder().advance()?;
            self.offset = 0;
        }

        let mut validator = Validator::new(self.config, None);
        let mut cursor = self.storage.cursor(self.offset);
        let header_bytes = cursor.peek_exact(H::SIZE, &mut validator)?;
        let header = H::parse(header_bytes)?;
        validate_root_object_encoding::<T, H>(&header)?;

        cursor.advance(H::SIZE, &mut validator)?;
        let view = T::Archived::access(&mut cursor, &mut validator)?;
        self.offset = cursor.pos();
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
        let mut header_validator = Validator::new(config, None);
        let cursor = storage.cursor(0);
        let header_bytes = cursor.peek_exact(H::SIZE, &mut header_validator)?;
        let header = H::parse(header_bytes)?;
        validate_root_object_encoding::<T, H>(&header)?;
        validate_root::<T, S>(storage, H::SIZE, config, stack)
    }
}

fn validate_root<T, S>(
    storage: &S,
    root_pos: usize,
    config: ValidationConfig,
    mut stack: Option<&mut ValidationPathStack>,
) -> Result<(), ZebinError>
where
    T: Archive,
    S: Storage,
    T::Archived: Access,
{
    let mut cursor = storage.cursor(root_pos);
    let (result, error_path) = {
        let mut validator = Validator::new(config, stack.as_deref_mut());
        let res = T::Archived::validate(&mut cursor, &mut validator).and_then(|()| {
            if !cursor.is_eof() {
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
        ParseHeaderError::InvalidObjectEncoding {
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
