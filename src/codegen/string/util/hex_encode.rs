//! Split from `the retired flat native_helpers.rs` (category `string.util`).

// --- codegen tier imports (migration) ---
/// Hex-encode `text` as a NUL-terminated C string payload (two hex digits per
/// byte, then `00`). Used to lay down read-only C-string data objects (library
/// sonames, `dlsym` names, framework paths).
pub(crate) fn hex_encode_cstring(text: &str) -> String {
    let mut hex = String::new();
    for byte in text.bytes() {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex.push_str("00"); // NUL terminator
    hex
}
