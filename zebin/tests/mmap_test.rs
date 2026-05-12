#![cfg(feature = "mmap")]

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use zebin::{Mmap, Storage, ZebinArchive, ZebinEncode};

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
