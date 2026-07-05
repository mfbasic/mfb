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
const SECTION_ABI_INDEX: u16 = 15;
const ABI_FORMAT_VERSION: u16 = 1;
const ABI_HASH_LEN: usize = 32;

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
            (SECTION_ABI_INDEX, abi_section(&[(0, [0xaa; 32]), (1, [0xbb; 32])])),
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
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let slice = bytes.get(offset..offset + 2).ok_or("truncated u16")?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
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
