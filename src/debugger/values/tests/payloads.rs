//! Payload decoding by type code, the collection headers and entries that
//! cannot be what they claim, every resource kind, and the `show type` forms
//! not covered beside it.

use super::*;

fn list() -> View {
    View::List {
        element: None,
        span: None,
    }
}

fn list_all(element: Option<Kind>) -> View {
    View::List {
        element,
        span: all(),
    }
}

/// A one-element fixed-width list of type code `code`, its payload `bytes`.
fn fixed(code: u8, bytes: &[u8]) -> Fake {
    Fake::new(collection(
        2,
        0,
        code,
        0,
        1,
        1,
        &[],
        bytes,
        bytes.len() as u64,
    ))
}

/// The value column of a one-row element table.
fn only_value(lines: Result<Vec<String>, String>) -> Result<String, String> {
    let lines = lines?;
    assert_eq!(lines.len(), 2, "{lines:?}");
    Ok(lines[1].rsplit(" | ").next().unwrap().to_string())
}

#[test]
fn every_fixed_width_element_type_decodes_by_its_code() {
    let cases: [(u8, Vec<u8>, &str); 7] = [
        (1, vec![0], "Nothing"),
        (2, vec![0], "0x00 (FALSE)"),
        (4, words(&[2.5_f64.to_bits()]), "0x4004000000000000 (2.5)"),
        (5, words(&[0x8000_0000]), "0x0000000080000000 (0.50)"),
        (7, vec![9], "0x09 (9)"),
        (8, words(&[250_000]), "0x000000000003d090 (2.50000)"),
        (
            9,
            0x41_u32.to_le_bytes().to_vec(),
            "0x00000041 (U+0041 'A')",
        ),
    ];
    for (code, bytes, shown) in cases {
        assert_eq!(
            only_value(fixed(code, &bytes).show(BASE, &list_all(None))),
            Ok(shown.to_string()),
            "type code {code}"
        );
    }
    // The summary names the element type.
    let summary = fixed(1, &[0]).show(BASE, &list()).unwrap();
    assert!(summary[0].starts_with("List Type: Nothing"), "{summary:?}");
}

#[test]
fn a_header_with_impossible_type_codes_or_sizes_is_refused() {
    let not = |what: &str, why: &str| Err(format!("not a {what} at 0x{BASE:016x}: {why}"));
    let set = View::Set {
        element: None,
        span: None,
    };
    let cases = [
        (
            collection(9, 0, 3, 0, 0, 0, &[], &[], 0),
            list(),
            not("List", "its kind byte is 9, which no collection has"),
        ),
        (
            collection(3, 6, 2, 0, 0, 0, &[], &[], 0),
            list(),
            not("List", "its kind byte is 3, a Set"),
        ),
        (
            collection(0, 0, 3, 0, 0, 0, &[], &[], 0),
            View::Map(MapView::Summary),
            not("Map", "its kind byte is 0, a List"),
        ),
        (
            collection(0, 6, 3, 0, 0, 0, &[], &[], 0),
            list(),
            not("List", "a list's key type is 0, not 6"),
        ),
        (
            collection(0, 0, 13, 0, 0, 0, &[], &[], 0),
            list(),
            not("List", "13 is not an element type code"),
        ),
        (
            collection(3, 6, 3, 0, 0, 0, &[], &[], 0),
            set,
            not("Set", "a set's element type code is 6 and its value type 3"),
        ),
        (
            collection(1, 0, 3, 0, 0, 0, &[], &[], 0),
            View::Map(MapView::Summary),
            not("Map", "0 and 3 are not key and value type codes"),
        ),
        (
            collection(0, 0, 3, 0, 0, 0, &[], &words(&[1, 2]), 8),
            list(),
            not("List", "its data length, 16, is over its data capacity, 8"),
        ),
        (
            collection(2, 0, 3, 0, 2, 2, &[], &words(&[1]), 8),
            list(),
            not("List", "2 elements of 8 bytes do not fit its 8 data bytes"),
        ),
        (
            collection(2, 0, 6, 0, 0, 0, &[], &[], 0),
            list(),
            not("List", "a fixed-width list of type code 6"),
        ),
    ];
    for (bytes, view, expected) in cases {
        assert_eq!(Fake::new(bytes).show(BASE, &view), expected);
    }
}

#[test]
fn an_entry_with_bad_flags_or_offsets_is_refused() {
    let flagged = Fake::new(collection(0, 0, 6, 0, 1, 1, &[(2, 0, 0, 0, 2)], b"hi", 8));
    assert_eq!(
        flagged.show(BASE, &list_all(None)),
        Err(format!("not a List at 0x{BASE:016x}: entry 0 has flags 2"))
    );
    let past = Fake::new(collection(0, 0, 6, 0, 1, 1, &[(1, 0, 0, 1, 5)], b"hi", 8));
    assert_eq!(
        past.show(BASE, &list_all(None)),
        Err(format!(
            "not a List at 0x{BASE:016x}: entry 0 points past its 2 data bytes"
        ))
    );
}

#[test]
fn a_range_past_the_end_names_the_first_element_missing() {
    let fake = fixed(3, &words(&[7]));
    let span = |start, end| View::List {
        element: None,
        span: Some(Span { start, end }),
    };
    assert_eq!(
        fake.show(BASE, &span(0, Some(3))),
        Err("no element 2: the List has 1 (numbered from 0)".to_string())
    );
    assert_eq!(
        fake.show(BASE, &span(4, None)),
        Err("no element 4: the List has 1 (numbered from 0)".to_string())
    );
}

#[test]
fn an_element_type_needs_an_index_or_range_after_it() {
    let fake = fixed(3, &words(&[7]));
    let wanted = Err("give an index or a range after the element type".to_string());
    assert_eq!(
        fake.show(
            BASE,
            &View::List {
                element: Some(Kind::Scalar(Scalar::Integer)),
                span: None
            }
        ),
        wanted
    );
    let set = Fake::new(collection(3, 3, 2, 0, 0, 1, &[], &[], 8));
    assert_eq!(
        set.show(
            BASE,
            &View::Set {
                element: Some(Kind::Scalar(Scalar::Integer)),
                span: None
            }
        ),
        wanted
    );
}

#[test]
fn a_map_summary_counts_live_keys_and_names_its_types() {
    let fake = Fake::new(collection(
        1,
        20,
        22,
        1,
        2,
        2,
        &[(1, 0, 8, 8, 8), (2, 0, 8, 8, 8)],
        &words(&[0, 0]),
        16,
    ));
    let summary = fake.show(BASE, &View::Map(MapView::Summary)).unwrap();
    assert_eq!(summary.len(), 1);
    assert!(
        summary[0].starts_with(
            "Map Keys: 1 Key Type: List Value Type: object (Set, record, enum, resource or \
             function) Removed: 1 Capacity: 2"
        ),
        "{summary:?}"
    );
}

#[test]
fn a_string_longer_than_the_limit_shows_its_first_bytes_whole_characters_only() {
    // MAX_TEXT bytes of 'a' but the last, then a two-byte 'é' that the limit
    // cuts in half.
    let mut text = vec![b'a'; MAX_TEXT as usize - 1];
    text.extend("é and more".as_bytes());
    let fake = Fake::new(string_record(&text));
    let lines = fake.show(BASE, &View::String).unwrap();
    let length = text.len();
    assert!(
        lines[0].starts_with(&format!("Length: {length} Value: \"aaaa")),
        "{}",
        &lines[0][..40]
    );
    assert!(
        lines[0].ends_with(&format!("aaa\" (the first {MAX_TEXT} bytes)")),
        "{}",
        &lines[0][lines[0].len() - 40..]
    );
    // An invalid byte that the limit does not cut is still refused.
    let fake = Fake::new(string_record(b"ok\xffno"));
    assert_eq!(
        fake.show(BASE, &View::String),
        Err(format!(
            "not a String at 0x{BASE:016x}: byte 2 of its value, 0xff, is not UTF-8"
        ))
    );
}

/// A list of type code `code` with one entry whose value is `payload`.
fn one_entry(code: u8, payload: &[u8]) -> Fake {
    Fake::new(collection(
        0,
        0,
        code,
        0,
        1,
        1,
        &[(1, 0, 0, 0, payload.len() as u64)],
        payload,
        payload.len().max(8) as u64,
    ))
}

#[test]
fn nested_collections_are_read_inline_or_through_their_pointer() {
    // An inline nested List: its whole block is the payload.
    let inner = fixed(3, &words(&[5]));
    let inline = one_entry(20, &inner.bytes);
    let shown = only_value(inline.show(BASE, &list_all(None))).unwrap();
    assert!(
        shown.starts_with("List Type: Integer Length: 1") && shown.ends_with(" (inline)"),
        "{shown}"
    );
    // A stored nested Map: an 8-byte pointer to a block elsewhere in the
    // arena. The outer list's block is 40 + 40 + 8 bytes; the map follows.
    let map = collection(1, 6, 3, 0, 0, 0, &[], &[], 0);
    let at = BASE + 88;
    let mut bytes = one_entry(21, &words(&[at])).bytes;
    assert_eq!(bytes.len(), 88);
    bytes.extend(&map);
    let shown = only_value(Fake::new(bytes).show(BASE, &list_all(None))).unwrap();
    assert!(
        shown.starts_with("Map Keys: 0 Key Type: String Value Type: Integer")
            && shown.ends_with(&format!(" (stored at 0x{at:016x})")),
        "{shown}"
    );
}

#[test]
fn type_code_22_is_read_as_the_kind_named_and_refuses_the_rest() {
    // A resource record follows the list block; the payload points at it.
    let at = BASE + 88;
    let mut bytes = one_entry(22, &words(&[at])).bytes;
    let mut record = words(&[13, 0x7000, 0, 0, 0, 0]);
    record.resize(112, 0);
    bytes.extend(&record);
    let fake = Fake::new(bytes);
    assert_eq!(
        only_value(fake.show(BASE, &list_all(None))),
        Ok("(type code 22: name it, as set or resource, to read it)".to_string())
    );
    assert_eq!(
        only_value(fake.show(BASE, &list_all(Some(Kind::Resource)))),
        Ok(
            "Type: Thread Closed: N Moved: N Handle: 0x7000 State: 0x0000000000000000 Scope: \
             none (not bound to a scope yet)"
                .to_string()
        )
    );
    assert_eq!(
        fake.show(BASE, &list_all(Some(Kind::String))),
        Err(
            "type code 22 holds a Set, a resource, a record, an enum or a function, not String"
                .to_string()
        )
    );
    // Named a Set, the pointer is followed to a Set header, which a resource
    // record is not.
    assert_eq!(
        fake.show(BASE, &list_all(Some(Kind::Set))),
        Err(format!(
            "not a Set at 0x{at:016x}: its kind byte is 13, which no collection has"
        ))
    );
    // A resource payload is a pointer, 8 bytes and no other size.
    let wide = one_entry(22, &[0; 16]);
    assert_eq!(
        wide.show(BASE, &list_all(Some(Kind::Resource))),
        Err("a resource payload is an 8-byte pointer, not 16 bytes".to_string())
    );
}

#[test]
fn an_element_read_as_another_type_or_size_is_refused() {
    let strings = one_entry(6, b"hi");
    assert_eq!(
        strings.show(BASE, &list_all(Some(Kind::List))),
        Err("the elements are String, not List".to_string())
    );
    for (kind, name) in [
        (Kind::Set, "Set"),
        (Kind::Map, "Map"),
        (Kind::Resource, "resource"),
    ] {
        assert_eq!(
            strings.show(BASE, &list_all(Some(kind))),
            Err(format!("the elements are String, not {name}"))
        );
    }
    // An Integer entry whose payload is not 8 bytes.
    let short = one_entry(3, &[1, 2, 3, 4]);
    assert_eq!(
        short.show(BASE, &list_all(None)),
        Err("an Integer payload is 8 bytes, not 4".to_string())
    );
    // An empty String element reads no bytes.
    let empty = one_entry(6, b"");
    assert_eq!(
        only_value(empty.show(BASE, &list_all(None))),
        Ok("\"\"".to_string())
    );
}

#[test]
fn keys_of_every_lookup_type_are_encoded() {
    assert_eq!(
        key_bytes(COLLECTION_TYPE_FLOAT, "2.5"),
        Ok(words(&[2.5_f64.to_bits()]))
    );
    assert_eq!(
        key_bytes(COLLECTION_TYPE_FLOAT, "x"),
        Err("'x' is not a Float".to_string())
    );
    assert_eq!(
        key_bytes(COLLECTION_TYPE_FIXED, "0.5"),
        Ok(words(&[0x8000_0000]))
    );
    assert_eq!(
        key_bytes(COLLECTION_TYPE_FIXED, "x"),
        Err("'x' is not a Fixed".to_string())
    );
    assert_eq!(
        key_bytes(COLLECTION_TYPE_MONEY, "2.5"),
        Ok(words(&[250_000]))
    );
    assert_eq!(
        key_bytes(COLLECTION_TYPE_MONEY, "x"),
        Err("'x' is not a Money amount".to_string())
    );
    assert_eq!(key_bytes(COLLECTION_TYPE_BOOLEAN, "false"), Ok(vec![0]));
    assert_eq!(
        key_bytes(COLLECTION_TYPE_BOOLEAN, "yes"),
        Err("'yes' is not TRUE or FALSE".to_string())
    );
    assert_eq!(
        key_bytes(COLLECTION_TYPE_SCALAR, "é"),
        Ok(0xe9_u32.to_le_bytes().to_vec())
    );
    assert_eq!(key_bytes(COLLECTION_TYPE_NOTHING, "Nothing"), Ok(vec![0]));
}

#[test]
fn every_resource_kind_names_its_handle_and_its_own_fields() {
    let show = |tag: u64| {
        let mut record = words(&[tag, 5, 0, 0, 0, 0x10, 0x20, 0x30, 0x40, 0x50]);
        record.resize(112, 0);
        Fake::new(record)
            .show(BASE, &View::Resource)
            .unwrap()
            .remove(0)
    };
    let cases = [
        (2, "Socket", "Handle (fd): 5", ""),
        (3, "UdpSocket", "Handle (fd): 5", ""),
        (4, "Listener", "Handle (fd): 5", ""),
        (
            5,
            "tls::Socket (OpenSSL)",
            "Handle (fd): 5",
            " SSL_CTX: 0x10 SSL: 0x20",
        ),
        (
            6,
            "tls::Socket (Network.framework)",
            "Handle (connection): 0x5",
            " Connection Context: 0x10 Dispatch Queue: 0x20",
        ),
        (
            7,
            "tls::Socket (SChannel)",
            "Handle (fd): 5",
            " SSPI Block: 0x20",
        ),
        (8, "tls::Listener", "Handle (fd): 5", " SSL_CTX: 0x10"),
        (
            9,
            "audio stream",
            "Handle (kind): 5",
            " Sample Rate: 16 Channels: 32 Bytes Per Frame: 48 Buffer Frames: 64 Audio State: \
             0x50",
        ),
        (10, "Process", "Handle: 0x5", ""),
        (11, "Image", "Handle: 0x5", ""),
        (12, "Font", "Handle: 0x5", ""),
        (14, "ThreadWorker", "Handle: 0x5", ""),
        (
            255,
            "native LINK resource",
            "Handle (native pointer): 0x5",
            "",
        ),
    ];
    for (tag, name, handle, tail) in cases {
        assert_eq!(
            show(tag),
            format!(
                "Type: {name} Closed: N Moved: N {handle} State: 0x0000000000000000 Scope: \
                 none (not bound to a scope yet){tail}"
            ),
            "tag {tag}"
        );
    }
}

#[test]
fn the_rest_of_the_show_type_forms_parse() {
    assert_eq!(parse_view("resource", &[], ""), Ok(View::Resource));
    assert_eq!(
        parse_view("resource", &["x"], ""),
        Err("too many arguments to 'show type resource'".to_string())
    );
    assert_eq!(
        parse_view("list", &["string"], ""),
        Ok(View::List {
            element: Some(Kind::String),
            span: None
        })
    );
    assert_eq!(
        parse_view("set", &["0..2"], ""),
        Ok(View::Set {
            element: None,
            span: Span::parse("0..2")
        })
    );
    assert_eq!(
        parse_view("list", &["int", "0", "1"], ""),
        Err("too many arguments to 'show type list'".to_string())
    );
    assert_eq!(
        parse_view("map", &["get"], "show type map x1 get"),
        Err("'get' needs a key: show type map <where> get <key>".to_string())
    );
    assert_eq!(
        parse_view("map", &["3"], ""),
        Ok(View::Map(MapView::Entry(3)))
    );
    assert_eq!(
        parse_view("map", &["keys", "string", "x"], ""),
        Err("too many arguments to 'show type map'".to_string())
    );
}
