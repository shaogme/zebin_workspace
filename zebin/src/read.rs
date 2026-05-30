use crate::error::ParseHeaderError;
use crate::io::Storage;
use crate::prelude::*;

/// Safe access layer output that keeps the validated byte slice alive.
pub struct ZebinReader<'a, T, C, H = ArchiveHeader>
where
    C: Cursor<'a>,
    H: ArchiveHeaderTrait,
{
    cursor: C,
    config: ValidationConfig,
    _phantom: core::marker::PhantomData<(T, H, &'a ())>,
}

impl<'a, T, C, H> ZebinReader<'a, T, C, H>
where
    C: Cursor<'a>,
    H: ArchiveHeaderTrait,
{
    pub fn new(cursor: C, config: ValidationConfig) -> Result<Self, ZebinError> {
        Ok(Self {
            cursor,
            config,
            _phantom: core::marker::PhantomData,
        })
    }

    pub fn offset(&self) -> usize {
        self.cursor.pos()
    }

    pub fn is_finished(&self) -> bool {
        self.cursor.is_eof()
    }

    pub fn access<S>(
        storage: &'a S,
        config: ValidationConfig,
    ) -> Result<<T::Archived as Access>::View<'a>, ZebinError>
    where
        T: Archive,
        T::Archived: Access,
        S: Storage<Mode = StaticMode> + ?Sized,
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

    pub fn read(&mut self) -> Result<<T::Archived as Access>::View<'a>, ZebinError>
    where
        T: Archive,
        T::Archived: Access,
    {
        let mut validator = Validator::new(self.config, None);
        let header_bytes = self.cursor.peek_exact(H::SIZE, &mut validator)?;
        let header = H::parse(header_bytes)?;
        validate_root_object_encoding::<T, H>(&header)?;

        self.cursor.advance(H::SIZE, &mut validator)?;
        let view = T::Archived::access(&mut self.cursor, &mut validator)?;
        Ok(view)
    }

    pub fn deserialize(cursor: C) -> Result<T, ZebinError>
    where
        T: Archive + 'a,
        T::Archived: Access,
        for<'b> <T::Archived as Access>::View<'b>: Deserialize<T>,
    {
        let mut reader = Self::new(cursor, ValidationConfig::default())?;
        let view = reader.read()?;
        view.deserialize()
    }

    pub fn validate<S>(
        storage: &'a S,
        config: ValidationConfig,
        stack: Option<&mut ValidationPathStack>,
    ) -> Result<(), ZebinError>
    where
        T: Archive,
        T::Archived: Access,
        S: Storage + ?Sized,
    {
        let mut header_validator = Validator::new(config, None);
        let mut cursor = storage.cursor(0);
        let header_bytes = cursor.peek_exact(H::SIZE, &mut header_validator)?;
        let header = H::parse(header_bytes)?;
        validate_root_object_encoding::<T, H>(&header)?;
        cursor.advance(H::SIZE, &mut header_validator)?;
        validate_root::<T, S::Cursor<'a>>(cursor, config, stack)
    }
}

fn validate_root<'a, T, C>(
    mut cursor: C,
    config: ValidationConfig,
    mut stack: Option<&mut ValidationPathStack>,
) -> Result<(), ZebinError>
where
    T: Archive,
    C: Cursor<'a>,
    T::Archived: Access,
{
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
