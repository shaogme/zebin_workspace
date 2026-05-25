#![cfg(feature = "alloc")]

use zebin::{
    ZebinArchive, ZebinEncode,
    archive::{PackedBoolSlice, PackedBoolVec, PackedU8Slice, PackedU8Vec},
};

#[derive(ZebinArchive, ZebinEncode)]
struct PackedAttrStruct {
    #[zebin(packed)]
    flags: Vec<bool>,
    #[zebin(packed = 4)]
    nibbles: Vec<u8>,
    plain: u32,
}

#[derive(ZebinArchive, ZebinEncode)]
struct PackedTupleStruct(
    #[zebin(packed)] Vec<bool>,
    #[zebin(packed = 4)] Vec<u8>,
    u32,
);

#[derive(ZebinArchive, ZebinEncode)]
enum PackedVariantEnum {
    Empty,
    Packed(
        #[zebin(packed)] Vec<bool>,
        #[zebin(packed = 4)] Vec<u8>,
        u32,
    ),
}

#[test]
fn test_packed_bool_slice_round_trip() {
    let values = [
        true, false, true, true, false, false, true, false, true, true, false, true, false,
    ];
    let packed = PackedBoolSlice::new(&values);
    let regular = values.to_vec();

    let packed_buf = zebin::encode(&packed).unwrap();
    let regular_buf = zebin::encode(&regular).unwrap();

    assert!(packed_buf.len() < regular_buf.len());

    let archived = zebin::reader::<PackedBoolSlice<'static>>(&packed_buf).unwrap();
    assert_eq!(archived.len(), values.len());
    for (index, expected) in values.iter().enumerate() {
        assert_eq!(archived.get(index), Some(*expected));
    }
}

#[test]
fn test_packed_nibble_slice_round_trip() {
    let values = [0u8, 1, 2, 15, 3, 14, 7, 8, 9, 4, 5, 6];
    let packed = PackedU8Slice::<4>::new(&values);
    let regular = values.to_vec();

    let packed_buf = zebin::encode(&packed).unwrap();
    let regular_buf = zebin::encode(&regular).unwrap();

    assert!(packed_buf.len() < regular_buf.len());

    let archived = zebin::reader::<PackedU8Slice<'static, 4>>(&packed_buf).unwrap();
    assert_eq!(archived.len(), values.len());
    for (index, expected) in values.iter().enumerate() {
        assert_eq!(archived.get(index), Some(*expected));
    }
}

#[test]
fn test_packed_nibble_rejects_out_of_range_value() {
    let values = [0u8, 1, 16, 3];
    let packed = PackedU8Slice::<4>::new(&values);
    let err = zebin::encode(&packed).unwrap_err();
    assert!(matches!(
        err,
        zebin::ZebinError::SerializationError {
            message: "Value exceeds packed bit capacity",
            ..
        }
    ));
}

#[test]
fn test_packed_vec_round_trip() {
    let bools = vec![true, false, true, false, true, true, false, false];
    let packed_bools = PackedBoolVec::from(bools.clone());
    let packed_bools_buf = zebin::encode(&packed_bools).unwrap();
    let archived_bools = zebin::reader::<PackedBoolVec>(&packed_bools_buf).unwrap();
    assert_eq!(archived_bools.len(), bools.len());
    for (index, expected) in bools.iter().enumerate() {
        assert_eq!(archived_bools.get(index), Some(*expected));
    }

    let nibbles = vec![0u8, 3, 7, 15, 1, 2, 4, 8];
    let packed_nibbles = PackedU8Vec::<4>::from(nibbles.clone());
    let packed_nibbles_buf = zebin::encode(&packed_nibbles).unwrap();
    let archived_nibbles = zebin::reader::<PackedU8Vec<4>>(&packed_nibbles_buf).unwrap();
    assert_eq!(archived_nibbles.len(), nibbles.len());
    for (index, expected) in nibbles.iter().enumerate() {
        assert_eq!(archived_nibbles.get(index), Some(*expected));
    }
}

#[test]
fn test_packed_attr_round_trip() {
    let value = PackedAttrStruct {
        flags: vec![true, false, true, true, false, true, false, false, true],
        nibbles: vec![0, 1, 2, 15, 3, 14, 7, 8],
        plain: 42,
    };

    let buf = zebin::encode(&value).unwrap();
    let archived = zebin::reader::<PackedAttrStruct>(&buf).unwrap();

    assert_eq!(archived.plain, 42);
    assert_eq!(archived.flags.len(), value.flags.len());
    assert_eq!(archived.nibbles.len(), value.nibbles.len());
    for (index, expected) in value.flags.iter().enumerate() {
        assert_eq!(archived.flags.get(index), Some(*expected));
    }
    for (index, expected) in value.nibbles.iter().enumerate() {
        assert_eq!(archived.nibbles.get(index), Some(*expected));
    }
}

#[test]
fn test_packed_vec_iter_helpers() {
    let vec: PackedU8Vec<4> = [1u8, 2, 3, 4].into_iter().collect();
    let collected: Vec<u8> = vec.into_iter().collect();
    assert_eq!(collected, vec![1, 2, 3, 4]);
}

#[test]
fn test_packed_tuple_struct_round_trip() {
    let value = PackedTupleStruct(
        vec![true, false, true, true, false, true],
        vec![0, 1, 2, 15, 3, 14, 7, 8],
        99,
    );

    let buf = zebin::encode(&value).unwrap();
    let archived = zebin::reader::<PackedTupleStruct>(&buf).unwrap();

    assert_eq!(archived.2, 99);
    assert_eq!(archived.0.len(), value.0.len());
    assert_eq!(archived.1.len(), value.1.len());
    for (index, expected) in value.0.iter().enumerate() {
        assert_eq!(archived.0.get(index), Some(*expected));
    }
    for (index, expected) in value.1.iter().enumerate() {
        assert_eq!(archived.1.get(index), Some(*expected));
    }
}

#[test]
fn test_packed_enum_variant_round_trip() {
    let empty = PackedVariantEnum::Empty;
    let empty_buf = zebin::encode(&empty).unwrap();
    let empty_archived = zebin::reader::<PackedVariantEnum>(&empty_buf).unwrap();
    assert_eq!(empty_archived.tag(), 0);
    assert!(empty_archived.is_empty());

    let value = PackedVariantEnum::Packed(
        vec![true, false, true, true, false, false, true, false],
        vec![0, 1, 2, 15, 3, 14, 7, 8],
        7,
    );

    let buf = zebin::encode(&value).unwrap();
    let archived = zebin::reader::<PackedVariantEnum>(&buf).unwrap();

    assert_eq!(archived.tag(), 1);
    let packed = archived.as_packed().unwrap();
    assert_eq!(packed.2, 7);
    assert_eq!(packed.0.len(), 8);
    assert_eq!(packed.1.len(), 8);
    for (index, expected) in [true, false, true, true, false, false, true, false]
        .iter()
        .enumerate()
    {
        assert_eq!(packed.0.get(index), Some(*expected));
    }
    for (index, expected) in [0u8, 1, 2, 15, 3, 14, 7, 8].iter().enumerate() {
        assert_eq!(packed.1.get(index), Some(*expected));
    }
}
