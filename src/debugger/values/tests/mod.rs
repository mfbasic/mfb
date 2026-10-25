use super::*;

mod payloads;

/// Where the fake arena region's usable space starts.
const BASE: u64 = 0x1_0000;

/// Fake memory: `bytes` at `BASE`, all of it one arena region's usable
/// space, with a block header's worth of readable bytes before it.
struct Fake {
    bytes: Vec<u8>,
}

impl Fake {
    fn new(bytes: Vec<u8>) -> Fake {
        Fake { bytes }
    }

    fn region(&self) -> Region {
        Region::of_area(0, 0, BASE, self.bytes.len() as u64)
    }

    fn show(&self, address: u64, view: &View) -> Result<Vec<String>, String> {
        let bytes = self.bytes.clone();
        let mut read = |at: u64, length: usize| {
            at.checked_sub(BASE)
                .and_then(|from| bytes.get(from as usize..from as usize + length))
                .map(<[u8]>::to_vec)
                .ok_or_else(|| format!("unmapped {at:#x}"))
        };
        show(&mut read, &[self.region()], address, view)
    }
}

fn words(values: &[u64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn string_record(text: &[u8]) -> Vec<u8> {
    let mut bytes = (text.len() as u64).to_le_bytes().to_vec();
    bytes.extend(text);
    bytes.push(0);
    bytes
}

/// A collection block: header, `entries` (flags, key offset, key length,
/// value offset, value length), then `data` padded to `data_capacity`.
fn collection(
    kind: u8,
    key_type: u8,
    value_type: u8,
    dead: u32,
    count: u64,
    capacity: u64,
    entries: &[(u8, u64, u64, u64, u64)],
    data: &[u8],
    data_capacity: u64,
) -> Vec<u8> {
    let mut bytes = vec![kind, key_type, value_type, 1];
    bytes.extend((dead << 8).to_le_bytes());
    bytes.extend(words(&[count, capacity, data.len() as u64, data_capacity]));
    for (flags, key_offset, key_length, value_offset, value_length) in entries {
        bytes.push(*flags);
        bytes.extend([0; 7]);
        bytes.extend(words(&[
            *key_offset,
            *key_length,
            *value_offset,
            *value_length,
        ]));
    }
    if kind != 2 {
        bytes.resize(40 + capacity as usize * 40, 0);
    }
    bytes.extend(data);
    bytes.resize(bytes.len() + data_capacity as usize - data.len(), 0);
    bytes
}

fn all() -> Option<Span> {
    Some(Span {
        start: 0,
        end: None,
    })
}

#[test]
fn scalars_show_their_bits_and_their_value() {
    let fake = Fake::new(
        [
            words(&[(-20_i64) as u64, 3.5_f64.to_bits(), 0x1_8000_0000, 125_000]),
            vec![1, 0x41, 2, 0],
            0xe9_u32.to_le_bytes().to_vec(),
        ]
        .concat(),
    );
    let value = |offset: u64, scalar: Scalar| {
        fake.show(BASE + offset, &View::Scalar(scalar))
            .map(|lines| lines.join("\n"))
    };
    assert_eq!(
        value(0, Scalar::Integer),
        Ok("Value: 0xffffffffffffffec (-20)".to_string())
    );
    assert_eq!(
        value(8, Scalar::Float),
        Ok("Value: 0x400c000000000000 (3.5)".to_string())
    );
    assert_eq!(
        value(16, Scalar::Fixed),
        Ok("Value: 0x0000000180000000 (1.50)".to_string())
    );
    assert_eq!(
        value(24, Scalar::Money),
        Ok("Value: 0x000000000001e848 (1.25000)".to_string())
    );
    assert_eq!(
        value(32, Scalar::Boolean),
        Ok("Value: 0x01 (TRUE)".to_string())
    );
    assert_eq!(value(33, Scalar::Byte), Ok("Value: 0x41 (65)".to_string()));
    assert_eq!(
        value(34, Scalar::Boolean),
        Err(format!(
            "not a Boolean at 0x{:016x}: its byte is 0x02, not 0 or 1",
            BASE + 34
        ))
    );
    assert_eq!(
        value(36, Scalar::Scalar),
        Ok("Value: 0x000000e9 (U+00E9 'é')".to_string())
    );
}

#[test]
fn money_shows_all_five_places_and_its_sign() {
    assert_eq!(money_text(-300_000), "-3.00000");
    assert_eq!(money_text(1), "0.00001");
    assert_eq!(money_text(-1), "-0.00001");
}

#[test]
fn a_string_literal_reads_as_a_string_outside_the_arena() {
    let bytes = string_record(b"function handler ");
    let mut read = |at: u64, length: usize| {
        at.checked_sub(BASE)
            .and_then(|from| bytes.get(from as usize..from as usize + length))
            .map(<[u8]>::to_vec)
            .ok_or_else(|| format!("unmapped {at:#x}"))
    };
    assert_eq!(
        show_literal(&mut read, BASE, "_mfb_str_2"),
        Ok(vec![
            "Length: 17 Value: \"function handler \" (the literal _mfb_str_2)".to_string()
        ])
    );
}

#[test]
fn memory_outside_the_arena_or_past_a_block_is_refused() {
    let fake = Fake::new(words(&[1]));
    assert_eq!(
        fake.show(0x10, &View::Scalar(Scalar::Integer)),
        Err(
            "0x0000000000000010 is not in an arena region, so it holds no Integer; \
                 'show mem' reads any memory"
                .to_string()
        )
    );
    assert_eq!(
        fake.show(BASE + 4, &View::Scalar(Scalar::Integer)),
        Err(format!(
            "not an Integer at 0x{:016x}: its 8 bytes run past the end of their arena \
                 region",
            BASE + 4
        ))
    );
}

#[test]
fn a_string_is_its_length_and_its_text_escaped() {
    let fake = Fake::new(string_record("a\"\\\n\u{1b}é".as_bytes()));
    assert_eq!(
        fake.show(BASE, &View::String),
        Ok(vec!["Length: 7 Value: \"a\\\"\\\\\\n\\u{1b}é\"".to_string()])
    );
    let fake = Fake::new(string_record(b"ab\xffd"));
    assert_eq!(
        fake.show(BASE, &View::String),
        Err(format!(
            "not a String at 0x{BASE:016x}: byte 2 of its value, 0xff, is not UTF-8"
        ))
    );
    let mut long = string_record(b"abc");
    long[0] = 200;
    let fake = Fake::new(long);
    assert_eq!(
        fake.show(BASE, &View::String),
        Err(format!(
            "not a String at 0x{BASE:016x}: its length, 200, runs past the end of its \
                 arena region"
        ))
    );
}

#[test]
fn a_fixed_width_list_shows_its_header_and_its_elements() {
    let fake = Fake::new(collection(
        2,
        0,
        3,
        0,
        3,
        4,
        &[],
        &words(&[10, (-20_i64) as u64, 30]),
        32,
    ));
    let block = format!("0x{BASE:016x}-0x{:016x} (72 bytes)", BASE + 72);
    assert_eq!(
        fake.show(
            BASE,
            &View::List {
                element: None,
                span: None
            }
        ),
        Ok(vec![format!(
            "List Type: Integer Length: 3 Capacity: 4 Data: 24/32 bytes Block: {block}"
        )])
    );
    assert_eq!(
        fake.show(
            BASE,
            &View::List {
                element: None,
                span: Span::parse("1..")
            }
        ),
        Ok(vec![
            "Index | Address            | Bytes | Value".to_string(),
            format!(
                "1     | 0x{:016x} | 8     | 0xffffffffffffffec (-20)",
                BASE + 48
            ),
            format!(
                "2     | 0x{:016x} | 8     | 0x000000000000001e (30)",
                BASE + 56
            ),
        ])
    );
    assert_eq!(
        fake.show(
            BASE,
            &View::List {
                element: None,
                span: Span::parse("3")
            }
        ),
        Err("no element 3: the List has 3 (numbered from 0)".to_string())
    );
    assert_eq!(
        fake.show(
            BASE,
            &View::List {
                element: Some(Kind::String),
                span: Span::parse("0")
            }
        ),
        Err("the elements are Integer, not String".to_string())
    );
}

#[test]
fn a_list_of_strings_reads_each_from_its_entry() {
    let fake = Fake::new(collection(
        0,
        0,
        6,
        0,
        2,
        2,
        &[(1, 0, 0, 0, 2), (1, 0, 0, 2, 3)],
        b"hibye",
        5,
    ));
    let data = BASE + 40 + 80;
    assert_eq!(
        fake.show(
            BASE,
            &View::List {
                element: None,
                span: all()
            }
        ),
        Ok(vec![
            "Index | Address            | Bytes | Value".to_string(),
            format!("0     | 0x{data:016x} | 2     | \"hi\""),
            format!("1     | 0x{:016x} | 3     | \"bye\"", data + 2),
        ])
    );
}

#[test]
fn a_header_that_cannot_be_the_collection_asked_for_is_refused() {
    let map = Fake::new(collection(1, 6, 3, 0, 0, 0, &[], &[], 0));
    let list = View::List {
        element: None,
        span: None,
    };
    assert_eq!(
        map.show(BASE, &list),
        Err(format!(
            "not a List at 0x{BASE:016x}: its kind byte is 1, a Map"
        ))
    );
    let over = Fake::new(collection(2, 0, 3, 0, 5, 4, &[], &words(&[0; 4]), 32));
    assert_eq!(
        over.show(BASE, &list),
        Err(format!(
            "not a List at 0x{BASE:016x}: its count, 5, is over its capacity, 4"
        ))
    );
    let big = Fake::new(collection(2, 0, 3, 0, 1, 1, &[], &words(&[7]), 4096));
    let mut bytes = big.bytes.clone();
    bytes.truncate(64);
    assert_eq!(
        Fake::new(bytes).show(BASE, &list),
        Err(format!(
            "not a List at 0x{BASE:016x}: its 4136 bytes run past the end of its arena \
                 region"
        ))
    );
}

#[test]
fn a_set_skips_removed_elements_and_counts_them() {
    let fake = Fake::new(collection(
        3,
        6,
        2,
        1,
        3,
        4,
        &[(1, 0, 3, 3, 1), (2, 4, 4, 8, 1), (1, 9, 5, 14, 1)],
        b"red\x01gone\x01green\x01",
        16,
    ));
    let data = BASE + 40 + 160;
    assert_eq!(
        fake.show(
            BASE,
            &View::Set {
                element: None,
                span: None
            }
        )
        .map(|lines| lines[0]
            .split(" Data:")
            .next()
            .unwrap_or_default()
            .to_string()),
        Ok("Set Type: String Length: 2 Removed: 1 Capacity: 4".to_string())
    );
    assert_eq!(
        fake.show(
            BASE,
            &View::Set {
                element: None,
                span: all()
            }
        ),
        Ok(vec![
            "Index | Address            | Bytes | Element".to_string(),
            format!("0     | 0x{data:016x} | 3     | \"red\""),
            format!("1     | 0x{:016x} | 5     | \"green\"", data + 9),
        ])
    );
}

#[test]
fn a_map_shows_its_keys_its_values_an_entry_and_a_lookup() {
    let mut data = b"Ada\0\0\0\0\0".to_vec();
    data.extend(words(&[36]));
    data.extend(b"Grace\0\0\0");
    data.extend(words(&[85]));
    let fake = Fake::new(collection(
        1,
        6,
        3,
        0,
        2,
        2,
        &[(1, 0, 3, 8, 8), (1, 16, 5, 24, 8)],
        &data,
        32,
    ));
    let base = BASE + 40 + 80;
    let map = |view: MapView| fake.show(BASE, &View::Map(view));
    assert_eq!(
        map(MapView::Keys(None)),
        Ok(vec![
            "Index | Address            | Bytes | Key".to_string(),
            format!("0     | 0x{base:016x} | 3     | \"Ada\""),
            format!("1     | 0x{:016x} | 5     | \"Grace\"", base + 16),
        ])
    );
    assert_eq!(
        map(MapView::Values(None)),
        Ok(vec![
            "Index | Address            | Bytes | Value".to_string(),
            format!(
                "0     | 0x{:016x} | 8     | 0x0000000000000024 (36)",
                base + 8
            ),
            format!(
                "1     | 0x{:016x} | 8     | 0x0000000000000055 (85)",
                base + 24
            ),
        ])
    );
    let grace = Ok(vec![
        "Index: 1".to_string(),
        format!("Key: \"Grace\" at 0x{:016x} (5 bytes)", base + 16),
        format!(
            "Value: 0x0000000000000055 (85) at 0x{:016x} (8 bytes)",
            base + 24
        ),
    ]);
    assert_eq!(map(MapView::Entry(1)), grace);
    assert_eq!(map(MapView::Get("Grace".to_string())), grace);
    assert_eq!(
        map(MapView::Get("Nobody".to_string())),
        Err("the map has no key Nobody".to_string())
    );
    assert_eq!(
        map(MapView::Entry(2)),
        Err("no entry 2: the map has 2 (numbered from 0)".to_string())
    );
}

#[test]
fn keys_are_encoded_as_their_type_stores_them() {
    assert_eq!(
        key_bytes(COLLECTION_TYPE_INTEGER, "-2"),
        Ok(words(&[(-2_i64) as u64]))
    );
    assert_eq!(key_bytes(COLLECTION_TYPE_INTEGER, "0x10"), Ok(words(&[16])));
    assert_eq!(key_bytes(COLLECTION_TYPE_BOOLEAN, "TRUE"), Ok(vec![1]));
    assert_eq!(key_bytes(COLLECTION_TYPE_BYTE, "255"), Ok(vec![255]));
    assert_eq!(
        key_bytes(COLLECTION_TYPE_SCALAR, "U+E9"),
        Ok(0xe9_u32.to_le_bytes().to_vec())
    );
    assert_eq!(
        key_bytes(COLLECTION_TYPE_BYTE, "256"),
        Err("'256' is not a Byte".to_string())
    );
    assert_eq!(
        key_bytes(COLLECTION_TYPE_OBJECT, "x"),
        Err(
            "an object (Set, record, enum, resource or function) key cannot be looked up; \
                 list the keys with 'keys'"
                .to_string()
        )
    );
}

#[test]
fn a_resource_shows_its_kind_flags_owner_and_fields() {
    let owner = ((0x16f0_0048_u64) << 12) - 2;
    let mut record = words(&[1, 3, 3, 0x2000, owner, 0x3000, 12, 1, 0, 0, 0, 1]);
    record.resize(112, 0);
    let fake = Fake::new(record.clone());
    assert_eq!(
        fake.show(BASE, &View::Resource),
        Ok(vec![format!(
            "Type: File Closed: Y Moved: Y Handle (fd): 3 State: 0x0000000000002000 Scope: \
                 0x{owner:x} (scope depth 2 of the frame whose token slot is 0x16f00048) Write \
                 Buffer: 0x3000 Write Filled: 12 Write Enabled: 1 Read Buffer: 0x0 Read \
                 Position: 0 Read Filled: 0 At EOF: 1"
        )])
    );
    record[0] = 99;
    assert_eq!(
        Fake::new(record).show(BASE, &View::Resource),
        Err(format!(
            "not a resource at 0x{BASE:016x}: its tag, 99, is no resource kind"
        ))
    );
}

#[test]
fn views_parse_their_types_indexes_and_ranges() {
    assert_eq!(Kind::parse("Integer"), Some(Kind::Scalar(Scalar::Integer)));
    assert_eq!(Kind::parse("bool"), Some(Kind::Scalar(Scalar::Boolean)));
    assert_eq!(Kind::parse("record"), None);
    assert_eq!(
        Span::parse("4"),
        Some(Span {
            start: 4,
            end: Some(5)
        })
    );
    assert_eq!(
        Span::parse("2.."),
        Some(Span {
            start: 2,
            end: None
        })
    );
    assert_eq!(Span::parse("5..5"), None);
    assert_eq!(Span::parse("18446744073709551615"), None);
    assert_eq!(
        parse_view("list", &["resource", "0..2"], ""),
        Ok(View::List {
            element: Some(Kind::Resource),
            span: Span::parse("0..2")
        })
    );
    assert_eq!(
        parse_view("map", &["get", "\"a"], "show type map x1 get \"a b\""),
        Ok(View::Map(MapView::Get("a b".to_string())))
    );
}
