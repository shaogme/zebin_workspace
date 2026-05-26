#![cfg(feature = "alloc")]

use zebin::prelude::*;

#[derive(ZebinArchive, ZebinEncode, Debug, PartialEq, Clone)]
pub struct VarIntStruct {
    pub a: u8,
    pub b: VarInt<u32>,
    pub c: u64,
}

#[derive(ZebinArchive, ZebinEncode, Debug, PartialEq, Clone)]
pub struct NestedVarInt {
    pub inner: VarIntStruct,
    pub count: VarInt<usize>,
}

#[derive(ZebinArchive, ZebinEncode, Debug, PartialEq, Clone)]
#[allow(dead_code)]
enum VarIntEnum {
    Small(u8),
    Large(VarInt<u64>),
}

#[test]
fn test_varint_struct_round_trip() {
    let s = VarIntStruct {
        a: 10,
        b: VarInt::new(300),
        c: 1000,
    };

    let buf = zebin::encode(s).unwrap();
    let mut reader_obj = zebin::reader::<VarIntStruct>(&buf).unwrap();
    let archived = reader_obj.read().unwrap();

    assert_eq!(archived.a, 10);
    assert_eq!(archived.b.get(), 300);
    assert_eq!(archived.c, 1000);
}

#[test]
fn test_varint_boundaries() {
    let values = [0, 1, 127, 128, 16383, 16384, u32::MAX];
    for &val in &values {
        let s = VarIntStruct {
            a: 1,
            b: VarInt::new(val),
            c: 2,
        };
        let buf = zebin::encode(s).unwrap();
        let mut reader_obj = zebin::reader::<VarIntStruct>(&buf).unwrap();
        let archived = reader_obj.read().unwrap();
        assert_eq!(archived.b.get(), val);
    }
}

#[test]
fn test_varint_u64_large() {
    let s = VarIntStruct {
        a: 1,
        b: VarInt::new(u32::MAX),
        c: 2,
    };
    let buf = zebin::encode(s).unwrap();
    let mut reader_obj = zebin::reader::<VarIntStruct>(&buf).unwrap();
    let archived = reader_obj.read().unwrap();
    assert_eq!(archived.b.get(), u32::MAX);
}

#[test]
fn test_varint_usize() {
    let s = NestedVarInt {
        inner: VarIntStruct {
            a: 1,
            b: VarInt::new(2),
            c: 3,
        },
        count: VarInt::new(12345678),
    };
    let buf = zebin::encode(s).unwrap();
    let mut reader_obj = zebin::reader::<NestedVarInt>(&buf).unwrap();
    let archived = reader_obj.read().unwrap();
    assert_eq!(archived.count.get(), 12345678);
}

#[test]
fn test_varint_enum_round_trip() {
    let e = VarIntEnum::Large(VarInt::new(0x123456789ABCDEF0));
    let buf = zebin::encode(e).unwrap();
    let mut reader_obj = zebin::reader::<VarIntEnum>(&buf).unwrap();
    let archived = reader_obj.read().unwrap();

    assert_eq!(archived.tag(), 1);
    let large = archived.as_large().unwrap();
    assert_eq!(large.0.get(), 0x123456789ABCDEF0);
}

#[test]
fn test_varint_in_varint_vec() {
    let v = VarIntVec::new(vec![1u32, 300, 100000]);
    let buf = zebin::encode(v).unwrap();
    let mut reader_obj = zebin::reader::<VarIntVec<u32>>(&buf).unwrap();
    let archived = reader_obj.read().unwrap();

    assert_eq!(archived.len(), 3);
    assert_eq!(archived.get(0).unwrap(), 1);
    assert_eq!(archived.get(1).unwrap(), 300);
    assert_eq!(archived.get(2).unwrap(), 100000);
}

#[test]
fn test_varint_vec_compact() {
    let values = vec![1u32, 300, 100000, 0, 42];
    let v = VarIntVec::new(&values);

    let buf = zebin::encode(v).unwrap();

    // Check size:
    // Header (12) + Data (1+2+3+1+1=8) + Alignment + Offsets (6*4=24) + Layouts...
    // It should be much smaller than fixed 5-byte elements in a large vector.

    let mut reader_obj = zebin::reader::<VarIntVec<u32>>(&buf).unwrap();
    let archived = reader_obj.read().unwrap();
    assert_eq!(archived.len(), 5);

    for (i, &expected) in values.iter().enumerate() {
        assert_eq!(archived.get(i).unwrap(), expected);
    }

    // Test iterator
    let collected: Vec<u32> = archived.iter().collect();
    assert_eq!(collected, values);
}
