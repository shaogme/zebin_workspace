use std::{borrow::Cow, collections::VecDeque};

use zebin::{ZebinArchive, ZebinSerialize};

#[derive(ZebinArchive, ZebinSerialize)]
struct NativeContainers {
    maybe_name: Option<String>,
    boxed_name: Box<String>,
    tags: [String; 2],
    numbers: [u32; 4],
    outcome: Result<String, String>,
    queue: VecDeque<String>,
}

#[derive(ZebinArchive, ZebinSerialize)]
struct BorrowedContainers {
    borrowed_text: Cow<'static, str>,
    owned_text: Cow<'static, str>,
    borrowed_numbers: Cow<'static, [u32]>,
    owned_numbers: Cow<'static, [u32]>,
}

#[test]
fn test_native_container_round_trip_some() {
    let mut queue = VecDeque::new();
    queue.push_back("front".to_string());
    queue.push_back("back".to_string());

    let value = NativeContainers {
        maybe_name: Some("Alice".to_string()),
        boxed_name: Box::new("boxed".to_string()),
        tags: ["alpha".to_string(), "beta".to_string()],
        numbers: [1, 2, 3, 4],
        outcome: Ok("success".to_string()),
        queue,
    };

    let buf = zebin::encode(&value).unwrap();
    let archived = zebin::decode::<NativeContainers>(&buf).unwrap();

    assert!(archived.maybe_name.is_some());
    assert_eq!(
        unsafe { archived.maybe_name.as_ref().unwrap().as_str() },
        "Alice"
    );
    assert_eq!(unsafe { archived.boxed_name.as_str() }, "boxed");
    assert_eq!(unsafe { archived.tags[0].as_str() }, "alpha");
    assert_eq!(unsafe { archived.tags[1].as_str() }, "beta");
    assert_eq!(archived.numbers, [1, 2, 3, 4]);
    assert!(archived.outcome.is_ok());
    assert_eq!(
        unsafe { archived.outcome.as_ok().unwrap().as_str() },
        "success"
    );
    let queue = unsafe { archived.queue.as_slice() };
    assert_eq!(queue.len(), 2);
    assert_eq!(unsafe { queue[0].as_str() }, "front");
    assert_eq!(unsafe { queue[1].as_str() }, "back");
}

#[test]
fn test_native_container_round_trip_none() {
    let mut queue = VecDeque::new();
    queue.push_back("left".to_string());
    queue.push_back("right".to_string());

    let value = NativeContainers {
        maybe_name: None,
        boxed_name: Box::new("root".to_string()),
        tags: ["one".to_string(), "two".to_string()],
        numbers: [9, 8, 7, 6],
        outcome: Err("failure".to_string()),
        queue,
    };

    let buf = zebin::encode(&value).unwrap();
    let archived = zebin::decode::<NativeContainers>(&buf).unwrap();

    assert!(archived.maybe_name.is_none());
    assert_eq!(unsafe { archived.boxed_name.as_str() }, "root");
    assert_eq!(unsafe { archived.tags[0].as_str() }, "one");
    assert_eq!(unsafe { archived.tags[1].as_str() }, "two");
    assert_eq!(archived.numbers, [9, 8, 7, 6]);
    assert!(archived.outcome.is_err());
    assert_eq!(
        unsafe { archived.outcome.as_err().unwrap().as_str() },
        "failure"
    );
    let queue = unsafe { archived.queue.as_slice() };
    assert_eq!(queue.len(), 2);
    assert_eq!(unsafe { queue[0].as_str() }, "left");
    assert_eq!(unsafe { queue[1].as_str() }, "right");
}

#[test]
fn test_borrowed_container_round_trip() {
    let value = BorrowedContainers {
        borrowed_text: Cow::Borrowed("borrowed"),
        owned_text: Cow::Owned("owned".to_string()),
        borrowed_numbers: Cow::Borrowed(&[1u32, 2, 3][..]),
        owned_numbers: Cow::Owned(vec![4u32, 5, 6]),
    };

    let buf = zebin::encode(&value).unwrap();
    let archived = zebin::decode::<BorrowedContainers>(&buf).unwrap();

    assert_eq!(unsafe { archived.borrowed_text.as_str() }, "borrowed");
    assert_eq!(unsafe { archived.owned_text.as_str() }, "owned");
    assert_eq!(unsafe { archived.borrowed_numbers.as_slice() }, &[1, 2, 3]);
    assert_eq!(unsafe { archived.owned_numbers.as_slice() }, &[4, 5, 6]);
}
