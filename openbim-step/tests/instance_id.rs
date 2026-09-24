#![allow(missing_docs)]
//! `InstanceId` keeps up to 22 digits inline and longer ones on the heap.
//! The two storage forms must be indistinguishable through the public API.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use openbim_step::InstanceId;

fn hash_of(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn every_length_round_trips_across_the_inline_boundary() {
    for len in [1, 2, 19, 20, 21, 22, 23, 24, 40] {
        let digits: String = "123456789".chars().cycle().take(len).collect();
        let id = InstanceId::new(&digits).expect("valid digits");
        assert_eq!(id.as_str(), digits, "len {len}");
        assert_eq!(id.to_string(), format!("#{digits}"), "len {len}");
        assert_eq!(
            format!("{id:?}"),
            format!("InstanceId({digits:?})"),
            "len {len}"
        );
        assert_eq!(id.clone(), id, "len {len}");
        // Hash matches the digit string's, so inline and heap ids of the
        // same text would hash alike and map lookups cannot split on storage.
        assert_eq!(hash_of(&id), hash_of(&digits), "len {len}");
    }
}

#[test]
fn ordering_is_the_digit_strings_ordering_across_storage_forms() {
    let inline = InstanceId::new(&"9".repeat(22)).expect("valid");
    let heap = InstanceId::new(&"1".repeat(23)).expect("valid");
    // Lexical, not numeric: "999..." (22) sorts after "111..." (23).
    assert!(heap < inline);
    assert_eq!(
        inline.cmp(&heap),
        "9".repeat(22).as_str().cmp("1".repeat(23).as_str())
    );
    assert_ne!(inline, heap);
}

#[test]
fn u64_ids_are_the_same_value_as_parsed_digits() {
    assert_eq!(
        InstanceId::from(u64::MAX),
        InstanceId::new("18446744073709551615").expect("valid")
    );
    assert_eq!(InstanceId::from(0_u64).as_str(), "0");
}

#[test]
fn different_digits_of_the_same_length_are_different_ids() {
    // Inline and heap lengths both: equality must compare digits, not size.
    for (a, b) in [
        ("12", "13"),
        (&*"4".repeat(22), &*"5".repeat(22)),
        (&*"6".repeat(30), &*"7".repeat(30)),
    ] {
        let (a, b) = (
            InstanceId::new(a).expect("valid"),
            InstanceId::new(b).expect("valid"),
        );
        assert_ne!(a, b);
        assert_ne!(a.cmp(&b), std::cmp::Ordering::Equal);
    }
}

#[test]
fn non_digits_and_empty_are_still_rejected() {
    assert!(InstanceId::new("").is_none());
    assert!(InstanceId::new("12a").is_none());
    assert!(InstanceId::new(&format!("{}x", "1".repeat(30))).is_none());
}
