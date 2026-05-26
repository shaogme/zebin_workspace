use crate::io::Storage;
use crate::prelude::*;
use core::ops::Deref;

/// Safe access layer output that keeps the validated byte slice alive.
pub struct ZebinReader<'a, T: Archive, H: ArchiveHeaderTrait = ArchiveHeader>
where
    T::Archived: Decode<'a>,
{
    bytes: &'a [u8],
    header: H,
    root: <T::Archived as Decode<'a>>::View,
}

impl<'a, T: Archive, H: ArchiveHeaderTrait> ZebinReader<'a, T, H>
where
    T::Archived: Decode<'a>,
{
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn header(&self) -> &H {
        &self.header
    }

    pub fn root(&self) -> &<T::Archived as Decode<'a>>::View {
        &self.root
    }

    pub fn restore(&self) -> Result<T, ZebinError>
    where
        <T::Archived as Decode<'a>>::View: Restore<T>,
    {
        self.root.restore()
    }

    pub fn new<S: Storage + ?Sized>(
        storage: &'a S,
        config: ValidationConfig,
    ) -> Result<Self, ZebinError> {
        let bytes = storage.as_slice();
        let header = H::parse(bytes)?;
        validate_root_object_encoding::<T, H>(&header)?;

        let mut validator = Validator::with_config(bytes, config, None);
        let mut cursor = Cursor::new(bytes, H::SIZE);
        let root = T::Archived::decode(&mut cursor, &mut validator)?;
        if cursor.pos() != bytes.len() {
            let pos = cursor.pos();
            return Err(validator
                .validation_error(
                    "archive validation failed: trailing bytes detected after root object",
                    pos,
                )
                .into());
        }

        Ok(Self {
            bytes,
            header,
            root,
        })
    }

    /// Create an iterator reader for reading consecutive archived objects from the storage.
    pub fn iter<S: Storage + ?Sized>(storage: &'a S) -> ZebinIter<'a, T, H> {
        ZebinIter::new(storage, ValidationConfig::default())
    }

    pub fn decode<S: Storage + ?Sized>(storage: &'a S) -> Result<T, ZebinError>
    where
        <T::Archived as Decode<'a>>::View: Restore<T>,
    {
        Self::new(storage, ValidationConfig::default())?.restore()
    }

    pub fn validate<S: Storage + ?Sized>(
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

impl<'a, T: Archive, H: ArchiveHeaderTrait> Deref for ZebinReader<'a, T, H>
where
    T::Archived: Decode<'a>,
{
    type Target = <T::Archived as Decode<'a>>::View;

    fn deref(&self) -> &Self::Target {
        &self.root
    }
}

/// Iterator for reading consecutive archived objects from the storage (like jsonl).
pub struct ZebinIter<'a, T: Archive, H: ArchiveHeaderTrait = ArchiveHeader>
where
    T::Archived: Decode<'a>,
{
    bytes: &'a [u8],
    offset: usize,
    config: ValidationConfig,
    _phantom: core::marker::PhantomData<(T, H)>,
}

impl<'a, T: Archive, H: ArchiveHeaderTrait> ZebinIter<'a, T, H>
where
    T::Archived: Decode<'a>,
{
    pub fn new(storage: &'a (impl Storage + ?Sized), config: ValidationConfig) -> Self {
        Self {
            bytes: storage.as_slice(),
            offset: 0,
            config,
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<'a, T: Archive, H: ArchiveHeaderTrait> Iterator for ZebinIter<'a, T, H>
where
    T::Archived: Decode<'a>,
{
    type Item = Result<ZebinReader<'a, T, H>, ZebinError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.bytes.len() {
            return None;
        }

        let remaining = &self.bytes[self.offset..];
        if remaining.is_empty() {
            return None;
        }

        let header = match H::parse(remaining) {
            Ok(h) => h,
            Err(e) => return Some(Err(e.into())),
        };

        if let Err(e) = validate_root_object_encoding::<T, H>(&header) {
            return Some(Err(e));
        }

        let mut validator = Validator::with_config(remaining, self.config, None);
        let mut cursor = Cursor::new(remaining, H::SIZE);

        let root = match T::Archived::decode(&mut cursor, &mut validator) {
            Ok(r) => r,
            Err(e) => return Some(Err(e.into())),
        };

        let consumed = cursor.pos();
        let chunk_bytes = &remaining[..consumed];
        self.offset += consumed;

        Some(Ok(ZebinReader {
            bytes: chunk_bytes,
            header,
            root,
        }))
    }
}
