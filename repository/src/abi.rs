//! Minimal reader for the per-symbol ABI index the compiler embeds in a
//! package's `packageBinaryRepr` (plan-10-B1). The registry does not decode
//! the whole MFPC container — it only needs the string pool (section 2) and
//! the ABI index (section 15) to publish `{ "<symbol>": "<hex sigHash>" }` for
//! resolution. The section lives inside `packageBinaryRepr`, so plan-23's
//! `packageBinaryHash` and the package signature already cover it; the registry
//! parses it best-effort and never has to trust it for correctness.

use std::collections::BTreeMap;

const MFPC_MAGIC: &[u8; 4] = b"MFPC";
const SECTION_STRING_POOL: u16 = 2;
const SECTION_NATIVE_LIBRARY_TABLE: u16 = 10;
const SECTION_ABI_INDEX: u16 = 15;
const ABI_FORMAT_VERSION: u16 = 1;
const ABI_HASH_LEN: usize = 32;

/// Wire discriminant for a `vendor` native-library locator (plan-46-B §4.1).
/// A vendor locator carries a 32-byte SHA-256 of the file; a `system` locator
/// (discriminant 0) names a file the registry never sees and carries no hash.
const WIRE_LIB_TYPE_VENDOR: u8 = 1;
const NATIVE_LIBRARY_HASH_LEN: usize = 32;

/// Wire encoding of the `libc` axis (plan-46-B §4.1), mirrored from
/// `src/binary_repr/mod.rs:353-359`. This crate is a dependency *of* the
/// compiler crate, not the other way round, so the constants are restated here
/// rather than imported — as `MFPC_MAGIC` and the section ids above already are.
const WIRE_LIBC_UNSPECIFIED: u8 = 0;
const WIRE_LIBC_GLIBC: u8 = 1;
const WIRE_LIBC_MUSL: u8 = 2;

/// Decode the `libc` wire byte into the token stored in
/// `package_version_targets.libc`. An unrecognized value is an error, not a
/// silent `None`: section 10 rides inside the signed payload, so a byte outside
/// the vocabulary means a broken or tampered package, and reporting it as
/// "no libc constraint" would let a tampered locator widen its own platform
/// match.
fn decode_libc(raw: u8) -> Result<Option<String>, String> {
    match raw {
        WIRE_LIBC_UNSPECIFIED => Ok(None),
        WIRE_LIBC_GLIBC => Ok(Some("glibc".to_string())),
        WIRE_LIBC_MUSL => Ok(Some("musl".to_string())),
        other => Err(format!(
            "native library locator declares unknown libc discriminant {other}"
        )),
    }
}

/// Ceilings on what section 10 may declare (bug-275).
///
/// `entry_count` and `locator_count` are raw `u32`s read straight off an
/// attacker-controlled payload, and parsing previously stopped only when the
/// offset ran off the end. At ~46 bytes per locator a single ~48 MiB payload can
/// encode on the order of a million of them, and `validate_package_request`
/// probes the blob store once per locator — an S3 `head_object` each on the
/// hosted backend. One `/validate` or `/publish` could therefore fan out to ~1M
/// backend operations, reachable by any self-registered owner.
///
/// A real `libraries` table is one entry per logical library with one locator per
/// supported platform triple, so these bounds are orders of magnitude above any
/// legitimate package while capping the fan-out.
const MAX_VENDOR_ENTRIES: usize = 1024;
const MAX_VENDOR_LOCATORS: usize = 4096;

/// One `vendor` locator drawn from a package's section-10 `NATIVE_LIBRARY_TABLE`
/// — the logical library it belongs to, its bare source filename, the hex
/// SHA-256 that is the vendored file's blob key (plan-48-A §4.4), and the
/// platform triple it applies to (plan-61-A §3).
///
/// One `VendorBlobRef` is one *locator*, not one distinct blob. Two locators
/// may legitimately share a `hash` — two platforms shipping a byte-identical
/// build under different `source` filenames — so callers must never dedupe by
/// hash when accumulating platform support, or they under-report targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendorBlobRef {
    pub logical: String,
    pub source: String,
    pub hash: String,
    pub os: String,
    /// `None` is the any-arch wildcard (an empty `arch` string on the wire),
    /// meaning the locator matches every architecture on its OS. It is not
    /// missing data.
    pub arch: Option<String>,
    /// `None` when the locator specifies no libc; otherwise `"glibc"` or
    /// `"musl"`. A token rather than the wire integer so the value needs no
    /// mapping table downstream (plan-61-A §Open Decisions).
    pub libc: Option<String>,
    /// `"vendor"` or `"system"`. Constant `"vendor"` for every ref this parser
    /// produces (§3.1) — carried so capturing `system` locators later is not a
    /// schema change.
    pub lib_type: String,
}

/// Parse every `vendor` locator's blob hash from a package's `packageBinaryRepr`
/// payload (plan-48-A §4.4). Returns an empty vector when the package carries no
/// section-10 table (nothing to vendor). Errors only when a table is present but
/// malformed — which, since section 10 rides inside the signed+welded payload,
/// means a broken or tampered package the caller should refuse rather than
/// silently treat as vendoring nothing.
pub fn parse_vendor_blobs(payload: &[u8]) -> Result<Vec<VendorBlobRef>, String> {
    // A payload that is not even an MFPC container carries no section 10, so it
    // vendors nothing. Treated best-effort (like `abi_index_json`): the payload
    // is welded to the signature, so a genuinely broken one fails
    // `verify_payload_hash` elsewhere, and the registry never has to trust the
    // table for correctness. Only a *malformed section 10 inside a valid
    // container* is a real error worth surfacing.
    let Ok(sections) = read_section_table(payload) else {
        return Ok(Vec::new());
    };
    let Some(table_bytes) = sections.get(&SECTION_NATIVE_LIBRARY_TABLE) else {
        return Ok(Vec::new());
    };
    let string_bytes = sections
        .get(&SECTION_STRING_POOL)
        .ok_or_else(|| "package is missing the string pool section".to_string())?;
    let strings = read_string_pool(string_bytes)?;
    read_native_vendor_locators(table_bytes, &strings)
}

fn read_native_vendor_locators(
    bytes: &[u8],
    strings: &[String],
) -> Result<Vec<VendorBlobRef>, String> {
    let mut offset = 0usize;
    let entry_count = read_u32(bytes, offset)? as usize;
    offset += 4;
    // Reject on the declared count before allocating or probing anything.
    if entry_count > MAX_VENDOR_ENTRIES {
        return Err(format!(
            "native library table declares {entry_count} entries (limit {MAX_VENDOR_ENTRIES})"
        ));
    }
    let mut total_locators = 0usize;
    let mut refs = Vec::new();
    for _ in 0..entry_count {
        let logical = table_string(strings, read_u32(bytes, offset)?)?;
        offset += 4;
        let locator_count = read_u32(bytes, offset)? as usize;
        offset += 4;
        // Bound the *total* across entries, not just each entry: many small
        // entries reach the same fan-out as one oversized one.
        total_locators = total_locators.saturating_add(locator_count);
        if total_locators > MAX_VENDOR_LOCATORS {
            return Err(format!(
                "native library table declares more than {MAX_VENDOR_LOCATORS} locators"
            ));
        }
        for _ in 0..locator_count {
            // os, arch: interned string ids. An empty `arch` is the any-arch
            // wildcard and becomes `None` — distinct from a concrete arch, and
            // never conflated with an absent locator (plan-61-A §3, gotcha 1).
            let os = table_string(strings, read_u32(bytes, offset)?)?;
            offset += 4;
            let arch = table_string(strings, read_u32(bytes, offset)?)?;
            offset += 4;
            let arch = if arch.is_empty() { None } else { Some(arch) };
            let libc = decode_libc(read_u8(bytes, offset)?)?;
            offset += 1;
            let lib_type = read_u8(bytes, offset)?;
            offset += 1;
            let source = table_string(strings, read_u32(bytes, offset)?)?;
            offset += 4;
            if lib_type == WIRE_LIB_TYPE_VENDOR {
                let end = offset
                    .checked_add(NATIVE_LIBRARY_HASH_LEN)
                    .ok_or("native library locator hash overflows")?;
                let raw = bytes
                    .get(offset..end)
                    .ok_or("truncated native library locator hash")?;
                refs.push(VendorBlobRef {
                    logical: logical.clone(),
                    source,
                    hash: hex::encode(raw),
                    os,
                    arch,
                    libc,
                    lib_type: "vendor".to_string(),
                });
                offset = end;
            }
        }
    }
    Ok(refs)
}

fn table_string(strings: &[String], id: u32) -> Result<String, String> {
    strings
        .get(id as usize)
        .cloned()
        .ok_or_else(|| "native library table names an out-of-range string".to_string())
}

/// Parse the exported-symbol ABI map from a package's `packageBinaryRepr`
/// payload: `{ "<name>": "<hex sigHash>" }`, keys sorted. Errors if the payload
/// is not an MFPC container or the ABI/string sections are malformed; callers
/// treat any error as "no ABI index" (an empty object).
pub fn parse_abi_index(payload: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let sections = read_section_table(payload)?;
    let string_bytes = sections
        .get(&SECTION_STRING_POOL)
        .ok_or_else(|| "package is missing the string pool section".to_string())?;
    let abi_bytes = sections
        .get(&SECTION_ABI_INDEX)
        .ok_or_else(|| "package is missing the ABI index section".to_string())?;
    let strings = read_string_pool(string_bytes)?;
    read_abi_exports(abi_bytes, &strings)
}

/// The JSON `abiIndex` value for a package payload (empty object when absent).
pub fn abi_index_json(payload: &[u8]) -> serde_json::Value {
    match parse_abi_index(payload) {
        Ok(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(name, hash)| (name, serde_json::Value::String(hash)))
                .collect(),
        ),
        Err(_) => serde_json::json!({}),
    }
}

fn read_section_table(bytes: &[u8]) -> Result<BTreeMap<u16, &[u8]>, String> {
    if bytes.len() < 16 || &bytes[0..4] != MFPC_MAGIC {
        return Err("payload is not an MFPC container".to_string());
    }
    let section_count = read_u32(bytes, 12)? as usize;
    let table_end = 16usize
        .checked_add(section_count.checked_mul(24).ok_or("bad section table")?)
        .ok_or("bad section table")?;
    if table_end > bytes.len() {
        return Err("truncated MFPC section table".to_string());
    }
    let mut sections = BTreeMap::new();
    for index in 0..section_count {
        let entry = 16 + index * 24;
        let id = read_u16(bytes, entry)?;
        let offset = read_u64(bytes, entry + 8)? as usize;
        let length = read_u64(bytes, entry + 16)? as usize;
        let end = offset.checked_add(length).ok_or("bad section length")?;
        if end > bytes.len() {
            return Err("truncated MFPC section".to_string());
        }
        // A repeated section id is tampering (matches the compiler reader).
        if sections.insert(id, &bytes[offset..end]).is_some() {
            return Err(format!("duplicate MFPC section id {id}"));
        }
    }
    Ok(sections)
}

fn read_string_pool(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut offset = 0usize;
    let count = read_u32(bytes, offset)? as usize;
    offset += 4;
    // Bound the pre-allocation by how many entries the section could actually
    // hold, not by its byte length (bug-276 R8). A `String` is 24 bytes while the
    // smallest possible entry is its 4-byte length prefix, so `count.min(len)`
    // over-reserved by up to 24x — a ~48 MiB section declaring a huge count forced
    // a ~1.15 GiB transient allocation, on every /validate and /publish, for a
    // pool that can hold at most `len / 4` strings.
    let mut strings = Vec::with_capacity(count.min(bytes.len() / 4));
    for _ in 0..count {
        let length = read_u32(bytes, offset)? as usize;
        offset += 4;
        let end = offset.checked_add(length).ok_or("bad string length")?;
        if end > bytes.len() {
            return Err("truncated string pool entry".to_string());
        }
        strings.push(
            std::str::from_utf8(&bytes[offset..end])
                .map_err(|_| "string pool entry is not valid UTF-8".to_string())?
                .to_string(),
        );
        offset = end;
    }
    Ok(strings)
}

fn read_abi_exports(bytes: &[u8], strings: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut offset = 0usize;
    let version = read_u16(bytes, offset)?;
    offset += 2;
    if version != ABI_FORMAT_VERSION {
        return Err(format!("unsupported ABI index format version {version}"));
    }
    offset += 2; // reserved
    let export_count = read_u32(bytes, offset)? as usize;
    offset += 4;
    let mut map = BTreeMap::new();
    for _ in 0..export_count {
        let name_index = read_u32(bytes, offset)? as usize;
        offset += 4;
        let _kind = read_u16(bytes, offset)?;
        offset += 2;
        let hash_end = offset.checked_add(ABI_HASH_LEN).ok_or("bad ABI hash")?;
        if hash_end > bytes.len() {
            return Err("truncated ABI export hash".to_string());
        }
        let hash = hex::encode(&bytes[offset..hash_end]);
        offset = hash_end;
        let name = strings
            .get(name_index)
            .ok_or_else(|| "ABI export names an out-of-range string".to_string())?;
        map.insert(name.clone(), hash);
    }
    Ok(map)
}

/// A minimal `vendor` locator for tests that care only about the blob hash —
/// the version→blob edges in `store.rs` and the reachability walk in `gc.rs`,
/// which predate the platform axis plan-61-A added. Shared here so those tests
/// keep asserting what they always asserted without each restating a full
/// platform triple they do not use.
#[cfg(test)]
pub(crate) fn vendor_ref_for_hash(hash: &str) -> VendorBlobRef {
    VendorBlobRef {
        logical: "libtest".to_string(),
        source: format!("{hash}.a"),
        hash: hash.to_string(),
        os: "linux".to_string(),
        arch: Some("x86_64".to_string()),
        libc: None,
        lib_type: "vendor".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut pool = Vec::new();
        put_u32(&mut pool, u32::MAX); // declared count
        pool.extend_from_slice(&[0u8; 64]); // but only 64 bytes of entries
        let err = read_string_pool(&pool).unwrap_err();
        assert!(
            err.contains("truncated"),
            "expected a truncation rejection, got: {err}"
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

    #[test]
    fn read_integer_helpers_report_truncation() {
        assert!(read_u16(&[0u8], 0).is_err());
        assert!(read_u32(&[0u8; 2], 0).is_err());
        assert!(read_u64(&[0u8; 4], 0).is_err());
        assert_eq!(read_u16(&[2, 0], 0).unwrap(), 2);
        assert_eq!(read_u32(&[2, 0, 0, 0], 0).unwrap(), 2);
        assert_eq!(read_u64(&[2, 0, 0, 0, 0, 0, 0, 0], 0).unwrap(), 2);
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let slice = bytes.get(offset..offset + 2).ok_or("truncated u16")?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u8(bytes: &[u8], offset: usize) -> Result<u8, String> {
    bytes.get(offset).copied().ok_or("truncated u8".to_string())
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let slice = bytes.get(offset..offset + 4).ok_or("truncated u32")?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let slice = bytes.get(offset..offset + 8).ok_or("truncated u64")?;
    Ok(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}
