#![cfg(feature = "mmap")]

use std::{
    fs::{self, File, OpenOptions},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use memmap2::{MmapMut, MmapOptions};
use zebin::{
    ArchiveHeader, ArchiveHeaderTrait, ArchivedLayout, ByteSink, Encode, EncodeState, Mmap,
    MmapEncoder, Storage, ZebinArchive, ZebinEncode, ZebinError,
};

#[derive(ZebinArchive, ZebinEncode)]
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

fn open_writable_mmap(path: &Path, len: usize) -> (File, MmapMut) {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .expect("open writable mmap file");
    file.set_len(len as u64).expect("set_len");
    let mmap = unsafe { MmapOptions::new().map_mut(&file).expect("map_mut") };
    (file, mmap)
}

fn drive_to_mmap<T>(value: &T, encoder: &mut MmapEncoder) -> Result<(), ZebinError>
where
    T: Encode,
    T::Archived: ArchivedLayout,
{
    let header = ArchiveHeader::create(<T::Archived as ArchivedLayout>::OBJECT_ENCODING as u8);
    encoder.write(header.encode().as_ref())?;

    let mut state = value.begin_encode()?;
    loop {
        match state.poll(encoder)? {
            core::task::Poll::Ready(()) => break,
            core::task::Poll::Pending => continue,
        }
    }
    Ok(())
}

#[test]
fn test_mmap_reads_archive_bytes() -> Result<(), Box<dyn std::error::Error>> {
    let user = MmapUser {
        id: 7,
        name: "Mika".to_string(),
    };
    let buf = zebin::encode(&user)?;

    let path = temp_archive_path();
    fs::write(&path, &buf)?;

    let mmap = Mmap::open(&path)?;
    assert_eq!(mmap.len(), buf.len());
    assert_eq!(mmap.as_slice(), buf.as_slice());

    let archived = zebin::reader::<MmapUser>(mmap.as_slice())?;
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
    let buf = zebin::encode(&MmapUser {
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
fn test_mmap_encoder_roundtrip_via_state_machine() -> Result<(), Box<dyn std::error::Error>> {
    let user = MmapUser {
        id: 42,
        name: "Aurora".to_string(),
    };
    let expected = zebin::encode(&user)?;

    let path = temp_archive_path();
    let (file, mmap_mut) = open_writable_mmap(&path, expected.len());

    let mut encoder = MmapEncoder::new(mmap_mut, 0);
    drive_to_mmap(&user, &mut encoder)?;
    assert_eq!(encoder.written(), expected.len());
    assert_eq!(encoder.pos(), expected.len());
    encoder.flush()?;
    drop(encoder);
    drop(file);

    let mmap = Mmap::open(&path)?;
    assert_eq!(mmap.as_slice(), expected.as_slice());
    let archived = zebin::reader::<MmapUser>(mmap.as_slice())?;
    assert_eq!(archived.id, 42);
    unsafe {
        assert_eq!(archived.name.as_str(), "Aurora");
    }

    drop(mmap);
    fs::remove_file(&path)?;
    Ok(())
}

#[test]
fn test_mmap_encoder_matches_vec_encode() -> Result<(), Box<dyn std::error::Error>> {
    let user = MmapUser {
        id: u64::MAX,
        name: "byte-exact-parity".to_string(),
    };
    let expected = zebin::encode(&user)?;

    let path = temp_archive_path();
    let (file, mmap_mut) = open_writable_mmap(&path, expected.len());

    let mut encoder = MmapEncoder::new(mmap_mut, 0);
    drive_to_mmap(&user, &mut encoder)?;
    encoder.flush()?;
    drop(encoder);
    drop(file);

    let on_disk = fs::read(&path)?;
    assert_eq!(on_disk, expected);

    fs::remove_file(&path)?;
    Ok(())
}

#[test]
fn test_mmap_encoder_overflow_returns_buffer_too_small()
-> Result<(), Box<dyn std::error::Error>> {
    let user = MmapUser {
        id: 9,
        name: "overflow".to_string(),
    };
    let expected = zebin::encode(&user)?;
    assert!(expected.len() > 1, "archive must be larger than 1 byte");

    let path = temp_archive_path();
    let (file, mmap_mut) = open_writable_mmap(&path, expected.len() - 1);

    let mut encoder = MmapEncoder::new(mmap_mut, 0);
    let result = drive_to_mmap(&user, &mut encoder);
    match result {
        Err(ZebinError::BufferTooSmall { .. }) => {}
        other => panic!("expected BufferTooSmall, got {other:?}"),
    }
    drop(encoder);
    drop(file);

    fs::remove_file(&path)?;
    Ok(())
}

#[test]
fn test_mmap_encoder_skip_and_align() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_archive_path();
    let (file, mmap_mut) = open_writable_mmap(&path, 64);

    let mut encoder = MmapEncoder::new(mmap_mut, 0);
    assert_eq!(encoder.pos(), 0);
    assert_eq!(encoder.capacity(), 64);

    encoder.skip(5)?;
    assert_eq!(encoder.pos(), 5);
    assert_eq!(encoder.written(), 5);

    encoder.align(NonZeroUsize::new(8).unwrap())?;
    assert_eq!(encoder.pos(), 8);
    assert_eq!(encoder.written(), 8);

    encoder.write(b"hello")?;
    assert_eq!(encoder.pos(), 13);
    assert_eq!(encoder.written(), 13);

    encoder.flush()?;
    let mmap = encoder.into_inner();
    assert_eq!(&mmap[0..8], &[0u8; 8], "skip/align region must be zeros");
    assert_eq!(&mmap[8..13], b"hello");
    drop(mmap);
    drop(file);

    fs::remove_file(&path)?;
    Ok(())
}

#[test]
fn test_mmap_encoder_write_past_end_errors() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_archive_path();
    let (file, mmap_mut) = open_writable_mmap(&path, 4);

    let mut encoder = MmapEncoder::new(mmap_mut, 0);
    encoder.write(b"abc")?;
    let err = encoder.write(b"too long").unwrap_err();
    assert!(matches!(err, ZebinError::BufferTooSmall { .. }));

    drop(encoder);
    drop(file);
    fs::remove_file(&path)?;
    Ok(())
}

