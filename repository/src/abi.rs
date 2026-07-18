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

/// One `vendor` locator drawn from a package's section-10 `NATIVE_LIBRARY_TABLE`
/// — the logical library it belongs to, its bare source filename, and the hex
/// SHA-256 that is the vendored file's blob key (plan-48-A §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendorBlobRef {
    pub logical: String,
    pub source: String,
    pub hash: String,
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
    let mut refs = Vec::new();
    for _ in 0..entry_count {
        let logical = table_string(strings, read_u32(bytes, offset)?)?;
        offset += 4;
        let locator_count = read_u32(bytes, offset)? as usize;
        offset += 4;
        for _ in 0..locator_count {
            // os, arch: interned string ids (skipped — not needed here).
            offset += 4; // os
            offset += 4; // arch
            let _libc = read_u8(bytes, offset)?;
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
    let mut strings = Vec::with_capacity(count.min(bytes.len()));
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
