//! The MFPC container: frozen section ids, wire enums, and the one
//! section-table reader (plan-126-C).
//!
//! # Why this module exists
//!
//! The section table was decoded **twice**, by two independently-written
//! functions over bytes that a package signature covers. The registry's copy
//! even carried the comment "matches the compiler reader" on its duplicate-id
//! check — the tell that two copies were being kept in step by hand. They had
//! already drifted, in *both* directions:
//!
//! | guard | compiler had | registry had |
//! |---|---|---|
//! | MFPC magic | yes | yes |
//! | MFPC **major version** == 2 | **yes** | **no** |
//! | declared section-count ceiling | **no** | **yes** (bug-578) |
//! | table-length overflow (`checked_add`/`checked_mul`) | yes | yes |
//! | truncated table / truncated section | yes | yes |
//! | duplicate section id (PKG-06) | yes | yes |
//! | `u64` → `usize` conversion | **`checked_usize`** | **`as usize`** |
//!
//! [`read_section_table`] enforces the **union**: every compiler guard plus the
//! registry's [`MAX_MFPC_SECTIONS`] ceiling. The plan that produced this module
//! predicted the union would simply be the compiler's set; it is not, and the
//! extra row means the *compiler* gained a guard here too, not only the
//! registry. See plan-126-C's Corrections.
//!
//! # The failure mode this protects against is silent
//!
//! On the registry side a stricter reader does not reject a publish — every
//! parse error there maps to "no such section" and an empty metadata field
//! (`abi_index_json` → `{}`). So a missing guard does not announce itself; it
//! records wrong metadata for a package the compiler would refuse to load. That
//! is why the guards are tested by asserting each one *fires*, rather than by
//! observing that a publish still succeeds.

use crate::bytes::{
    checked_u16_at, checked_u32_at, checked_u64_at, checked_usize, put_u16, put_u32, put_u64,
};
use std::collections::BTreeMap;

/// MFPC container magic: the first four bytes of a `packageBinaryRepr` payload.
pub const MFPC_MAGIC: &[u8; 4] = b"MFPC";

/// MFPC container major version.
///
/// Bumped to 2 for the clean break to the structured Binary Representation
/// payload; the reader rejects the old flat (v1) layout outright rather than
/// attempting it. [`encode_sections`] stamps it and [`read_section_table`]
/// checks it — the writer and the reader of the one field now sit in the same
/// module, which is why this is the only definition (plan-126-C; `bytes.rs`
/// briefly held a second copy).
pub const MFPC_MAJOR_VERSION: u16 = 2;

/// One MFPC section: a frozen wire id and its opaque body.
pub struct Section {
    pub id: u16,
    pub data: Vec<u8>,
}

impl Section {
    pub fn new(id: u16, data: Vec<u8>) -> Self {
        Self { id, data }
    }
}

/// Frame `sections` as an MFPC container: the 16-byte header, a 24-byte table
/// entry per section, then the bodies in order.
///
/// The inverse of [`read_section_table`], and deliberately kept beside it so a
/// change to the table layout cannot update one without the other being on
/// screen. `encode_sections_round_trips_through_read_section_table` holds them
/// to each other.
pub fn encode_sections(sections: &[Section]) -> Vec<u8> {
    let section_table_size = sections.len() * 24;
    let mut offset = 16 + section_table_size;
    let mut bytes = Vec::new();

    bytes.extend_from_slice(MFPC_MAGIC);
    put_u16(&mut bytes, MFPC_MAJOR_VERSION);
    put_u16(&mut bytes, 0);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, sections.len() as u32);

    for section in sections {
        put_u16(&mut bytes, section.id);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, 0);
        put_u64(&mut bytes, offset as u64);
        put_u64(&mut bytes, section.data.len() as u64);
        offset += section.data.len();
    }

    for section in sections {
        bytes.extend_from_slice(&section.data);
    }

    bytes
}

/// Ceiling on the **declared** section count, checked before the table is
/// walked or anything is inserted (bug-578).
///
/// The writer defines fourteen section ids (1..=8, 10, 11, 15..=18) and emits at
/// most all fourteen — measured across every committed compiler-produced
/// package, where the maximum observed is 14 (libsnd, sqlite3); the table of
/// per-package maxima lives beside the registry's other format ceilings in
/// `repository/src/abi.rs`. 256 leaves room for every section id the format is
/// likely ever to define while capping the table walk, which the body-length
/// check alone bounded at roughly 2.7 million 24-byte entries.
///
/// **It bounds damage; it is not a schema.** A container declaring exactly 256
/// sections is legal and must decode, which
/// `a_declared_section_count_over_the_ceiling_is_rejected` asserts in both
/// directions.
pub const MAX_MFPC_SECTIONS: usize = 256;

// === Frozen section ids ====================================================
// Wire format. Declared in numeric order. Ids 12-14 are reserved by the format
// for DEBUG_INFO/SOURCE_MAP/AUDIT_INFO, and ids 9 and 19 are unassigned gaps.

pub const SECTION_MANIFEST: u16 = 1;
pub const SECTION_STRING_POOL: u16 = 2;
pub const SECTION_TYPE_TABLE: u16 = 3;
pub const SECTION_CONST_POOL: u16 = 4;
pub const SECTION_IMPORT_TABLE: u16 = 5;
pub const SECTION_EXPORT_TABLE: u16 = 6;
pub const SECTION_GLOBAL_TABLE: u16 = 7;
pub const SECTION_FUNCTION_TABLE: u16 = 8;
/// Optional native-library locator table (plan-46-B §4.1). Emitted only for a
/// binding package that declares a `LINK` block; the container's optional flag
/// bit 0 ("contains native LINK metadata") is set alongside it.
pub const SECTION_NATIVE_LIBRARY_TABLE: u16 = 10;
pub const SECTION_RESOURCE_TABLE: u16 = 11;
pub const SECTION_ABI_INDEX: u16 = 15;
/// Structured Binary Representation payload section. Replaces the old flat code
/// section as the carrier of function bodies.
pub const SECTION_BINARY_REPR: u16 = 16;
/// Optional documentation section (plan-09-doc.md §5). Self-describing and
/// length-prefixed; a consumer that does not understand it skips it entirely.
pub const SECTION_DOC_TABLE: u16 = 17;
/// Optional human-facing package metadata (plan-61-D).
///
/// Named `PACKAGE_META` rather than `DESCRIPTION` so `license`/`keywords` can
/// join it later without consuming another section id. Self-contained and
/// length-prefixed like the DOC section: it does **not** intern into the string
/// pool, so it can be parsed without section 2.
///
/// **Never put security-relevant data here.** The format has no
/// "critical section" marker, so a reader that predates this section accepts a
/// package carrying it and silently ignores the contents. That is exactly right
/// for a description — a missing one is cosmetic — and exactly wrong for
/// anything a consumer must not miss.
pub const SECTION_PACKAGE_META: u16 = 18;
/// Field ids within section 18. Unknown ids are **skipped**, not rejected, so a
/// later field is additive within the section just as the section itself is
/// additive within the container.
pub const PACKAGE_META_FIELD_DESCRIPTION: u16 = 1;

/// ABI signature-hash input format version.
///
/// bug-277 moved kind-11 (`STATE`) composites from opaque to structural hashing,
/// which shifts the `sigHash` of a stateful export — but deliberately did NOT
/// bump this. The gate in `read_abi_index` guards the section's *wire encoding*,
/// which that change leaves untouched; bumping it would reject every
/// previously-built `.mfp` wholesale, including the overwhelming majority that
/// export no `STATE` type at all. A package that does carry a stale kind-11 hash
/// is already rejected precisely, per symbol, by `validate_abi_index`
/// recomputing it from the function table. Bump this only for an actual
/// ABI_INDEX layout change.
pub const ABI_FORMAT_VERSION: u16 = 1;

/// Byte cap on a package description carried in section 18.
///
/// Re-checked at section-read time rather than trusted from manifest
/// validation: a hand-built payload never went through the manifest.
pub const MAX_DESCRIPTION_BYTES: usize = 4096;

/// Length of a native-library SHA-256 in section 10.
pub const NATIVE_LIBRARY_HASH_LEN: usize = 32;

// === Native-library wire enums (plan-46-B §4.1) ============================

/// No `libc` constraint: the locator matches any libc.
pub const WIRE_LIBC_UNSPECIFIED: u8 = 0;
pub const WIRE_LIBC_GLIBC: u8 = 1;
pub const WIRE_LIBC_MUSL: u8 = 2;

/// A `system` library: a file the registry never sees, carrying no hash.
pub const WIRE_LIB_TYPE_SYSTEM: u8 = 0;
/// A `vendor` library: carries a 32-byte SHA-256 of the file.
pub const WIRE_LIB_TYPE_VENDOR: u8 = 1;

/// Decode an MFPC container's section table into `id -> body` (plan-126-C).
///
/// A `BTreeMap` rather than a `HashMap` so iteration order is deterministic —
/// a `HashMap` deciding an emission or reporting order has produced flaky
/// goldens in this tree before.
///
/// Enforces the **union** of the guards the two previous copies had between
/// them; see the module doc for the table and for which side was missing what.
/// In particular a **duplicate section id is always tampering**: every MFPC
/// section is a singleton, and `insert` silently keeping the last copy would let
/// a crafted package ship two views of one section — one to satisfy a cheap
/// inspector, the other to be decoded and lowered (PKG-06).
pub fn read_section_table(bytes: &[u8]) -> Result<BTreeMap<u16, &[u8]>, String> {
    if bytes.len() < 16 || &bytes[0..4] != MFPC_MAGIC {
        return Err(
            "package payload does not have the binary representation container magic".to_string(),
        );
    }
    // The guard the registry's copy lacked. A v1 container is not a damaged v2
    // one: its payload layout is different throughout, so attempting it would
    // mis-decode rather than fail cleanly.
    let major = checked_u16_at(bytes, 4)?;
    if major != MFPC_MAJOR_VERSION {
        return Err(format!(
            "unsupported MFPC major version {major} (expected {MFPC_MAJOR_VERSION}); \
             this package predates the structured Binary Representation format and must be rebuilt"
        ));
    }
    let section_count = checked_u32_at(bytes, 12)? as usize;
    // The guard the *compiler's* copy lacked (bug-578). Reject on the declared
    // count before walking or inserting anything: the body-length check alone
    // allowed ~2.7 million table entries.
    if section_count > MAX_MFPC_SECTIONS {
        return Err(format!(
            "MFPC container declares {section_count} sections (limit {MAX_MFPC_SECTIONS})"
        ));
    }
    let table_end = 16usize
        .checked_add(
            section_count
                .checked_mul(24)
                .ok_or_else(|| "invalid MFPC section table length".to_string())?,
        )
        .ok_or_else(|| "invalid MFPC section table length".to_string())?;
    if table_end > bytes.len() {
        return Err("truncated MFPC section table".to_string());
    }

    let mut sections = BTreeMap::new();
    for index in 0..section_count {
        let entry = 16 + index * 24;
        let id = checked_u16_at(bytes, entry)?;
        // `checked_usize`, not `as usize` — the conversion the registry's copy
        // got wrong. `as usize` truncates on a 32-bit target, so a declared
        // offset of `0x1_0000_0000` would have its bounds validated against 0
        // and every structural check downstream would pass on a length that
        // does not describe the real body.
        let offset = checked_usize(checked_u64_at(bytes, entry + 8)?, "MFPC section offset")?;
        let length = checked_usize(checked_u64_at(bytes, entry + 16)?, "MFPC section length")?;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| "invalid MFPC section length".to_string())?;
        if end > bytes.len() {
            return Err("truncated MFPC section".to_string());
        }
        if sections.insert(id, &bytes[offset..end]).is_some() {
            return Err(format!("duplicate MFPC section id {id}"));
        }
    }

    Ok(sections)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::{put_u16, put_u32, put_u64};

    /// Frame a container with a hand-built table, so each test can violate one
    /// rule at a time. `count_override` lets a test declare more sections than
    /// it writes.
    fn container(sections: &[(u16, Vec<u8>)], major: u16, count_override: Option<u32>) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MFPC_MAGIC);
        put_u16(&mut bytes, major);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, 0);
        put_u32(&mut bytes, count_override.unwrap_or(sections.len() as u32));
        let mut data_offset = 16 + sections.len() * 24;
        for (id, data) in sections {
            put_u16(&mut bytes, *id);
            put_u16(&mut bytes, 0);
            put_u32(&mut bytes, 0);
            put_u64(&mut bytes, data_offset as u64);
            put_u64(&mut bytes, data.len() as u64);
            data_offset += data.len();
        }
        for (_id, data) in sections {
            bytes.extend_from_slice(data);
        }
        bytes
    }

    /// Moved from `bytes.rs` with `encode_sections` (plan-126-C): the header and
    /// the first table entry land at the offsets the format fixes.
    #[test]
    fn encode_sections_frames_header_and_offsets() {
        let sections = vec![Section::new(1, vec![1, 2, 3]), Section::new(2, vec![9, 9])];
        let bytes = encode_sections(&sections);
        assert_eq!(&bytes[0..4], MFPC_MAGIC);
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

    /// The writer and the reader now live side by side, so hold them to each
    /// other: whatever `encode_sections` frames, `read_section_table` must
    /// decode back to the same id→body map. Before plan-126-C they sat in
    /// different files (the reader in two copies), and nothing tested the pair.
    #[test]
    fn encode_sections_round_trips_through_read_section_table() {
        let sections = vec![
            Section::new(SECTION_MANIFEST, vec![1, 2, 3]),
            Section::new(SECTION_STRING_POOL, Vec::new()),
            Section::new(SECTION_ABI_INDEX, vec![0xAB; 40]),
        ];
        let bytes = encode_sections(&sections);
        let table = read_section_table(&bytes).expect("an encoded container decodes");
        assert_eq!(table.len(), sections.len());
        for section in &sections {
            assert_eq!(
                table[&section.id],
                section.data.as_slice(),
                "section {} must round-trip byte-for-byte",
                section.id
            );
        }
    }

    fn ok_container() -> Vec<u8> {
        container(
            &[
                (SECTION_MANIFEST, vec![1, 2, 3]),
                (SECTION_STRING_POOL, vec![4, 5]),
            ],
            MFPC_MAJOR_VERSION,
            None,
        )
    }

    #[test]
    fn a_well_formed_table_decodes_in_id_order() {
        let bytes = ok_container();
        let table = read_section_table(&bytes).expect("valid container");
        assert_eq!(table.len(), 2);
        assert_eq!(table[&SECTION_MANIFEST], &[1, 2, 3]);
        assert_eq!(table[&SECTION_STRING_POOL], &[4, 5]);
        // BTreeMap, so iteration is by id and deterministic.
        assert_eq!(
            table.keys().copied().collect::<Vec<_>>(),
            vec![SECTION_MANIFEST, SECTION_STRING_POOL],
        );
    }

    #[test]
    fn a_payload_without_the_magic_is_refused() {
        assert!(read_section_table(b"not a container at all!!").is_err());
        // Too short to even hold a header.
        assert!(read_section_table(b"MFPC").is_err());
    }

    /// The guard the **registry** was missing. A v1 container's payload layout
    /// differs throughout, so attempting it would mis-decode rather than fail.
    #[test]
    fn a_container_declaring_major_version_three_is_rejected() {
        let bytes = container(&[(SECTION_MANIFEST, vec![1])], 3, None);
        let err = read_section_table(&bytes).unwrap_err();
        assert!(err.contains("unsupported MFPC major version 3"), "{err}");
        assert!(err.contains("must be rebuilt"), "{err}");

        // And v1, the layout the major bump exists to reject.
        let bytes = container(&[(SECTION_MANIFEST, vec![1])], 1, None);
        assert!(read_section_table(&bytes)
            .unwrap_err()
            .contains("unsupported MFPC major version 1"));
    }

    /// The guard the **compiler** was missing (bug-578). Reject on the declared
    /// count before walking, so a hostile count cannot drive the loop.
    #[test]
    fn a_declared_section_count_over_the_ceiling_is_rejected() {
        let bytes = container(
            &[(SECTION_MANIFEST, vec![1])],
            MFPC_MAJOR_VERSION,
            Some(MAX_MFPC_SECTIONS as u32 + 1),
        );
        let err = read_section_table(&bytes).unwrap_err();
        assert!(
            err.contains(&format!("declares {} sections", MAX_MFPC_SECTIONS + 1)),
            "{err}"
        );
        assert!(err.contains(&format!("limit {MAX_MFPC_SECTIONS}")), "{err}");

        // Exactly at the ceiling is allowed — the cap bounds damage, it is not
        // a schema, so it must not reject a legal container.
        let sections: Vec<(u16, Vec<u8>)> = (0..MAX_MFPC_SECTIONS)
            .map(|i| (i as u16, vec![0u8]))
            .collect();
        let bytes = container(&sections, MFPC_MAJOR_VERSION, None);
        let table = read_section_table(&bytes).expect("a table at the ceiling is valid");
        assert_eq!(table.len(), MAX_MFPC_SECTIONS);
    }

    #[test]
    fn a_truncated_section_table_is_rejected() {
        // Declare two sections but cut the table short.
        let mut bytes = ok_container();
        bytes.truncate(16 + 24);
        let err = read_section_table(&bytes).unwrap_err();
        assert_eq!(err, "truncated MFPC section table");
    }

    #[test]
    fn a_section_body_past_the_end_is_rejected() {
        let mut bytes = ok_container();
        // Overstate the first section's length.
        let length_at = 16 + 16;
        bytes[length_at..length_at + 8].copy_from_slice(&9999u64.to_le_bytes());
        let err = read_section_table(&bytes).unwrap_err();
        assert_eq!(err, "truncated MFPC section");
    }

    /// PKG-06. Every MFPC section is a singleton, so a repeated id is always
    /// tampering: `insert` keeping the last copy would let a package ship two
    /// views of one section, one for an inspector and one to be lowered.
    #[test]
    fn a_duplicate_section_id_is_rejected() {
        let bytes = container(
            &[
                (SECTION_ABI_INDEX, vec![1, 2]),
                (SECTION_ABI_INDEX, vec![3, 4]),
            ],
            MFPC_MAJOR_VERSION,
            None,
        );
        let err = read_section_table(&bytes).unwrap_err();
        assert_eq!(err, "duplicate MFPC section id 15");
    }

    /// A colossal declared offset or length is refused rather than used.
    ///
    /// **`checked_usize`'s own rejection is not reachable on a 64-bit host**, and
    /// this test deliberately does not claim otherwise. `usize::try_from(u64)`
    /// always succeeds where `usize` is 64 bits wide, so the
    /// `checked_usize`-vs-`as usize` divergence between the two old copies is
    /// observable only on a 32-bit target — where `as usize` would truncate
    /// `0x1_0000_0000` to `0` and every downstream bounds check would then pass
    /// on a length that does not describe the real body.
    ///
    /// What *is* observable here is that the combined guard chain refuses the
    /// value instead of indexing with it: on 64-bit the rejection comes from the
    /// `offset.checked_add(length)` overflow one step later. Both spellings are
    /// therefore kept — `checked_usize` for the 32-bit case this test cannot
    /// reach, `checked_add` for this one.
    #[test]
    fn a_colossal_u64_offset_or_length_is_refused_not_used() {
        for field_offset in [8usize, 16] {
            let mut bytes = ok_container();
            let at = 16 + field_offset;
            bytes[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
            let err = read_section_table(&bytes).unwrap_err();
            // Either guard is an acceptable refusal; silently proceeding is not.
            assert!(
                err.contains("MFPC section")
                    && (err.contains("exceeds the address space")
                        || err.contains("invalid MFPC section length")
                        || err.contains("truncated MFPC section")),
                "a colossal value must be refused, got: {err}"
            );
        }

        // And the narrower, definitely-reachable case: a length that is
        // representable but overruns the container.
        let mut bytes = ok_container();
        let at = 16 + 16;
        bytes[at..at + 8].copy_from_slice(&(u64::from(u32::MAX)).to_le_bytes());
        assert_eq!(
            read_section_table(&bytes).unwrap_err(),
            "truncated MFPC section"
        );
    }

    /// `checked_usize` is the guard the registry's copy replaced with
    /// `as usize`. Its rejection cannot be provoked through
    /// [`read_section_table`] on a 64-bit host (see the test above), so it is
    /// pinned directly instead — otherwise the union-of-guards claim in this
    /// module's doc would rest on an untested line.
    #[test]
    fn checked_usize_refuses_a_value_wider_than_the_address_space() {
        // Exactly representable on 64-bit, so assert the success direction here
        // and the failure direction on a value no `usize` width can hold.
        assert_eq!(
            checked_usize(u64::from(u32::MAX), "MFPC section offset").unwrap(),
            u32::MAX as usize
        );
        #[cfg(target_pointer_width = "32")]
        {
            let err = checked_usize(0x1_0000_0000, "MFPC section offset").unwrap_err();
            assert!(err.contains("exceeds the address space"), "{err}");
        }
    }

    /// The section ids are frozen wire format: a renumbering would silently
    /// mis-decode every already-published package, and nothing in the type
    /// system would catch it. Pinned by literal.
    #[test]
    fn the_section_ids_are_frozen_wire_values() {
        assert_eq!(SECTION_MANIFEST, 1);
        assert_eq!(SECTION_STRING_POOL, 2);
        assert_eq!(SECTION_TYPE_TABLE, 3);
        assert_eq!(SECTION_CONST_POOL, 4);
        assert_eq!(SECTION_IMPORT_TABLE, 5);
        assert_eq!(SECTION_EXPORT_TABLE, 6);
        assert_eq!(SECTION_GLOBAL_TABLE, 7);
        assert_eq!(SECTION_FUNCTION_TABLE, 8);
        assert_eq!(SECTION_NATIVE_LIBRARY_TABLE, 10);
        assert_eq!(SECTION_RESOURCE_TABLE, 11);
        assert_eq!(SECTION_ABI_INDEX, 15);
        assert_eq!(SECTION_BINARY_REPR, 16);
        assert_eq!(SECTION_DOC_TABLE, 17);
        assert_eq!(SECTION_PACKAGE_META, 18);
        assert_eq!(PACKAGE_META_FIELD_DESCRIPTION, 1);
        assert_eq!(MFPC_MAJOR_VERSION, 2);
        assert_eq!(MFPC_MAGIC, b"MFPC");
        // Ids 9, 12, 13, 14 and 19 are deliberately unused here: 12-14 are
        // reserved by the format, 9 and 19 are gaps. Nothing may claim them
        // without a format decision.
        assert_eq!(WIRE_LIBC_UNSPECIFIED, 0);
        assert_eq!(WIRE_LIBC_GLIBC, 1);
        assert_eq!(WIRE_LIBC_MUSL, 2);
        assert_eq!(WIRE_LIB_TYPE_SYSTEM, 0);
        assert_eq!(WIRE_LIB_TYPE_VENDOR, 1);
    }
}
