//! Data-object layout. ISA-neutral — identical in shape to the AArch64 encoder's
//! `data` module (a string object is a `u64` length prefix + bytes + NUL; a raw
//! object is decoded hex), kept local so the x86 encoder is self-contained.

use super::*;

pub(super) fn encode_data(plan: &NativeCodePlan) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    for object in &plan.data_objects {
        data.resize(align(data.len(), object.align), 0);
        if object.kind == "raw" {
            data.extend_from_slice(&decode_hex_bytes(&object.value)?);
        } else {
            put_u64(&mut data, object.value.len() as u64);
            data.extend_from_slice(object.value.as_bytes());
            data.push(0);
        }
        data.resize(align(data.len(), object.align), 0);
    }
    Ok(data)
}

fn decode_hex_bytes(value: &str) -> Result<Vec<u8>, String> {
    let compact = value
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace() && *byte != b'_')
        .collect::<Vec<_>>();
    if compact.len() % 2 != 0 {
        return Err("raw data object hex value must have an even digit count".to_string());
    }
    compact
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_digit(pair[0])?;
            let low = hex_digit(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_digit(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("raw data object contains non-hex digit".to_string()),
    }
}

pub(super) fn align(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
