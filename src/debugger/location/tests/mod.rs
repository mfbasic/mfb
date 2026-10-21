use super::*;

mod values;

fn memory(inner: Location, offset: i64) -> Location {
    Location {
        term: Term::Memory(Box::new(inner)),
        offset,
    }
}

#[test]
fn a_location_is_a_number_a_name_or_memory_each_with_an_offset() {
    assert_eq!(Location::parse("sp"), Ok(Location::named("sp", 0)));
    assert_eq!(Location::parse("sp+0x18"), Ok(Location::named("sp", 0x18)));
    assert_eq!(Location::parse("0x10+8"), Ok(Location::number(0x18)));
    assert_eq!(
        Location::parse("[sp]"),
        Ok(memory(Location::named("sp", 0), 0))
    );
    assert_eq!(
        Location::parse("[sp+0x18]"),
        Ok(memory(Location::named("sp", 0x18), 0))
    );
    assert_eq!(
        Location::parse("[sp]+0x18"),
        Ok(memory(Location::named("sp", 0), 0x18))
    );
    assert_eq!(
        Location::parse("[[sp+0x10]+8]-4"),
        Ok(memory(memory(Location::named("sp", 0x10), 8), -4))
    );
    assert_eq!(
        Location::parse("[0x1000]"),
        Ok(memory(Location::number(0x1000), 0))
    );
    for (word, message) in [
        ("[sp", "'[sp' opens a '[' it does not close"),
        ("[[sp]", "'[[sp]' opens a '[' it does not close"),
        ("[]", "'[]' is not an address, a register or a function"),
        (
            "[sp]x",
            "'[sp]x' is not an address, a register or a function",
        ),
        ("[sp]+zz", "'zz' is not a number"),
        ("[@]", "'@' is not an address, a register or a function"),
        ("sp]", "'sp]' is not an address, a register or a function"),
    ] {
        assert_eq!(Location::parse(word), Err(message.to_string()), "{word}");
    }
}

#[test]
fn a_location_is_shown_with_full_width_numbers_and_hex_offsets() {
    for (word, shown) in [
        ("sp+24", "sp+0x18"),
        ("x19-16", "x19-0x10"),
        ("[sp+24]", "[sp+0x18]"),
        ("[sp]+0x18", "[sp]+0x18"),
        ("[0x12348593]", "[0x0000000012348593]"),
        ("[[x0]-8]", "[[x0]-0x8]"),
    ] {
        assert_eq!(Location::parse(word).unwrap().shown(), shown, "{word}");
    }
}

#[test]
fn a_value_is_a_whole_number_in_decimal_or_hex_perhaps_negative() {
    assert_eq!(signed("5"), Ok(5));
    assert_eq!(signed("0x10"), Ok(0x10));
    assert_eq!(signed("-1"), Ok(u64::MAX));
    assert_eq!(signed("-0x10"), Ok(0u64.wrapping_sub(0x10)));
    assert_eq!(signed("0xffffffffffffffff"), Ok(u64::MAX));
    assert_eq!(signed("-0x8000000000000000"), Ok(1 << 63));
    assert_eq!(
        signed("-0x8000000000000001"),
        Err("'-0x8000000000000001' is below the smallest value, -0x8000000000000000".to_string())
    );
    assert_eq!(
        signed("five"),
        Err("'five' is not a value: give a number, such as 5, -1 or 0x10".to_string())
    );
}

#[test]
fn a_condition_takes_a_test_and_a_value_or_in_with_a_memory_read() {
    assert_eq!(
        Condition::parse("x0", "eq", Some("5")),
        Ok(Condition {
            location: Location::named("x0", 0),
            test: Test::Equal(5)
        })
    );
    assert_eq!(
        Condition::parse("x0", "gt", Some("-1")).unwrap().shown(),
        "x0 gt 0xffffffffffffffff (-1)"
    );
    assert_eq!(
        Condition::parse("[x0+8]", "in", None).unwrap().shown(),
        "[x0+0x8] in"
    );
    for (words, message) in [
        (
            ("x0", "is", Some("5")),
            "'is' is not a test: give eq, ne, gt, lt or in",
        ),
        (
            ("x0", "eq", None),
            "'eq' needs a value to test against: <where> eq <value>",
        ),
        (
            ("[x0]", "in", Some("5")),
            "'in' takes no value: <where> in at <address|function[+offset]>",
        ),
        (
            ("x0+8", "in", None),
            "'x0+0x8' can always be worked out: 'in' needs a memory read ([…])",
        ),
    ] {
        assert_eq!(
            Condition::parse(words.0, words.1, words.2),
            Err(message.to_string()),
            "{words:?}"
        );
    }
}
