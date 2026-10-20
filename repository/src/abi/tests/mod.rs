use super::*;
// plan-126-C: both are now this file's *tests*-only vocabulary — the
// production code that used them (`read_section_table`) moved to
// `mfb_wire::mfpc`. The tests stay: they assert the registry's
// *best-effort posture* around the shared reader, which is this crate's
// behaviour and not the reader's.
use mfb_wire::mfpc::{MAX_MFPC_SECTIONS, MFPC_MAGIC};

fn put_u16(dst: &mut Vec<u8>, value: u16) {
    dst.extend_from_slice(&value.to_le_bytes());
}
fn put_u32(dst: &mut Vec<u8>, value: u32) {
    dst.extend_from_slice(&value.to_le_bytes());
}
fn put_u64(dst: &mut Vec<u8>, value: u64) {
    dst.extend_from_slice(&value.to_le_bytes());
}

fn string_pool(strings: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    put_u32(&mut bytes, strings.len() as u32);
    for value in strings {
        put_u32(&mut bytes, value.len() as u32);
        bytes.extend_from_slice(value.as_bytes());
    }
    bytes
}

fn abi_section(exports: &[(u32, [u8; 32])]) -> Vec<u8> {
    let mut bytes = Vec::new();
    put_u16(&mut bytes, ABI_FORMAT_VERSION);
    put_u16(&mut bytes, 0);
    put_u32(&mut bytes, exports.len() as u32);
    for (name_index, hash) in exports {
        put_u32(&mut bytes, *name_index);
        put_u16(&mut bytes, 1); // kind (ignored by the reader)
        bytes.extend_from_slice(hash);
    }
    put_u32(&mut bytes, 0); // zero dep edges
    bytes
}

/// Assemble a minimal MFPC container carrying just the two sections the
/// registry reads.
fn container(sections: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MFPC_MAGIC);
    put_u16(&mut bytes, 2); // major
    put_u16(&mut bytes, 0);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, sections.len() as u32);
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

/// Like [`container`], but with a caller-chosen MFPC major version, so a
/// test can build the one thing the registry used to accept and no longer
/// does.
fn container_with_major(sections: &[(u16, Vec<u8>)], major: u16) -> Vec<u8> {
    let mut bytes = container(sections);
    bytes[4..6].copy_from_slice(&major.to_le_bytes());
    bytes
}

/// **plan-126-C Phase 3 — the guard this crate was missing.**
///
/// Before the shared reader, this crate's `read_section_table` never checked
/// the MFPC major version: only the compiler's copy did. A v1 or v3
/// container's payload layout differs throughout, so decoding one as v2
/// mis-reads rather than failing.
///
/// The failure mode is **silent**, which is why this asserts the error
/// directly instead of watching a publish succeed: `abi_index_json` maps
/// *every* parse error to `{}`, so a missing guard does not reject anything
/// — it records empty metadata for a package the compiler would refuse to
/// load, and nothing announces it.
#[test]
fn a_wrong_mfpc_major_version_is_now_rejected_by_this_crate_too() {
    let sections = [
        (SECTION_STRING_POOL, string_pool(&["greet"])),
        // string index 0 is "greet" in the pool above.
        (SECTION_ABI_INDEX, abi_section(&[(0, [0x11; 32])])),
    ];

    // v2 is what the compiler emits: parses, and yields the real index.
    let good = container(&sections);
    let parsed = parse_abi_index(&good).expect("a v2 container parses");
    assert_eq!(parsed.len(), 1);
    assert!(abi_index_json(&good).get("greet").is_some());

    // v3 and v1 are both refused, explicitly and by name.
    for major in [1u16, 3] {
        let bytes = container_with_major(&sections, major);
        let err = parse_abi_index(&bytes).unwrap_err();
        assert!(
            err.contains(&format!("unsupported MFPC major version {major}")),
            "major {major}: {err}"
        );

        // And this is the insidious part, asserted rather than assumed: the
        // error is swallowed into an empty object, so a publish carrying
        // this payload would have succeeded with empty `abiIndex` either
        // way. The guard changes *which* packages land with empty metadata,
        // not whether they land.
        assert_eq!(abi_index_json(&bytes), serde_json::json!({}));
    }
}

/// The registry's **best-effort posture** toward optional sections survives
/// the reader swap: a payload that is not an MFPC container at all must
/// still mean "no such section", never an error, for the parsers whose
/// absence case is normal. A package with no section 10 or 18 is the
/// ordinary case, not a finding.
#[test]
fn a_non_container_payload_still_means_no_such_section_not_an_error() {
    for payload in [
        &b""[..],
        &b"not an MFPC container"[..],
        // A v3 container: structurally a container, wrong version. The
        // best-effort parsers must treat it as "nothing here" too.
        &container_with_major(&[(SECTION_STRING_POOL, string_pool(&["x"]))], 3)[..],
    ] {
        assert_eq!(
            parse_vendor_blobs(payload).expect("vendor blobs are best-effort"),
            Vec::new(),
        );
        assert_eq!(
            parse_package_description(payload).expect("description is best-effort"),
            None,
        );
        assert_eq!(
            parse_manifest_metadata(payload).expect("manifest metadata is best-effort"),
            None,
        );
    }
}

#[test]
fn parses_symbol_hash_map_from_container() {
    let payload = container(&[
        (SECTION_STRING_POOL, string_pool(&["greet", "farewell"])),
        (
            SECTION_ABI_INDEX,
            abi_section(&[(0, [0xaa; 32]), (1, [0xbb; 32])]),
        ),
    ]);
    let map = parse_abi_index(&payload).unwrap();
    assert_eq!(map.get("greet").unwrap(), &hex::encode([0xaa; 32]));
    assert_eq!(map.get("farewell").unwrap(), &hex::encode([0xbb; 32]));

    let json = abi_index_json(&payload);
    assert_eq!(json["greet"], hex::encode([0xaa; 32]));
}

#[test]
fn non_container_payloads_are_an_empty_index() {
    assert!(parse_abi_index(b"not a container").is_err());
    assert_eq!(abi_index_json(b"MFPCtestpayload"), serde_json::json!({}));
}

/// Build a section-10 `NATIVE_LIBRARY_TABLE`. Each locator is
/// `(os_id, arch_id, libc, lib_type, source_id, hash)`; `hash` is present iff
/// `lib_type` is vendor (1).
fn native_library_table(
    entries: &[(u32, &[(u32, u32, u8, u8, u32, Option<[u8; 32]>)])],
) -> Vec<u8> {
    let mut bytes = Vec::new();
    put_u32(&mut bytes, entries.len() as u32);
    for (logical, locators) in entries {
        put_u32(&mut bytes, *logical);
        put_u32(&mut bytes, locators.len() as u32);
        for (os, arch, libc, lib_type, source, hash) in *locators {
            put_u32(&mut bytes, *os);
            put_u32(&mut bytes, *arch);
            bytes.push(*libc);
            bytes.push(*lib_type);
            put_u32(&mut bytes, *source);
            if let Some(hash) = hash {
                bytes.extend_from_slice(hash);
            }
        }
    }
    bytes
}

#[test]
fn parses_vendor_hashes_from_section_ten() {
    // strings: 0=sqlite3, 1=linux, 2=x86_64, 3=libsqlite3.so, 4=libc.so.6
    let strings = string_pool(&["sqlite3", "linux", "x86_64", "libsqlite3.so", "libc.so.6"]);
    let table = native_library_table(&[(
        0,
        &[
            // vendor locator carrying a hash
            (1, 2, 1, 1, 3, Some([0x11; 32])),
            // a system locator (lib_type 0) with no hash — must be skipped
            (1, 2, 1, 0, 4, None),
        ],
    )]);
    let payload = container(&[
        (SECTION_STRING_POOL, strings),
        (SECTION_NATIVE_LIBRARY_TABLE, table),
    ]);
    let refs = parse_vendor_blobs(&payload).unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].logical, "sqlite3");
    assert_eq!(refs[0].source, "libsqlite3.so");
    assert_eq!(refs[0].hash, hex::encode([0x11; 32]));
}

/// plan-61-A Phase 2: the platform axis the parser used to `offset += 4`
/// straight past is resolved through the same string pool that already
/// resolved `logical` and `source`.
#[test]
fn resolves_the_platform_triple_for_each_locator() {
    // strings: 0=snd, 1=linux, 2=x86_64, 3=aarch64, 4=a.a, 5=b.a, 6=macos, 7=""
    let strings = string_pool(&[
        "snd", "linux", "x86_64", "aarch64", "a.a", "b.a", "macos", "",
    ]);
    let table = native_library_table(&[(
        0,
        &[
            // linux/x86_64, glibc
            (1, 2, WIRE_LIBC_GLIBC, 1, 4, Some([0x11; 32])),
            // linux/aarch64, musl
            (1, 3, WIRE_LIBC_MUSL, 1, 5, Some([0x22; 32])),
        ],
    )]);
    let payload = container(&[
        (SECTION_STRING_POOL, strings),
        (SECTION_NATIVE_LIBRARY_TABLE, table),
    ]);
    let refs = parse_vendor_blobs(&payload).unwrap();
    assert_eq!(refs.len(), 2);

    assert_eq!(refs[0].os, "linux");
    assert_eq!(refs[0].arch.as_deref(), Some("x86_64"));
    assert_eq!(refs[0].libc.as_deref(), Some("glibc"));
    assert_eq!(refs[0].lib_type, "vendor");
    assert_eq!(refs[0].source, "a.a");

    assert_eq!(refs[1].os, "linux");
    assert_eq!(refs[1].arch.as_deref(), Some("aarch64"));
    assert_eq!(refs[1].libc.as_deref(), Some("musl"));
}

/// An empty `arch` is the any-arch wildcard — the locator matches every
/// architecture on its OS — and must stay distinguishable from a concrete
/// arch. `bindings/libsnd`'s macOS locator (no `arch` key) is the shape.
#[test]
fn an_empty_arch_is_the_any_arch_wildcard_not_a_concrete_arch() {
    // strings: 0=snd, 1=macos, 2="", 3=libsnd.dylib, 4=x86_64, 5=libsnd.a
    let strings = string_pool(&["snd", "macos", "", "libsnd.dylib", "x86_64", "libsnd.a"]);
    let table = native_library_table(&[(
        0,
        &[
            (1, 2, WIRE_LIBC_UNSPECIFIED, 1, 3, Some([0x33; 32])),
            (1, 4, WIRE_LIBC_UNSPECIFIED, 1, 5, Some([0x44; 32])),
        ],
    )]);
    let payload = container(&[
        (SECTION_STRING_POOL, strings),
        (SECTION_NATIVE_LIBRARY_TABLE, table),
    ]);
    let refs = parse_vendor_blobs(&payload).unwrap();
    assert_eq!(refs[0].arch, None, "an empty arch is the wildcard");
    assert_eq!(refs[1].arch.as_deref(), Some("x86_64"));
    assert_ne!(
        refs[0].arch, refs[1].arch,
        "the wildcard must not collapse into a concrete arch"
    );
    // No libc constraint is `None`, never the string "unspecified".
    assert_eq!(refs[0].libc, None);
}

/// Section 10 rides inside the signed payload, so a libc byte outside the
/// vocabulary means a broken or tampered package. Reporting it as "no libc
/// constraint" would let a tampered locator silently widen its own match.
#[test]
fn an_unknown_libc_discriminant_is_rejected() {
    let strings = string_pool(&["snd", "linux", "x86_64", "libsnd.a"]);
    let table = native_library_table(&[(0, &[(1, 2, 99, 1, 3, Some([0x55; 32]))])]);
    let payload = container(&[
        (SECTION_STRING_POOL, strings),
        (SECTION_NATIVE_LIBRARY_TABLE, table),
    ]);
    assert!(parse_vendor_blobs(&payload)
        .unwrap_err()
        .contains("unknown libc discriminant 99"));
}

/// Gotcha 2 (plan-61-A §3), at the parser layer: two locators with distinct
/// `source` filenames but byte-identical contents share one hash. That is
/// legal — `PROJECT_JSON_LIBRARY_SOURCE_CONFLICT` only forbids two vendor
/// locators sharing a *source* — and it is the only shape reachable from a
/// valid manifest that makes dedupe-by-hash lose a platform.
#[test]
fn two_locators_sharing_one_hash_stay_two_locators() {
    // strings: 0=snd, 1=linux, 2=x86_64, 3=snd-glibc.a, 4=snd-musl.a
    let strings = string_pool(&["snd", "linux", "x86_64", "snd-glibc.a", "snd-musl.a"]);
    let shared = [0x77; 32];
    let table = native_library_table(&[(
        0,
        &[
            (1, 2, WIRE_LIBC_GLIBC, 1, 3, Some(shared)),
            (1, 2, WIRE_LIBC_MUSL, 1, 4, Some(shared)),
        ],
    )]);
    let payload = container(&[
        (SECTION_STRING_POOL, strings),
        (SECTION_NATIVE_LIBRARY_TABLE, table),
    ]);
    let refs = parse_vendor_blobs(&payload).unwrap();
    assert_eq!(
        refs.len(),
        2,
        "one blob shipped under two names is still two supported platforms"
    );
    assert_eq!(refs[0].hash, refs[1].hash);
    assert_eq!(refs[0].libc.as_deref(), Some("glibc"));
    assert_eq!(refs[1].libc.as_deref(), Some("musl"));
}

/// plan-61-A §4: `author`/`url` come out of MANIFEST section 1, resolved
/// through the string pool — the copy the signature covers.
#[test]
fn reads_author_and_url_from_the_manifest_section() {
    let mut manifest = Vec::new();
    for _ in 0..6 {
        put_u32(&mut manifest, 0);
    }
    put_u32(&mut manifest, 1); // author -> "alice"
    put_u32(&mut manifest, 2); // url    -> "https://example.invalid"
    for _ in 0..6 {
        put_u16(&mut manifest, 0);
    }
    for _ in 0..5 {
        put_u32(&mut manifest, 0);
    }
    let payload = container(&[
        (SECTION_MANIFEST, manifest),
        (
            SECTION_STRING_POOL,
            string_pool(&["", "alice", "https://example.invalid"]),
        ),
    ]);
    let meta = parse_manifest_metadata(&payload).unwrap().unwrap();
    assert_eq!(meta.author, "alice");
    assert_eq!(meta.url, "https://example.invalid");
}

/// A payload with no section 1 carries no metadata — best-effort `None`,
/// matching how a missing section 10 means "vendors nothing". A section
/// that is present but unreadable is an error, not silent emptiness.
#[test]
fn manifest_metadata_is_absent_or_malformed_but_never_silently_wrong() {
    let payload = container(&[(SECTION_STRING_POOL, string_pool(&["x"]))]);
    assert_eq!(parse_manifest_metadata(&payload).unwrap(), None);
    assert_eq!(parse_manifest_metadata(b"not a container").unwrap(), None);

    // Section 1 present but too short to hold the url id.
    let payload = container(&[
        (SECTION_MANIFEST, vec![0u8; 26]),
        (SECTION_STRING_POOL, string_pool(&["x"])),
    ]);
    assert!(parse_manifest_metadata(&payload).is_err());

    // Section 1 present, string pool missing.
    let payload = container(&[(SECTION_MANIFEST, vec![0u8; 64])]);
    assert!(parse_manifest_metadata(&payload)
        .unwrap_err()
        .contains("missing the string pool"));

    // An author id past the end of the pool is an error, not an empty name.
    let mut manifest = vec![0u8; 24];
    put_u32(&mut manifest, 99); // author -> out of range
    put_u32(&mut manifest, 0);
    manifest.resize(64, 0);
    let payload = container(&[
        (SECTION_MANIFEST, manifest),
        (SECTION_STRING_POOL, string_pool(&["x"])),
    ]);
    assert!(parse_manifest_metadata(&payload).is_err());
}

/// plan-61-D Phase 2, repository side: section 18 is read here too, so the
/// registry can surface a description without re-deriving the format.
#[test]
fn reads_the_description_from_section_eighteen() {
    let mut section = Vec::new();
    put_u32(&mut section, 1);
    put_u16(&mut section, 1); // fieldId = description
    let text = "A demo package.";
    put_u32(&mut section, text.len() as u32);
    section.extend_from_slice(text.as_bytes());
    let payload = container(&[(SECTION_PACKAGE_META, section)]);
    assert_eq!(
        parse_package_description(&payload).unwrap().as_deref(),
        Some("A demo package."),
    );

    // No section 18 at all — every package built before plan-61-D, and
    // every package that declares no description.
    let payload = container(&[(SECTION_STRING_POOL, string_pool(&["x"]))]);
    assert_eq!(parse_package_description(&payload).unwrap(), None);
    assert_eq!(parse_package_description(b"not a container").unwrap(), None);

    // Unknown field ids are skipped rather than rejected.
    let mut section = Vec::new();
    put_u32(&mut section, 2);
    put_u16(&mut section, 777);
    put_u32(&mut section, 3);
    section.extend_from_slice(b"abc");
    put_u16(&mut section, 1);
    put_u32(&mut section, 4);
    section.extend_from_slice(b"real");
    let payload = container(&[(SECTION_PACKAGE_META, section)]);
    assert_eq!(
        parse_package_description(&payload).unwrap().as_deref(),
        Some("real"),
    );

    // The cap is re-checked here: this reader sees payloads the compiler's
    // manifest validation never touched.
    let mut section = Vec::new();
    let oversized = "x".repeat(MAX_DESCRIPTION_BYTES + 1);
    put_u32(&mut section, 1);
    put_u16(&mut section, 1);
    put_u32(&mut section, oversized.len() as u32);
    section.extend_from_slice(oversized.as_bytes());
    let payload = container(&[(SECTION_PACKAGE_META, section)]);
    assert!(parse_package_description(&payload)
        .unwrap_err()
        .contains("exceeds the 4096 byte limit"));
}

#[test]
fn no_section_ten_means_no_vendor_blobs() {
    // A container with only a string pool vendors nothing.
    let payload = container(&[(SECTION_STRING_POOL, string_pool(&["x"]))]);
    assert!(parse_vendor_blobs(&payload).unwrap().is_empty());
    // A non-container payload is best-effort empty, not an error (§ abi weld).
    assert!(parse_vendor_blobs(b"MFPCnope").unwrap().is_empty());
}

#[test]
fn truncated_vendor_hash_is_rejected() {
    let strings = string_pool(&["sqlite3", "linux", "x86_64", "libsqlite3.so"]);
    // A vendor locator whose 32-byte hash is missing from the bytes.
    let mut table = Vec::new();
    put_u32(&mut table, 1); // one entry
    put_u32(&mut table, 0); // logical = "sqlite3"
    put_u32(&mut table, 1); // one locator
    put_u32(&mut table, 1); // os
    put_u32(&mut table, 2); // arch
    table.push(1); // libc
    table.push(1); // lib_type = vendor
    put_u32(&mut table, 3); // source — but no hash follows
    let payload = container(&[
        (SECTION_STRING_POOL, strings),
        (SECTION_NATIVE_LIBRARY_TABLE, table),
    ]);
    assert!(parse_vendor_blobs(&payload)
        .unwrap_err()
        .contains("truncated native library locator hash"));
}

/// An oversized section-10 table is rejected on its declared counts, before
/// any locator is parsed or any blob probed (bug-275).
///
/// The counts are raw `u32`s off an attacker-controlled payload and the
/// caller probes the blob store once per locator, so an unbounded table turns
/// one `/validate` into ~1M backend operations. Both shapes are covered: one
/// entry declaring a huge locator count, and many entries each declaring a
/// modest one — the second is why the locator bound has to be a running total
/// rather than per-entry.
#[test]
fn oversized_vendor_table_is_rejected_before_probing() {
    let strings = string_pool(&["sqlite3", "linux", "x86_64", "libsqlite3.so"]);

    // Shape 1: a single entry claiming far more locators than the cap. Note
    // the bytes for those locators are never supplied — rejection has to come
    // from the count itself, not from running off the end of the payload.
    let mut table = Vec::new();
    put_u32(&mut table, 1); // one entry
    put_u32(&mut table, 0); // logical = "sqlite3"
    put_u32(&mut table, 1_000_000); // locator count
    let payload = container(&[
        (SECTION_STRING_POOL, strings.clone()),
        (SECTION_NATIVE_LIBRARY_TABLE, table),
    ]);
    let err = parse_vendor_blobs(&payload).unwrap_err();
    assert!(
        err.contains("locators"),
        "expected a locator-cap rejection, got: {err}"
    );

    // Shape 2: an absurd entry count, rejected before the entry loop starts.
    let mut table = Vec::new();
    put_u32(&mut table, 5_000_000); // entry count
    let payload = container(&[
        (SECTION_STRING_POOL, strings.clone()),
        (SECTION_NATIVE_LIBRARY_TABLE, table),
    ]);
    let err = parse_vendor_blobs(&payload).unwrap_err();
    assert!(
        err.contains("entries"),
        "expected an entry-cap rejection, got: {err}"
    );

    // Shape 3: many entries, each with a modest locator count that no
    // per-entry check would object to. Only a running total catches this.
    //
    // The bytes have to be real: the parser walks each entry's locators
    // before reaching the next entry, so the total cannot accumulate over
    // declared-but-absent locators. `system` locators (lib_type 0) carry no
    // hash, which keeps the payload small.
    let mut table = Vec::new();
    let entries = 64u32;
    let per_entry = 100u32; // 6400 total, well past MAX_VENDOR_LOCATORS
    put_u32(&mut table, entries);
    for _ in 0..entries {
        put_u32(&mut table, 0); // logical = "sqlite3"
        put_u32(&mut table, per_entry);
        for _ in 0..per_entry {
            put_u32(&mut table, 1); // os
            put_u32(&mut table, 2); // arch
            table.push(1); // libc
            table.push(0); // lib_type = system (no hash follows)
            put_u32(&mut table, 3); // source
        }
    }
    let payload = container(&[
        (SECTION_STRING_POOL, strings),
        (SECTION_NATIVE_LIBRARY_TABLE, table),
    ]);
    let err = parse_vendor_blobs(&payload).unwrap_err();
    assert!(
        err.contains("locators"),
        "expected the running total to reject, got: {err}"
    );
}

/// The string-pool pre-allocation is bounded by what the section could hold,
/// not by its byte length (bug-276 R8).
///
/// A `String` is 24 bytes and the smallest entry is its 4-byte length prefix,
/// so reserving `count.min(bytes.len())` over-reserved by up to 24x — a
/// ~48 MiB section declaring a huge count forced a ~1.15 GiB transient on
/// every /validate and /publish. The declared count here is far larger than
/// the section can hold; the parse must reject on truncation without first
/// trying to reserve for `count` strings.
#[test]
fn string_pool_does_not_preallocate_beyond_what_the_section_can_hold() {
    // bug-578 moved the *outer* bound to an absolute entry ceiling, which a
    // `u32::MAX` count now trips first. The bug-276 R8 invariant is about
    // the counts the ceiling still admits, so probe it at the ceiling: the
    // section can hold 16 entries, the header claims 1,048,576, and the
    // reservation must follow the section, not the claim.
    let mut pool = Vec::new();
    put_u32(&mut pool, MAX_STRING_POOL_ENTRIES as u32); // declared count
    pool.extend_from_slice(&[0u8; 64]); // but only 64 bytes of entries
    let err = read_string_pool(&pool).unwrap_err();
    assert!(
        err.contains("truncated"),
        "expected a truncation rejection, got: {err}"
    );

    // A count past the ceiling is refused earlier still, on the count alone.
    let mut pool = Vec::new();
    put_u32(&mut pool, u32::MAX);
    pool.extend_from_slice(&[0u8; 64]);
    let err = read_string_pool(&pool).unwrap_err();
    assert!(
        err.contains("entries"),
        "expected an entry-cap rejection, got: {err}"
    );
}

/// A table within the caps still parses — the bound must not reject packages
/// that legitimately vendor a library for several platforms.
#[test]
fn normal_sized_vendor_table_still_parses() {
    let strings = string_pool(&["sqlite3", "linux", "x86_64", "libsqlite3.so"]);
    let mut table = Vec::new();
    put_u32(&mut table, 1); // one entry
    put_u32(&mut table, 0); // logical = "sqlite3"
    put_u32(&mut table, 2); // two platform locators
    for _ in 0..2 {
        put_u32(&mut table, 1); // os
        put_u32(&mut table, 2); // arch
        table.push(1); // libc
        table.push(1); // lib_type = vendor
        put_u32(&mut table, 3); // source
        table.extend_from_slice(&[0xcd; NATIVE_LIBRARY_HASH_LEN]);
    }
    let payload = container(&[
        (SECTION_STRING_POOL, strings),
        (SECTION_NATIVE_LIBRARY_TABLE, table),
    ]);
    let refs = parse_vendor_blobs(&payload).expect("a normal table parses");
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].logical, "sqlite3");
    assert_eq!(refs[0].hash, hex::encode([0xcd; NATIVE_LIBRARY_HASH_LEN]));
}

#[test]
fn missing_string_pool_or_abi_section_is_an_error() {
    // Only the ABI section present: the string pool is missing.
    let only_abi = container(&[(SECTION_ABI_INDEX, abi_section(&[]))]);
    assert!(parse_abi_index(&only_abi)
        .unwrap_err()
        .contains("string pool"));
    // Only the string pool present: the ABI index is missing.
    let only_pool = container(&[(SECTION_STRING_POOL, string_pool(&["x"]))]);
    assert!(parse_abi_index(&only_pool)
        .unwrap_err()
        .contains("ABI index"));
}

#[test]
fn duplicate_section_id_is_rejected() {
    // Two sections share the string-pool id.
    let payload = container(&[
        (SECTION_STRING_POOL, string_pool(&["a"])),
        (SECTION_STRING_POOL, string_pool(&["b"])),
    ]);
    assert!(parse_abi_index(&payload)
        .unwrap_err()
        .contains("duplicate MFPC section"));
}

#[test]
fn truncated_section_table_and_section_are_rejected() {
    // A header claiming more sections than the bytes can hold.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MFPC_MAGIC);
    put_u16(&mut bytes, 2);
    put_u16(&mut bytes, 0);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, 100); // 100 sections, but no table follows
    assert!(parse_abi_index(&bytes)
        .unwrap_err()
        .contains("truncated MFPC section table"));

    // A section entry whose offset+length runs past the buffer.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MFPC_MAGIC);
    put_u16(&mut bytes, 2);
    put_u16(&mut bytes, 0);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, 1); // one section
    put_u16(&mut bytes, SECTION_STRING_POOL);
    put_u16(&mut bytes, 0);
    put_u32(&mut bytes, 0);
    put_u64(&mut bytes, 40); // offset well past the buffer
    put_u64(&mut bytes, 10); // length
    assert!(parse_abi_index(&bytes)
        .unwrap_err()
        .contains("truncated MFPC section"));
}

#[test]
fn unsupported_abi_version_and_out_of_range_name_are_rejected() {
    // ABI section with an unsupported format version.
    let mut abi = Vec::new();
    put_u16(&mut abi, 99); // version
    put_u16(&mut abi, 0);
    put_u32(&mut abi, 0);
    let payload = container(&[
        (SECTION_STRING_POOL, string_pool(&["a"])),
        (SECTION_ABI_INDEX, abi),
    ]);
    assert!(parse_abi_index(&payload)
        .unwrap_err()
        .contains("unsupported ABI index format version"));

    // ABI export naming a string index that does not exist.
    let payload = container(&[
        (SECTION_STRING_POOL, string_pool(&["a"])), // one string (index 0)
        (SECTION_ABI_INDEX, abi_section(&[(5, [0u8; 32])])), // names index 5
    ]);
    assert!(parse_abi_index(&payload)
        .unwrap_err()
        .contains("out-of-range string"));
}

#[test]
fn truncated_string_pool_entry_is_rejected() {
    // A string pool claiming a longer entry than the bytes provide.
    let mut pool = Vec::new();
    put_u32(&mut pool, 1); // count = 1
    put_u32(&mut pool, 50); // length = 50, but no bytes follow
    let payload = container(&[
        (SECTION_STRING_POOL, pool),
        (SECTION_ABI_INDEX, abi_section(&[])),
    ]);
    assert!(parse_abi_index(&payload)
        .unwrap_err()
        .contains("truncated string pool entry"));
}

/// The `read_u64` row is gone with the helper (plan-126-C): its only
/// production caller was this file's `read_section_table`, now
/// `mfb_wire::mfpc`'s, where `bytes::checked_u64_at` carries its own
/// truncation and wrap-offset tests.
#[test]
fn read_integer_helpers_report_truncation() {
    assert!(read_u16(&[0u8], 0).is_err());
    assert!(read_u32(&[0u8; 2], 0).is_err());
    assert_eq!(read_u16(&[2, 0], 0).unwrap(), 2);
    assert_eq!(read_u32(&[2, 0, 0, 0], 0).unwrap(), 2);
}

// ---------------------------------------------------------------------
// bug-578: format-level ceilings on section, string-pool, ABI-export and
// package-meta counts.
//
// Measured on HEAD before the fix (`python3` census of every committed
// compiler-produced package, `packages/*/*.mfp`):
//
//   package      sections  pool entries  pool bytes  abi exports  meta fields
//   cli               11            45         423            8            1
//   json_schema       12           628       9,175           24            1
//   jwt               12           914      12,763           48            1
//   libsnd            14            56         669           12            1
//   mustache          12           305       4,112           12            1
//   sqlite3           14            64         767           28            1
//   yaml              12           356       4,600           11            1
//
// The ceilings below sit three to four orders of magnitude above those
// real maxima; see the constant docs for each derivation.
// ---------------------------------------------------------------------

/// A pool whose entries are all zero-length: `count` real 4-byte entries.
fn empty_entry_pool(count: u32) -> Vec<u8> {
    let mut pool = Vec::with_capacity(4 + count as usize * 4);
    put_u32(&mut pool, count);
    for _ in 0..count {
        put_u32(&mut pool, 0);
    }
    pool
}

/// The headline shape (bug-578): the declared count is backed by real
/// bytes, so truncation never fires and the pre-fix parser built one
/// `String` per entry.
///
/// A `String` is 24 bytes on a 64-bit target while the smallest entry is
/// its 4-byte length prefix, so a ~48 MiB pool of empty entries expands to
/// a ~288 MiB `Vec<String>` on a 512 MiB server. bug-276 R8 capped the
/// *pre-allocation* at `bytes.len() / 4`, which is exactly the number of
/// entries the attacker supplies here — it never bounded the pool in
/// absolute terms. Only an entry ceiling does.
#[test]
fn string_pool_rejects_excessive_entry_count() {
    let pool = empty_entry_pool(MAX_STRING_POOL_ENTRIES as u32 + 1);
    let err = read_string_pool(&pool).unwrap_err();
    assert!(
        err.contains("string pool") && err.contains("entries"),
        "expected an entry-cap rejection, got: {err}"
    );
}

/// The same ceiling has to hold through every public entry point, because
/// `validate_package_request` reaches the pool through all of them.
#[test]
fn every_consumer_rejects_an_oversized_string_pool() {
    let pool = empty_entry_pool(MAX_STRING_POOL_ENTRIES as u32 + 1);
    let payload = container(&[
        (SECTION_MANIFEST, vec![0u8; 64]),
        (SECTION_STRING_POOL, pool.clone()),
        (SECTION_ABI_INDEX, abi_section(&[])),
        (
            SECTION_NATIVE_LIBRARY_TABLE,
            native_library_table(&[(0, &[])]),
        ),
    ]);
    assert!(parse_abi_index(&payload).unwrap_err().contains("entries"));
    assert!(parse_vendor_blobs(&payload)
        .unwrap_err()
        .contains("entries"));
    assert!(parse_manifest_metadata(&payload)
        .unwrap_err()
        .contains("entries"));
    // `abi_index_json` is best-effort, so the rejection surfaces as the
    // empty object rather than an error — but the pool is still never built.
    assert_eq!(abi_index_json(&payload), serde_json::json!({}));
}

/// Aggregate decoded string bytes are capped too. Byte-for-byte the pool is
/// no worse than 1:1 with the request body, but the cap keeps the pool's
/// share of a 64 MiB body bounded independently of the entry ceiling.
#[test]
fn string_pool_rejects_excessive_aggregate_bytes() {
    let chunk = 1024 * 1024;
    let entries = MAX_STRING_POOL_BYTES / chunk + 1;
    let mut pool = Vec::new();
    put_u32(&mut pool, entries as u32);
    for _ in 0..entries {
        put_u32(&mut pool, chunk as u32);
        pool.resize(pool.len() + chunk, b'x');
    }
    let err = read_string_pool(&pool).unwrap_err();
    assert!(
        err.contains("string pool") && err.contains("bytes"),
        "expected an aggregate-byte rejection, got: {err}"
    );
}

/// The section table was bounded only by the body: a 64 MiB payload can
/// declare ~2.7 million 24-byte table entries, every one of them walked and
/// inserted into a `BTreeMap` before anything else is checked.
#[test]
fn section_table_rejects_excessive_section_count() {
    let count = MAX_MFPC_SECTIONS + 1;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MFPC_MAGIC);
    put_u16(&mut bytes, 2);
    put_u16(&mut bytes, 0);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, count as u32);
    // Real, distinct, zero-length entries: rejection must come from the
    // count, not from truncation or the duplicate-id check.
    for index in 0..count {
        put_u16(&mut bytes, index as u16 + 1);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, 0);
        put_u64(&mut bytes, 0);
        put_u64(&mut bytes, 0);
    }
    let err = read_section_table(&bytes).unwrap_err();
    assert!(
        err.contains("sections"),
        "expected a section-count rejection, got: {err}"
    );
}

/// `read_abi_exports` had no export ceiling: 38 wire bytes per export means
/// a 64 MiB payload drives ~1.7 million iterations, each one a `hex::encode`
/// allocation, on every `/validate` and `/publish`.
#[test]
fn abi_index_rejects_excessive_export_count() {
    let count = MAX_ABI_EXPORTS + 1;
    let mut abi = Vec::with_capacity(8 + count * 38);
    put_u16(&mut abi, ABI_FORMAT_VERSION);
    put_u16(&mut abi, 0);
    put_u32(&mut abi, count as u32);
    for _ in 0..count {
        put_u32(&mut abi, 0); // every export names string 0
        put_u16(&mut abi, 1);
        abi.extend_from_slice(&[0u8; ABI_HASH_LEN]);
    }
    let payload = container(&[
        (SECTION_STRING_POOL, string_pool(&["greet"])),
        (SECTION_ABI_INDEX, abi),
    ]);
    let err = parse_abi_index(&payload).unwrap_err();
    assert!(
        err.contains("exports"),
        "expected an export-cap rejection, got: {err}"
    );
}

/// Section 18's field loop was bounded only by truncation at 6 bytes per
/// field — ~10 million iterations for a 64 MiB payload.
#[test]
fn package_meta_rejects_excessive_field_count() {
    let count = MAX_PACKAGE_META_FIELDS + 1;
    let mut section = Vec::new();
    put_u32(&mut section, count as u32);
    for _ in 0..count {
        put_u16(&mut section, 777); // unknown field id: skipped, not stored
        put_u32(&mut section, 0);
    }
    let payload = container(&[(SECTION_PACKAGE_META, section)]);
    let err = parse_package_description(&payload).unwrap_err();
    assert!(
        err.contains("fields"),
        "expected a field-cap rejection, got: {err}"
    );
}

/// POSITIVE PIN 1 — a real compiler-produced `.mfp` still parses to exactly
/// the same ABI index, vendor locators and metadata.
///
/// `tests/fixtures/libsnd.mfp` is the widest shape the compiler emits: all
/// 14 sections, a section-10 table with seven real `vendor` locators across
/// three architectures and both libc flavours, a section-18 description, and
/// a 12-entry ABI index. If a ceiling ever starts refusing legitimate
/// packages, this is the test that fails.
///
/// It is a COMMITTED copy, deliberately. The pin originally read the build
/// output at `packages/libsnd/libsnd.mfp`, which `.gitignore:34`
/// (`packages/**/*.mfp`) excludes — so the one test proving the parser
/// ceilings do not refuse real packages passed only on a machine that had
/// already built libsnd, and failed on every fresh clone and in CI. A pin
/// whose subject is untracked is not a pin. Regenerate with
/// `mfb build -pkg packages/libsnd` and copy it here if the emitted shape
/// ever changes; the assertions below are exact, so a stale copy fails loudly
/// rather than silently checking less.
#[test]
fn a_real_compiler_produced_package_still_parses() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/libsnd.mfp");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    let package = crate::package::parse_mfp_package(&bytes).expect("a real .mfp parses");

    let sections = read_section_table(&package.payload).expect("section table");
    assert_eq!(
        sections.len(),
        14,
        "libsnd emits every section the writer has"
    );

    let pool = read_string_pool(sections.get(&SECTION_STRING_POOL).unwrap()).unwrap();
    assert_eq!(pool.len(), 56);

    let abi = parse_abi_index(&package.payload).expect("abi index");
    assert_eq!(abi.len(), 12);
    assert!(abi.contains_key("closeSound"));
    assert!(abi.contains_key("getFormats"));

    let vendors = parse_vendor_blobs(&package.payload).expect("vendor locators");
    assert_eq!(vendors.len(), 7);
    assert_eq!(vendors[0].logical, "libsnd");
    assert_eq!(vendors[0].os, "macos");
    assert_eq!(vendors[0].arch.as_deref(), Some("aarch64"));
    assert_eq!(vendors[0].libc, None);
    assert_eq!(vendors[0].source, "libsndfile.1.0.37.dylib");
    assert_eq!(
        vendors[0].hash,
        "7297000499a2f5e146ad4c369db90f00c652ccc3b5cafafce9d388e43aa3617e"
    );
    assert_eq!(vendors[3].libc.as_deref(), Some("glibc"));
    assert_eq!(vendors[4].libc.as_deref(), Some("musl"));

    let meta = parse_manifest_metadata(&package.payload).expect("manifest");
    assert_eq!(
        meta,
        Some(ManifestMetadata {
            author: String::new(),
            url: String::new(),
        })
    );

    let description = parse_package_description(&package.payload)
        .expect("description")
        .expect("libsnd declares a description");
    assert!(description.starts_with("A reusable native binding package for libsndfile"));
}

/// POSITIVE PIN 2 — a payload sitting exactly *on* every ceiling is still
/// accepted. A cap that is off by one in the rejecting direction refuses a
/// legitimate package, which is the failure mode these ceilings must not
/// introduce.
#[test]
fn payloads_exactly_at_every_ceiling_are_still_accepted() {
    // String pool: exactly MAX_STRING_POOL_ENTRIES entries.
    let pool = read_string_pool(&empty_entry_pool(MAX_STRING_POOL_ENTRIES as u32))
        .expect("a pool at the entry ceiling is valid");
    assert_eq!(pool.len(), MAX_STRING_POOL_ENTRIES);

    // String pool: exactly MAX_STRING_POOL_BYTES of aggregate string bytes.
    let mut wide = Vec::new();
    put_u32(&mut wide, 1);
    put_u32(&mut wide, MAX_STRING_POOL_BYTES as u32);
    wide.resize(wide.len() + MAX_STRING_POOL_BYTES, b'x');
    let wide = read_string_pool(&wide).expect("a pool at the byte ceiling is valid");
    assert_eq!(wide[0].len(), MAX_STRING_POOL_BYTES);

    // Section table: exactly MAX_MFPC_SECTIONS distinct sections.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MFPC_MAGIC);
    put_u16(&mut bytes, 2);
    put_u16(&mut bytes, 0);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, MAX_MFPC_SECTIONS as u32);
    for index in 0..MAX_MFPC_SECTIONS {
        put_u16(&mut bytes, index as u16 + 1);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, 0);
        put_u64(&mut bytes, 0);
        put_u64(&mut bytes, 0);
    }
    let table = read_section_table(&bytes).expect("a table at the section ceiling is valid");
    assert_eq!(table.len(), MAX_MFPC_SECTIONS);

    // ABI index: exactly MAX_ABI_EXPORTS exports.
    let mut abi = Vec::with_capacity(8 + MAX_ABI_EXPORTS * 38);
    put_u16(&mut abi, ABI_FORMAT_VERSION);
    put_u16(&mut abi, 0);
    put_u32(&mut abi, MAX_ABI_EXPORTS as u32);
    for index in 0..MAX_ABI_EXPORTS {
        put_u32(&mut abi, (index % 2) as u32);
        put_u16(&mut abi, 1);
        abi.extend_from_slice(&[0u8; ABI_HASH_LEN]);
    }
    let payload = container(&[
        (SECTION_STRING_POOL, string_pool(&["greet", "farewell"])),
        (SECTION_ABI_INDEX, abi),
    ]);
    let map = parse_abi_index(&payload).expect("an index at the export ceiling is valid");
    assert_eq!(map.len(), 2, "every export names one of the two strings");

    // Package meta: exactly MAX_PACKAGE_META_FIELDS fields, the last one
    // the real description.
    let mut section = Vec::new();
    put_u32(&mut section, MAX_PACKAGE_META_FIELDS as u32);
    for _ in 0..MAX_PACKAGE_META_FIELDS - 1 {
        put_u16(&mut section, 777);
        put_u32(&mut section, 0);
    }
    put_u16(&mut section, PACKAGE_META_FIELD_DESCRIPTION);
    put_u32(&mut section, 4);
    section.extend_from_slice(b"real");
    let payload = container(&[(SECTION_PACKAGE_META, section)]);
    assert_eq!(
        parse_package_description(&payload).unwrap().as_deref(),
        Some("real"),
    );
}
