use super::*;

pub(super) fn put_pair_list(bytes: &mut Vec<u8>, pairs: &[(String, String)]) {
    put_u32(bytes, pairs.len() as u32);
    for (first, second) in pairs {
        put_bytes(bytes, first.as_bytes());
        put_bytes(bytes, second.as_bytes());
    }
}

pub(super) fn put_optional_str(bytes: &mut Vec<u8>, value: &Option<String>) {
    match value {
        Some(message) => {
            bytes.push(1);
            put_bytes(bytes, message.as_bytes());
        }
        None => bytes.push(0),
    }
}

/// Prose blocks are stored as a count followed by `(u8 kind, str text)` pairs.
pub(super) fn put_prose_list(bytes: &mut Vec<u8>, prose: &[(u8, String)]) {
    put_u32(bytes, prose.len() as u32);
    for (kind, text) in prose {
        bytes.push(*kind);
        put_bytes(bytes, text.as_bytes());
    }
}

pub(super) fn cursor_prose_list(
    bytes: &[u8],
    offset: &mut usize,
) -> Result<Vec<(u8, String)>, String> {
    let count = cursor_u32(bytes, offset)? as usize;
    let mut values = Vec::with_capacity(bounded_capacity(count, bytes.len() - *offset, 5));
    for _ in 0..count {
        let kind = *bytes
            .get(*offset)
            .ok_or_else(|| "truncated prose kind".to_string())?;
        *offset += 1;
        values.push((kind, cursor_string(bytes, offset)?));
    }
    Ok(values)
}

pub(super) fn cursor_pair_list(
    bytes: &[u8],
    offset: &mut usize,
) -> Result<Vec<(String, String)>, String> {
    let count = cursor_u32(bytes, offset)? as usize;
    let mut values = Vec::with_capacity(bounded_capacity(count, bytes.len() - *offset, 8));
    for _ in 0..count {
        let first = cursor_string(bytes, offset)?;
        let second = cursor_string(bytes, offset)?;
        values.push((first, second));
    }
    Ok(values)
}

pub(super) fn cursor_optional_str(
    bytes: &[u8],
    offset: &mut usize,
) -> Result<Option<String>, String> {
    let flag = *bytes
        .get(*offset)
        .ok_or_else(|| "truncated optional string flag".to_string())?;
    *offset += 1;
    if flag == 0 {
        Ok(None)
    } else {
        Ok(Some(cursor_string(bytes, offset)?))
    }
}

/// Cap an attacker-supplied element count to what the remaining bytes could
/// possibly hold (PKG-05). Each element occupies at least `min_elem` (>= 1)
/// bytes on the wire, so `remaining / min_elem` is a hard upper bound on the
/// real element count; pre-allocating beyond it only serves a memory-exhaustion
/// DoS (a 4-byte `0xFFFF_FFFF` count would otherwise request gigabytes up
/// front). The vec still grows to the true length as elements are decoded.
pub(super) fn bounded_capacity(count: usize, remaining: usize, min_elem: usize) -> usize {
    count.min(remaining / min_elem.max(1))
}

pub(super) fn hash_bytes(bytes: &[u8]) -> [u8; ABI_HASH_LEN] {
    let digest = Sha256::digest(bytes);
    let mut hash = [0; ABI_HASH_LEN];
    hash.copy_from_slice(&digest);
    hash
}

pub(super) fn sorted_pairs(mut values: Vec<(String, String)>) -> Vec<(String, String)> {
    values.sort();
    values
}

pub(super) fn hex_hash(hash: &[u8; ABI_HASH_LEN]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn skip_length_prefixed(
    bytes: &[u8],
    offset: &mut usize,
    field: &str,
) -> Result<(), String> {
    let length = cursor_u32(bytes, offset)? as usize;
    let end = offset
        .checked_add(length)
        .ok_or_else(|| format!("invalid .mfp {field} length"))?;
    if end > bytes.len() {
        return Err(format!("truncated .mfp {field}"));
    }
    *offset = end;
    Ok(())
}

pub(super) fn read_length_prefixed(
    bytes: &[u8],
    offset: &mut usize,
    field: &str,
) -> Result<String, String> {
    let length = cursor_u32(bytes, offset)? as usize;
    let end = offset
        .checked_add(length)
        .ok_or_else(|| format!("invalid .mfp {field} length"))?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| format!("truncated .mfp {field}"))?;
    *offset = end;
    String::from_utf8(value.to_vec()).map_err(|_| format!(".mfp {field} is not valid UTF-8"))
}

pub(super) fn cursor_u8(bytes: &[u8], offset: &mut usize) -> Result<u8, String> {
    let value = *bytes
        .get(*offset)
        .ok_or_else(|| "truncated binary representation".to_string())?;
    *offset = offset
        .checked_add(1)
        .ok_or_else(|| "invalid u8 offset".to_string())?;
    Ok(value)
}

pub(super) fn cursor_u16(bytes: &[u8], offset: &mut usize) -> Result<u16, String> {
    let value = checked_u16_at(bytes, *offset)?;
    *offset = offset
        .checked_add(2)
        .ok_or_else(|| "invalid u16 offset".to_string())?;
    Ok(value)
}

pub(super) fn cursor_hash(bytes: &[u8], offset: &mut usize) -> Result<[u8; ABI_HASH_LEN], String> {
    let end = offset
        .checked_add(ABI_HASH_LEN)
        .ok_or_else(|| "invalid hash offset".to_string())?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| "truncated ABI hash".to_string())?;
    let mut hash = [0; ABI_HASH_LEN];
    hash.copy_from_slice(value);
    *offset = end;
    Ok(hash)
}

pub(super) fn cursor_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, String> {
    let value = checked_u32_at(bytes, *offset)?;
    *offset = offset
        .checked_add(4)
        .ok_or_else(|| "invalid u32 offset".to_string())?;
    Ok(value)
}

pub(super) fn cursor_u64(bytes: &[u8], offset: &mut usize) -> Result<u64, String> {
    let value = checked_u64_at(bytes, *offset)?;
    *offset = offset
        .checked_add(8)
        .ok_or_else(|| "invalid u64 offset".to_string())?;
    Ok(value)
}

/// Read a `u32`-length-prefixed UTF-8 string (as written by [`put_bytes`]).
pub(super) fn cursor_string(bytes: &[u8], offset: &mut usize) -> Result<String, String> {
    let length = cursor_u32(bytes, offset)? as usize;
    let end = offset
        .checked_add(length)
        .ok_or_else(|| "invalid length-prefixed string".to_string())?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| "truncated length-prefixed string".to_string())?;
    let value = std::str::from_utf8(value)
        .map_err(|_| "length-prefixed string is not valid UTF-8".to_string())?
        .to_string();
    *offset = end;
    Ok(value)
}

pub(super) fn checked_u16_at(bytes: &[u8], offset: usize) -> Result<u16, String> {
    // `checked_add` keeps `offset + N` from wrapping on a hostile offset (PKG-07),
    // so the slice bound stays correct on every target width.
    let end = offset
        .checked_add(2)
        .ok_or_else(|| "truncated binary representation".to_string())?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| "truncated binary representation".to_string())?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

pub(super) fn checked_u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| "truncated binary representation".to_string())?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| "truncated binary representation".to_string())?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

pub(super) fn checked_u64_at(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let end = offset
        .checked_add(8)
        .ok_or_else(|| "truncated binary representation".to_string())?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| "truncated binary representation".to_string())?;
    Ok(u64::from_le_bytes([
        value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
    ]))
}

/// Narrow a decoded 64-bit offset or length to `usize`, rejecting a value the host
/// cannot address.
///
/// `as usize` truncates on a 32-bit target, so a hostile `.mfp` declaring a length
/// of `0x1_0000_0000` would have its bounds validated against `0` — the structural
/// checks downstream would then pass on a length that does not describe the real
/// body.
pub(super) fn checked_usize(value: u64, field: &str) -> Result<usize, String> {
    usize::try_from(value)
        .map_err(|_| format!("invalid {field}: {value} exceeds the address space"))
}

impl Section {
    pub(super) fn new(id: u16, data: Vec<u8>) -> Self {
        Self { id, data }
    }
}

pub(super) fn encode_sections(sections: &[Section]) -> Vec<u8> {
    let section_table_size = sections.len() * 24;
    let mut offset = 16 + section_table_size;
    let mut bytes = Vec::new();

    bytes.extend_from_slice(b"MFPC");
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

pub(super) fn hex_dump(bytes: &[u8]) -> String {
    let mut output = String::new();
    for chunk in bytes.chunks(16) {
        for (index, byte) in chunk.iter().enumerate() {
            if index > 0 {
                output.push(' ');
            }
            output.push_str(&format!("{byte:02X}"));
        }
        output.push('\n');
    }
    output
}

pub(super) fn put_bytes(dst: &mut Vec<u8>, bytes: &[u8]) {
    put_u32(dst, bytes.len() as u32);
    dst.extend_from_slice(bytes);
}

pub(super) fn put_u16(dst: &mut Vec<u8>, value: u16) {
    dst.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn put_u32(dst: &mut Vec<u8>, value: u32) {
    dst.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn put_u64(dst: &mut Vec<u8>, value: u64) {
    dst.extend_from_slice(&value.to_le_bytes());
}
