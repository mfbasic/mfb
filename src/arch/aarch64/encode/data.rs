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

/// Round `value` up to a multiple of `alignment`. An alignment of 0 (only
/// reachable from a malformed plan — decoded `.mfp` IR is not re-validated
/// before codegen, see audit-1 PKG-02) means "no alignment": return `value`
/// unchanged rather than panicking in `div_ceil` with a divide-by-zero (bug-18).
pub(super) fn align(value: usize, alignment: usize) -> usize {
    if alignment <= 1 {
        return value;
    }
    value.div_ceil(alignment) * alignment
}

fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
