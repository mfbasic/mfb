//! Byte-level decode/encode primitives for the `.mfp` container and its MFPC
//! payload (plan-126-B, moved verbatim from `src/binary_repr/util.rs`).
//!
//! Every reader here takes `(&[u8], &mut usize)` and advances the offset, so a
//! decoder reads as a sequence of field reads rather than hand-computed
//! indices. That shape is the point: the offset arithmetic that used to be
//! inline at every call site is in one place, guarded once.
//!
//! Three guard classes run through the whole file, and they are not
//! decoration — every one of them is a rejected `.mfp` away from a crash or a
//! mis-decode:
//!
//! * **`checked_add` on every offset** (PKG-07). `offset + N` on a hostile
//!   offset would wrap, and the slice bound would then be computed from the
//!   wrapped value.
//! * **[`checked_usize`] on every decoded 64-bit length**. `as usize`
//!   truncates on a 32-bit target, so a declared length of `0x1_0000_0000`
//!   would have its bounds validated against `0`.
//! * **[`bounded_capacity`] on every element count** (PKG-05). A 4-byte
//!   `0xFFFF_FFFF` count would otherwise pre-allocate gigabytes.
//!
//! The functions are `pub` rather than `pub(super)` because both sibling crates
//! consume them now. Nothing here reaches a filesystem, a socket or a clock;
//! `sha2` is the only dependency ([`hash_bytes`]).

use sha2::{Digest, Sha256};

/// Length of an ABI signature hash, and of every other SHA-256 in the format.
///
/// Wire format, frozen. Shared with the compiler, which re-exports it.
pub const ABI_HASH_LEN: usize = 32;

// `MFPC_MAJOR_VERSION`, `Section` and `encode_sections` moved to `crate::mfpc`
// (plan-126-C), beside the section ids and `read_section_table`. They were parked
// here by plan-126-B only because `encode_sections` needed them; that left this
// file — whose job is arch-neutral byte primitives — holding what was an MFPC
// module in all but name, and `MFPC_MAJOR_VERSION` defined twice once `mfpc`
// existed. One definition each now, in the module that owns the container.

pub fn put_pair_list(bytes: &mut Vec<u8>, pairs: &[(String, String)]) {
    put_u32(bytes, pairs.len() as u32);
    for (first, second) in pairs {
        put_bytes(bytes, first.as_bytes());
        put_bytes(bytes, second.as_bytes());
    }
}

pub fn put_optional_str(bytes: &mut Vec<u8>, value: &Option<String>) {
    match value {
        Some(message) => {
            bytes.push(1);
            put_bytes(bytes, message.as_bytes());
        }
        None => bytes.push(0),
    }
}

/// Prose blocks are stored as a count followed by `(u8 kind, str text)` pairs.
pub fn put_prose_list(bytes: &mut Vec<u8>, prose: &[(u8, String)]) {
    put_u32(bytes, prose.len() as u32);
    for (kind, text) in prose {
        bytes.push(*kind);
        put_bytes(bytes, text.as_bytes());
    }
}

pub fn cursor_prose_list(bytes: &[u8], offset: &mut usize) -> Result<Vec<(u8, String)>, String> {
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

pub fn cursor_pair_list(bytes: &[u8], offset: &mut usize) -> Result<Vec<(String, String)>, String> {
    let count = cursor_u32(bytes, offset)? as usize;
    let mut values = Vec::with_capacity(bounded_capacity(count, bytes.len() - *offset, 8));
    for _ in 0..count {
        let first = cursor_string(bytes, offset)?;
        let second = cursor_string(bytes, offset)?;
        values.push((first, second));
    }
    Ok(values)
}

pub fn cursor_optional_str(bytes: &[u8], offset: &mut usize) -> Result<Option<String>, String> {
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
pub fn bounded_capacity(count: usize, remaining: usize, min_elem: usize) -> usize {
    count.min(remaining / min_elem.max(1))
}

pub fn hash_bytes(bytes: &[u8]) -> [u8; ABI_HASH_LEN] {
    let digest = Sha256::digest(bytes);
    let mut hash = [0; ABI_HASH_LEN];
    hash.copy_from_slice(&digest);
    hash
}

pub fn sorted_pairs(mut values: Vec<(String, String)>) -> Vec<(String, String)> {
    values.sort();
    values
}

pub fn hex_hash(hash: &[u8; ABI_HASH_LEN]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn skip_length_prefixed(bytes: &[u8], offset: &mut usize, field: &str) -> Result<(), String> {
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

pub fn read_length_prefixed(
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

pub fn cursor_u8(bytes: &[u8], offset: &mut usize) -> Result<u8, String> {
    let value = *bytes
        .get(*offset)
        .ok_or_else(|| "truncated binary representation".to_string())?;
    *offset = offset
        .checked_add(1)
        .ok_or_else(|| "invalid u8 offset".to_string())?;
    Ok(value)
}

pub fn cursor_u16(bytes: &[u8], offset: &mut usize) -> Result<u16, String> {
    let value = checked_u16_at(bytes, *offset)?;
    *offset = offset
        .checked_add(2)
        .ok_or_else(|| "invalid u16 offset".to_string())?;
    Ok(value)
}

pub fn cursor_hash(bytes: &[u8], offset: &mut usize) -> Result<[u8; ABI_HASH_LEN], String> {
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

pub fn cursor_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, String> {
    let value = checked_u32_at(bytes, *offset)?;
    *offset = offset
        .checked_add(4)
        .ok_or_else(|| "invalid u32 offset".to_string())?;
    Ok(value)
}

pub fn cursor_u64(bytes: &[u8], offset: &mut usize) -> Result<u64, String> {
    let value = checked_u64_at(bytes, *offset)?;
    *offset = offset
        .checked_add(8)
        .ok_or_else(|| "invalid u64 offset".to_string())?;
    Ok(value)
}

/// Read a `u32`-length-prefixed UTF-8 string (as written by [`put_bytes`]).
pub fn cursor_string(bytes: &[u8], offset: &mut usize) -> Result<String, String> {
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

pub fn checked_u16_at(bytes: &[u8], offset: usize) -> Result<u16, String> {
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

pub fn checked_u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| "truncated binary representation".to_string())?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| "truncated binary representation".to_string())?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

pub fn checked_u64_at(bytes: &[u8], offset: usize) -> Result<u64, String> {
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
pub fn checked_usize(value: u64, field: &str) -> Result<usize, String> {
    usize::try_from(value)
        .map_err(|_| format!("invalid {field}: {value} exceeds the address space"))
}

pub fn hex_dump(bytes: &[u8]) -> String {
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

pub fn put_bytes(dst: &mut Vec<u8>, bytes: &[u8]) {
    put_u32(dst, bytes.len() as u32);
    dst.extend_from_slice(bytes);
}

pub fn put_u16(dst: &mut Vec<u8>, value: u16) {
    dst.extend_from_slice(&value.to_le_bytes());
}

pub fn put_u32(dst: &mut Vec<u8>, value: u32) {
    dst.extend_from_slice(&value.to_le_bytes());
}

pub fn put_u64(dst: &mut Vec<u8>, value: u64) {
    dst.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
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
    fn hex_dump_formats_rows_of_sixteen() {
        let out = hex_dump(&[0xAB, 0x00, 0xFF]);
        assert_eq!(out, "AB 00 FF\n");
        // 17 bytes wraps to a second line.
        let data: Vec<u8> = (0..17u8).collect();
        let dump = hex_dump(&data);
        assert_eq!(dump.lines().count(), 2);
    }
}
