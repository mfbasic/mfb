// ---------------------------------------------------------------------------
// util.rs — low-level cursor readers, capacity guards, section framing.
// ---------------------------------------------------------------------------

use super::*;

#[test]
fn cursor_scalars_round_trip() {
    let mut bytes = Vec::new();
    bytes.push(0xAB);
    put_u16(&mut bytes, 0x1234);
    put_u32(&mut bytes, 0xDEAD_BEEF);
    put_u64(&mut bytes, 0x0102_0304_0506_0708);
    let mut offset = 0;
    assert_eq!(cursor_u8(&bytes, &mut offset).unwrap(), 0xAB);
    assert_eq!(cursor_u16(&bytes, &mut offset).unwrap(), 0x1234);
    assert_eq!(cursor_u32(&bytes, &mut offset).unwrap(), 0xDEAD_BEEF);
    assert_eq!(
        cursor_u64(&bytes, &mut offset).unwrap(),
        0x0102_0304_0506_0708
    );
    assert_eq!(offset, bytes.len());
}

#[test]
fn cursor_scalars_reject_truncation() {
    let mut o = 0;
    assert!(cursor_u8(&[], &mut o).is_err());
    let mut o = 0;
    assert!(cursor_u16(&[0], &mut o).is_err());
    let mut o = 0;
    assert!(cursor_u32(&[0, 0, 0], &mut o).is_err());
    let mut o = 0;
    assert!(cursor_u64(&[0; 7], &mut o).is_err());
    let mut o = 0;
    assert!(cursor_hash(&[0; 31], &mut o).is_err());
}

#[test]
fn cursor_string_round_trips_and_rejects_bad_input() {
    let mut bytes = Vec::new();
    put_bytes(&mut bytes, "héllo".as_bytes());
    let mut offset = 0;
    assert_eq!(cursor_string(&bytes, &mut offset).unwrap(), "héllo");
    assert_eq!(offset, bytes.len());

    // Truncated body (claims 10 bytes, has 2).
    let mut bad = Vec::new();
    put_u32(&mut bad, 10);
    bad.extend_from_slice(b"ab");
    let mut o = 0;
    assert!(cursor_string(&bad, &mut o).is_err());

    // Invalid UTF-8 body.
    let mut invalid = Vec::new();
    put_u32(&mut invalid, 1);
    invalid.push(0xFF);
    let mut o = 0;
    assert!(cursor_string(&invalid, &mut o).is_err());
}

#[test]
fn cursor_hash_reads_thirty_two_bytes() {
    let data: Vec<u8> = (0..32u8).collect();
    let mut offset = 0;
    let hash = cursor_hash(&data, &mut offset).unwrap();
    assert_eq!(hash.to_vec(), data);
    assert_eq!(offset, 32);
}

#[test]
fn cursor_prose_and_pair_and_optional_round_trip() {
    let mut bytes = Vec::new();
    put_prose_list(&mut bytes, &[(0, "a".to_string()), (2, "b".to_string())]);
    put_pair_list(&mut bytes, &[("k".to_string(), "v".to_string())]);
    put_optional_str(&mut bytes, &Some("present".to_string()));
    put_optional_str(&mut bytes, &None);

    let mut offset = 0;
    let prose = cursor_prose_list(&bytes, &mut offset).unwrap();
    assert_eq!(prose, vec![(0, "a".to_string()), (2, "b".to_string())]);
    let pairs = cursor_pair_list(&bytes, &mut offset).unwrap();
    assert_eq!(pairs, vec![("k".to_string(), "v".to_string())]);
    assert_eq!(
        cursor_optional_str(&bytes, &mut offset).unwrap(),
        Some("present".to_string())
    );
    assert_eq!(cursor_optional_str(&bytes, &mut offset).unwrap(), None);
    assert_eq!(offset, bytes.len());
}

#[test]
fn cursor_prose_list_rejects_truncated_kind() {
    // Count says 1 element but no kind byte follows.
    let mut bytes = Vec::new();
    put_u32(&mut bytes, 1);
    let mut o = 0;
    assert!(cursor_prose_list(&bytes, &mut o).is_err());
}

#[test]
fn cursor_optional_str_rejects_truncated_flag() {
    let mut o = 0;
    assert!(cursor_optional_str(&[], &mut o).is_err());
}

#[test]
fn bounded_capacity_caps_to_remaining() {
    // A hostile count is clamped by remaining/min_elem.
    assert_eq!(bounded_capacity(u32::MAX as usize, 40, 8), 5);
    // A small count passes through unchanged.
    assert_eq!(bounded_capacity(3, 40, 8), 3);
    // min_elem of 0 is treated as 1 (no div-by-zero).
    assert_eq!(bounded_capacity(7, 100, 0), 7);
}

#[test]
fn hash_bytes_and_hex_hash_are_consistent() {
    let hash = hash_bytes(b"abc");
    assert_eq!(hash.len(), ABI_HASH_LEN);
    let hex = hex_hash(&hash);
    assert_eq!(hex.len(), ABI_HASH_LEN * 2);
    // SHA-256("abc") starts with ba7816bf.
    assert!(hex.starts_with("ba7816bf"));
}

#[test]
fn sorted_pairs_orders_lexicographically() {
    let sorted = sorted_pairs(vec![
        ("b".to_string(), "2".to_string()),
        ("a".to_string(), "1".to_string()),
    ]);
    assert_eq!(sorted[0].0, "a");
    assert_eq!(sorted[1].0, "b");
}

#[test]
fn length_prefixed_helpers_round_trip_and_reject() {
    let mut bytes = Vec::new();
    put_bytes(&mut bytes, b"payload");
    put_bytes(&mut bytes, b"skip-me");
    let mut offset = 0;
    assert_eq!(
        read_length_prefixed(&bytes, &mut offset, "f").unwrap(),
        "payload"
    );
    skip_length_prefixed(&bytes, &mut offset, "g").unwrap();
    assert_eq!(offset, bytes.len());

    // Truncated: claims a long length.
    let mut bad = Vec::new();
    put_u32(&mut bad, 100);
    bad.extend_from_slice(b"x");
    let mut o = 0;
    assert!(read_length_prefixed(&bad, &mut o, "f").is_err());
    let mut o = 0;
    assert!(skip_length_prefixed(&bad, &mut o, "f").is_err());

    // Non-UTF8 in read_length_prefixed.
    let mut invalid = Vec::new();
    put_u32(&mut invalid, 1);
    invalid.push(0xFF);
    let mut o = 0;
    assert!(read_length_prefixed(&invalid, &mut o, "f").is_err());
}

#[test]
fn checked_scalars_reject_out_of_bounds() {
    assert!(checked_u16_at(&[0], 0).is_err());
    assert!(checked_u32_at(&[0, 0], 0).is_err());
    assert!(checked_u64_at(&[0; 4], 0).is_err());
    // Overflowing offset.
    assert!(checked_u16_at(&[0; 4], usize::MAX).is_err());
}

#[test]
fn encode_sections_frames_header_and_offsets() {
    let sections = vec![Section::new(1, vec![1, 2, 3]), Section::new(2, vec![9, 9])];
    let bytes = encode_sections(&sections);
    assert_eq!(&bytes[0..4], b"MFPC");
    // major version at offset 4.
    assert_eq!(checked_u16_at(&bytes, 4).unwrap(), MFPC_MAJOR_VERSION);
    // section count at offset 12.
    assert_eq!(checked_u32_at(&bytes, 12).unwrap(), 2);
    // First section table entry: id 1, offset points past the header+table.
    assert_eq!(checked_u16_at(&bytes, 16).unwrap(), 1);
    let first_off = checked_u64_at(&bytes, 16 + 8).unwrap() as usize;
    assert_eq!(first_off, 16 + 2 * 24);
    assert_eq!(&bytes[first_off..first_off + 3], &[1, 2, 3]);
}

/// plan-61-D Phase 2: section 18 round-trips, and is **omitted entirely**
/// when there is no description — an empty section would change the bytes
/// of every package that has none, which is the thing this design exists to
/// avoid.
#[test]
fn package_meta_section_round_trips_and_is_omitted_when_empty() {
    assert!(
        encode_package_meta("").is_none(),
        "no description means no section at all, not an empty one",
    );

    let encoded = encode_package_meta("A demo package.").expect("a description encodes");
    assert_eq!(read_package_meta(&encoded).unwrap(), "A demo package.");

    // UTF-8 survives, and the cap is counted in *bytes*, not characters.
    let unicode = "描述 — naïve café 🎵";
    let encoded = encode_package_meta(unicode).unwrap();
    assert_eq!(read_package_meta(&encoded).unwrap(), unicode);
}

/// Unknown field ids inside section 18 are **skipped**, not rejected. That
/// is what makes a later field (`license`, `keywords`) additive within the
/// section, exactly as the section itself is additive within the container.
#[test]
fn an_unknown_package_meta_field_id_is_skipped_not_rejected() {
    let mut bytes = Vec::new();
    put_u32(&mut bytes, 3); // three fields
                            // An unknown id *before* the description, so a reader that bailed on
                            // the first unknown field would never reach the value it does know.
    put_u16(&mut bytes, 999);
    put_u32(&mut bytes, 5);
    bytes.extend_from_slice(b"skipme");
    // ...that was 6 bytes declared as 5, so trim to keep the frame honest.
    bytes.truncate(bytes.len() - 1);
    put_u16(&mut bytes, PACKAGE_META_FIELD_DESCRIPTION);
    put_u32(&mut bytes, 4);
    bytes.extend_from_slice(b"real");
    // And an unknown id *after* it too.
    put_u16(&mut bytes, 1000);
    put_u32(&mut bytes, 2);
    bytes.extend_from_slice(b"xy");

    assert_eq!(
        read_package_meta(&bytes).unwrap(),
        "real",
        "unknown field ids must be skipped on both sides of a known one",
    );
}

/// The 4096-byte cap is re-checked at section-read time, not trusted from
/// manifest validation — a hand-built payload never went through the
/// manifest.
#[test]
fn an_over_cap_description_is_rejected_at_read_time() {
    let mut bytes = Vec::new();
    let oversized = "x".repeat(crate::manifest::MAX_DESCRIPTION_BYTES + 1);
    put_u32(&mut bytes, 1);
    put_u16(&mut bytes, PACKAGE_META_FIELD_DESCRIPTION);
    put_u32(&mut bytes, oversized.len() as u32);
    bytes.extend_from_slice(oversized.as_bytes());

    let err = read_package_meta(&bytes).unwrap_err();
    assert!(err.contains("exceeds the 4096 byte limit"), "{err}");

    // A field claiming more bytes than the section holds is truncation, not
    // a cap violation, and must not read past the end.
    let mut bytes = Vec::new();
    put_u32(&mut bytes, 1);
    put_u16(&mut bytes, PACKAGE_META_FIELD_DESCRIPTION);
    put_u32(&mut bytes, 100);
    bytes.extend_from_slice(b"short");
    assert!(read_package_meta(&bytes).is_err());
}

#[test]
fn hex_dump_formats_rows_of_sixteen() {
    let out = hex_dump(&[0xAB, 0x00, 0xFF]);
    assert_eq!(out, "AB 00 FF\n");
    // 17 bytes wraps to a second line.
    let data: Vec<u8> = (0..17u8).collect();
    let dump = hex_dump(&data);
    assert_eq!(dump.lines().count(), 2);
}
