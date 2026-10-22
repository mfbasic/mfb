// ---------------------------------------------------------------------------
// reader.rs — every truncation of a `.mfp` is refused, and tables with a
// short payload or trailing bytes are refused.
// ---------------------------------------------------------------------------

use super::fixtures::*;
use super::*;

#[test]
fn every_truncation_of_an_mfp_is_refused() {
    let inner = encode_project(
        &rich_project(),
        &BinaryReprMetadata::new("richpkg".to_string(), "1.0.0".to_string()),
    );
    let good = wrap_mfp(&inner, "richpkg", "richpkg", "1.0.0");
    assert!(mfp_binary_repr_payload(&good).is_ok());
    let messages: std::collections::BTreeSet<String> = (0..good.len())
        .map(|length| {
            mfp_binary_repr_payload(&good[..length])
                .map(|_| ())
                .expect_err("a truncated file is refused")
        })
        .collect();
    // The fixed-size fields have their own refusals.
    assert!(
        messages.contains("truncated .mfp packageBinaryHash"),
        "{messages:?}"
    );
}

#[test]
fn tables_with_a_short_payload_or_trailing_bytes_are_refused() {
    let err = |result: Result<(), String>| result.unwrap_err();
    // One const of kind 3 claiming 10 bytes, with 2.
    let mut short = 1u32.to_le_bytes().to_vec();
    short.extend(3u16.to_le_bytes());
    short.extend(0u16.to_le_bytes());
    short.extend(10u32.to_le_bytes());
    short.extend([0, 0]);
    assert_eq!(
        err(read_const_pool(&short).map(|_| ())),
        "truncated const payload"
    );
    let trailing = [0u8, 0, 0, 0, 0xff];
    assert_eq!(
        err(read_const_pool(&trailing).map(|_| ())),
        "invalid trailing bytes in const pool"
    );
    assert_eq!(
        err(read_import_table(&trailing).map(|_| ())),
        "invalid trailing bytes in import table"
    );
    assert_eq!(
        err(read_export_table(&trailing).map(|_| ())),
        "invalid trailing bytes in export table"
    );
}
