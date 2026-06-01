#![cfg(feature = "mmap")]

use std::{
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use zebin::prelude::{
    ArchiveHeader, ArchiveHeaderTrait, ArchivedLayout, CursorMut, Mmap, MmapMut, MmapSerializer,
    Serialize, Serializer, StorageMut, ZebinAccess, ZebinDeserialize, ZebinError, ZebinSerialize,
};
use zebin::{access, serialize};

#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Clone)]
pub struct MmapUser {
    pub id: u64,
    pub name: String,
}

fn temp_archive_path() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX_EPOCH")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "zebin-mmap-test-{}-{}.bin",
        std::process::id(),
        stamp
    ))
}

fn open_writable_mmap(path: &Path, len: usize) -> MmapMut {
    MmapMut::create(path, len as u64).expect("create writable mmap")
}

fn drive_to_mmap<T>(value: T, serializer: &mut MmapSerializer) -> Result<(), ZebinError>
where
    T: Serialize + 'static,
    T::Archived: ArchivedLayout,
    T: for<'a> Serialize<Input<'a> = T>,
{
    let header = ArchiveHeader::create(<T::Archived as ArchivedLayout>::OBJECT_ENCODING as u8);
    let mut writer = (&mut *serializer).into_cursor_mut();
    writer.write(header.serialize().as_ref())?;

    let mut body_serializer = T::serializer();
    if body_serializer.input(value, &mut writer)?.is_pending() {
        while body_serializer.poll_pending(&mut writer)?.is_pending() {}
    }
    let _ = body_serializer.finish(&mut writer)?;
    Ok(())
}

#[test]
fn test_mmap_reads_archive_bytes() -> Result<(), Box<dyn std::error::Error>> {
    let user = MmapUser {
        id: 7,
        name: "Mika".to_string(),
    };
    let buf = serialize(user)?;

    let path = temp_archive_path();
    fs::write(&path, &buf)?;

    let mmap = Mmap::open(&path)?;
    assert_eq!(mmap.len(), buf.len());
    assert_eq!(mmap.as_slice(), buf.as_slice());

    let archived = access::<MmapUser, _>(&mmap)?;
    assert_eq!(archived.id, 7);
    unsafe {
        assert_eq!(archived.name.as_str(), "Mika");
    }

    drop(mmap);
    fs::remove_file(&path)?;
    Ok(())
}

#[test]
fn test_mmap_is_read_only_for_extend() -> Result<(), Box<dyn std::error::Error>> {
    let buf = serialize(MmapUser {
        id: 1,
        name: "read-only".to_string(),
    })?;

    let path = temp_archive_path();
    fs::write(&path, &buf)?;

    let mut mmap = Mmap::open(&path)?;
    assert!(mmap.extend(b"extra").is_err());

    drop(mmap);
    fs::remove_file(&path)?;
    Ok(())
}

#[test]
fn test_mmap_serializer_roundtrip_via_state_machine() -> Result<(), Box<dyn std::error::Error>> {
    let user = MmapUser {
        id: 42,
        name: "Aurora".to_string(),
    };
    let expected = serialize(&user)?;

    let path = temp_archive_path();
    let mmap_mut = open_writable_mmap(&path, expected.len());

    let mut serializer = MmapSerializer::new(mmap_mut, 0);
    drive_to_mmap(user, &mut serializer)?;
    assert_eq!(serializer.written(), expected.len());
    assert_eq!(serializer.pos(), expected.len());
    serializer.flush()?;
    drop(serializer);

    let mmap = Mmap::open(&path)?;
    assert_eq!(mmap.as_slice(), expected.as_slice());
    let archived = access::<MmapUser, _>(&mmap)?;
    assert_eq!(archived.id, 42);
    unsafe {
        assert_eq!(archived.name.as_str(), "Aurora");
    }

    drop(mmap);
    fs::remove_file(&path)?;
    Ok(())
}

#[test]
fn test_mmap_serializer_matches_vec_serialize() -> Result<(), Box<dyn std::error::Error>> {
    let user = MmapUser {
        id: u64::MAX,
        name: "byte-exact-parity".to_string(),
    };
    let expected = serialize(&user)?;

    let path = temp_archive_path();
    let mmap_mut = open_writable_mmap(&path, expected.len());

    let mut serializer = MmapSerializer::new(mmap_mut, 0);
    drive_to_mmap(user, &mut serializer)?;
    serializer.flush()?;
    drop(serializer);

    let on_disk = fs::read(&path)?;
    assert_eq!(on_disk, expected);

    fs::remove_file(&path)?;
    Ok(())
}

#[test]
fn test_mmap_serializer_overflow_returns_buffer_too_small() -> Result<(), Box<dyn std::error::Error>>
{
    let user = MmapUser {
        id: 9,
        name: "overflow".to_string(),
    };
    let expected = serialize(&user)?;
    assert!(expected.len() > 1, "archive must be larger than 1 byte");

    let path = temp_archive_path();
    let mmap_mut = open_writable_mmap(&path, expected.len() - 1);

    let mut serializer = MmapSerializer::new(mmap_mut, 0);
    let result = drive_to_mmap(user, &mut serializer);
    match result {
        Err(ZebinError::BufferTooSmall { .. }) => {}
        other => panic!("expected BufferTooSmall, got {other:?}"),
    }
    drop(serializer);

    fs::remove_file(&path)?;
    Ok(())
}

#[test]
fn test_mmap_serializer_skip_and_align() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_archive_path();
    let mmap_mut = open_writable_mmap(&path, 64);

    let mut serializer = MmapSerializer::new(mmap_mut, 0);
    assert_eq!(serializer.pos(), 0);
    assert_eq!(serializer.capacity(), 64);

    {
        let mut writer = (&mut serializer).into_cursor_mut();
        writer.skip(5)?;
    }
    assert_eq!(serializer.pos(), 5);
    assert_eq!(serializer.written(), 5);

    {
        let mut writer = (&mut serializer).into_cursor_mut();
        writer.align(NonZeroUsize::new(8).unwrap())?;
    }
    assert_eq!(serializer.pos(), 8);
    assert_eq!(serializer.written(), 8);

    {
        let mut writer = (&mut serializer).into_cursor_mut();
        writer.write(b"hello")?;
    }
    assert_eq!(serializer.pos(), 13);
    assert_eq!(serializer.written(), 13);

    serializer.flush()?;
    let mmap = serializer.into_inner();
    assert_eq!(&mmap[0..8], &[0u8; 8], "skip/align region must be zeros");
    assert_eq!(&mmap[8..13], b"hello");
    drop(mmap);

    fs::remove_file(&path)?;
    Ok(())
}

#[test]
fn test_mmap_serializer_write_past_end_errors() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_archive_path();
    let mmap_mut = open_writable_mmap(&path, 4);

    let mut serializer = MmapSerializer::new(mmap_mut, 0);
    {
        let mut writer = (&mut serializer).into_cursor_mut();
        writer.write(b"abc")?;
        let err = writer.write(b"too long").unwrap_err();
        assert!(matches!(err, ZebinError::BufferTooSmall { .. }));
    }

    drop(serializer);
    fs::remove_file(&path)?;
    Ok(())
}

#[test]
fn test_mmap_serializer_with_writer() -> Result<(), Box<dyn std::error::Error>> {
    use zebin::writer;

    let user = MmapUser {
        id: 99,
        name: "MmapMutWriter".to_string(),
    };
    let expected = serialize(&user)?;

    let path = temp_archive_path();
    let mmap_mut = open_writable_mmap(&path, expected.len());

    let mut serializer = MmapSerializer::new(mmap_mut, 0);
    {
        let mut writer_obj = writer::<MmapUser, _>(&mut serializer)?;
        writer_obj.write_all(user)?;
    }
    serializer.flush()?;
    drop(serializer);

    let mmap = Mmap::open(&path)?;
    assert_eq!(mmap.as_slice(), expected.as_slice());
    let archived = access::<MmapUser, _>(&mmap)?;
    assert_eq!(archived.id, 99);
    unsafe {
        assert_eq!(archived.name.as_str(), "MmapMutWriter");
    }

    drop(mmap);
    fs::remove_file(&path)?;
    Ok(())
}
