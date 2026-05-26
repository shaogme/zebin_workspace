#[cfg(feature = "alloc")]
use zebin::{ZebinArchive, ZebinEncode};

#[cfg(feature = "alloc")]
#[derive(Debug, PartialEq, Eq, ZebinArchive, ZebinEncode, Clone)]
struct AlignedContainers {
    values: Vec<u64>,
    fixed: [u64; 3],
}

#[cfg(feature = "alloc")]
#[test]
fn test_aligned_containers_round_trip() {
    let value = AlignedContainers {
        values: vec![11, 22, 33, 44],
        fixed: [55, 66, 77],
    };

    let buf = zebin::encode(value.clone()).unwrap();
    let archived = zebin::reader::<AlignedContainers>(&buf).unwrap();

    assert_eq!(unsafe { archived.values.as_slice() }, &[11, 22, 33, 44]);
    assert_eq!(archived.fixed, [55, 66, 77]);
}

#[cfg(feature = "alloc")]
#[test]
fn test_aligned_containers_chunked_writer_matches_full_encode() {
    let value = AlignedContainers {
        values: vec![101, 202, 303],
        fixed: [404, 505, 606],
    };

    let expected = zebin::encode(&value).unwrap();
    let mut writer = zebin::ZebinWriter::<&AlignedContainers>::new(&value).unwrap();
    let mut actual = Vec::new();
    let mut chunk = [0u8; 7];

    while !writer.is_finished() {
        let written = writer.write(&mut chunk).unwrap();
        if written > 0 {
            actual.extend_from_slice(&chunk[..written]);
        }
    }

    assert_eq!(actual, expected);
    zebin::reader::<AlignedContainers>(&actual).unwrap();
}

#[cfg(feature = "alloc")]
#[test]
fn test_root_vec_u64_round_trip() {
    let value = vec![7u64, 14, 21, 28];

    let buf = zebin::encode(value.clone()).unwrap();
    let archived = zebin::reader::<Vec<u64>>(&buf).unwrap();

    assert_eq!(unsafe { archived.as_slice() }, &[7, 14, 21, 28]);
}

#[test]
fn test_root_array_u64_round_trip() {
    let value = [8u64, 16, 32];

    let mut buf = [0u8; 128];
    let mut writer = zebin::encode_chunked(value.clone()).unwrap();
    let mut written = 0;
    while !writer.is_finished() {
        let n = writer.write(&mut buf[written..]).unwrap();
        if n == 0 {
            break;
        }
        written += n;
    }

    let archived = zebin::reader::<[u64; 3]>(&buf[..written]).unwrap();

    assert_eq!(archived.root(), &[8, 16, 32]);
}
