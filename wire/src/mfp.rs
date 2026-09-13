//! The `.mfp` container's fixed prefix: magic, field readers, and the
//! signature-header rule (plan-126-B Phase 3).
//!
//! # Three decoders, one set of primitives
//!
//! The `.mfp` fixed prefix is decoded in **three** places, each advancing a
//! shared offset over the same field order, and each with a deliberately
//! *different* guard set:
//!
//! | decoder | `ident` | per-field byte limits | name charset |
//! |---|---|---|---|
//! | `manifest::package::read_mfp_header` | optional | yes | `validate_package_name` |
//! | `binary_repr::reader::mfp_binary_repr_payload` | required non-empty | no | none |
//! | `mfb_repository::package::parse_mfp_package` | required non-empty | yes | none |
//!
//! **Those differences are policy, and they stay.** bug-340 B8 recorded the
//! decision not to merge the decoders: the manifest reader's extra guards sit
//! on a trust boundary (a header `name` becomes `packages/<name>.mfp`, so
//! `../../x` escapes the project), and the registry requires an `ident` the
//! manifest reader tolerates as absent. Folding them into one function would
//! have to drop one side's guards or impose them on the other.
//!
//! What they share is *how to read a length-prefixed field without trusting the
//! length*. That is what lives here. The guards stay parameters
//! ([`read_mfp_string`]'s `limit` and `required`) rather than being baked in, so
//! each caller keeps naming its own policy at the call site.
//!
//! # The field order
//!
//! ```text
//! magic(8) containerMajor(u16) containerMinor(u16)
//! binaryReprMajor(u16) binaryReprMinor(u16) flags(u32)     <- FIXED_PREFIX_LEN = 20
//! name ident version author url identKey signingKey
//! proof proofSig attestation attestationSig                <- all u32-length-prefixed
//! packageBinaryHash(32 raw bytes, no length prefix)
//! binaryReprLength(u64) signatureType(u16) signatureLength(u32)
//! signature packageBinaryRepr
//! ```

use crate::bytes::{checked_u16_at, checked_u32_at, checked_u64_at};

/// The 8-byte `.mfp` container magic (plan-23 §4).
///
/// One home for what three decoders each used to declare their own copy of —
/// the registry's was a bare literal array with no name at all (bug-340 B8).
pub const MFP_MAGIC: [u8; 8] = [0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];

/// Bytes before the first variable-length field: magic (8) +
/// containerMajor/Minor (4) + binaryReprMajor/Minor (4) + flags (4).
pub const FIXED_PREFIX_LEN: usize = 20;

/// Read a `u32`-length-prefixed field, rejecting a length the container cannot
/// hold and one over the caller's `limit`.
///
/// `limit` is a **parameter, not a constant**: it is the per-field cap the
/// manifest reader and the registry reader both apply (`name` 255, `url` 2048,
/// `proof` 4096, …), and the compiler's payload decoder deliberately does not.
/// Keeping it in the signature is what lets one reader serve all three policies.
pub fn read_mfp_bytes(
    bytes: &[u8],
    offset: &mut usize,
    field: &str,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let length = read_u32(bytes, *offset)? as usize;
    *offset = offset
        .checked_add(4)
        .ok_or_else(|| format!("invalid .mfp {field} length"))?;

    // The limit is checked *before* the bounds check so an over-cap field
    // reports the cap rather than "truncated" — a 5 KB `proof` in a 4 KB
    // container is a policy violation, not a damaged file, and the two need
    // different operator responses.
    if length > limit {
        return Err(format!(".mfp {field} exceeds the {limit} byte limit"));
    }

    let end = offset
        .checked_add(length)
        .ok_or_else(|| format!("invalid .mfp {field} length"))?;
    if end > bytes.len() {
        return Err(format!("truncated .mfp {field}"));
    }

    let value = bytes[*offset..end].to_vec();
    *offset = end;
    Ok(value)
}

/// [`read_mfp_bytes`] plus UTF-8 validation and an optional non-empty check.
///
/// `required` is the second policy parameter: the manifest reader takes `ident`
/// as `required=false` (a locally-built package has none) while the registry
/// takes it as `required=true` (an unidentified package cannot be published).
/// That single boolean is the whole of the divergence bug-340 B8 protects on
/// this field.
pub fn read_mfp_string(
    bytes: &[u8],
    offset: &mut usize,
    field: &str,
    limit: usize,
    required: bool,
) -> Result<String, String> {
    let raw = read_mfp_bytes(bytes, offset, field, limit)?;
    let value = String::from_utf8(raw).map_err(|_| format!(".mfp {field} is not valid UTF-8"))?;
    if required && value.is_empty() {
        return Err(format!(".mfp {field} must not be empty"));
    }
    Ok(value)
}

/// The `.mfp` signature-type/length rule.
///
/// Type 0 is unsigned and must carry a zero-length signature; type 1 is Ed25519
/// and must carry exactly 64 bytes. Any other type is refused rather than
/// skipped — a signature algorithm this build does not implement must not read
/// as "no signature to check".
///
/// The compiler and the registry each had their own copy of this match
/// (`validate_mfp_signature_header` and `validate_signature_header`). They were
/// diffed before merging and their bodies were **identical**, so nothing was
/// chosen over anything: this is that one rule.
pub fn validate_signature_header(
    signature_type: u16,
    signature_length: usize,
) -> Result<(), String> {
    match (signature_type, signature_length) {
        (0, 0) | (1, 64) => Ok(()),
        (0, _) => Err("unsigned .mfp package must have zero signature length".to_string()),
        (1, _) => Err("Ed25519 .mfp package must have a 64 byte signature".to_string()),
        _ => Err(format!("unsupported .mfp signature type {signature_type}")),
    }
}

/// Read a `u16` from the `.mfp` header at an absolute offset.
///
/// The `.mfp`-flavoured error text ("truncated .mfp header") is why these are
/// distinct from `bytes::checked_u16_at`, whose message names the binary
/// representation instead. Both header decoders reported the former.
pub fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    checked_u16_at(bytes, offset).map_err(|_| "truncated .mfp header".to_string())
}

/// Read a `u32` from the `.mfp` header at an absolute offset.
pub fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    checked_u32_at(bytes, offset).map_err(|_| "truncated .mfp header".to_string())
}

/// Read a `u64` from the `.mfp` header at an absolute offset.
pub fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    checked_u64_at(bytes, offset).map_err(|_| "truncated .mfp header".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `u32`-length-prefixed field.
    fn prefixed(value: &[u8]) -> Vec<u8> {
        let mut bytes = (value.len() as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(value);
        bytes
    }

    #[test]
    fn magic_and_fixed_prefix_are_the_frozen_values() {
        // Wire format. If either of these changes, every published `.mfp`
        // stops decoding, so they are pinned by literal rather than by
        // reference to the constant they name.
        assert_eq!(MFP_MAGIC, [0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00]);
        assert_eq!(&MFP_MAGIC[0..3], b"MFP");
        assert_eq!(FIXED_PREFIX_LEN, 20);
    }

    #[test]
    fn read_mfp_bytes_round_trips_and_advances_the_offset() {
        let mut bytes = prefixed(b"hello");
        bytes.extend_from_slice(&prefixed(b"world"));
        let mut offset = 0;
        assert_eq!(
            read_mfp_bytes(&bytes, &mut offset, "first", 64).unwrap(),
            b"hello"
        );
        assert_eq!(offset, 9);
        assert_eq!(
            read_mfp_bytes(&bytes, &mut offset, "second", 64).unwrap(),
            b"world"
        );
        assert_eq!(offset, bytes.len());
    }

    #[test]
    fn read_mfp_bytes_rejects_a_truncated_field() {
        // Declares 10 bytes, carries 3.
        let mut bytes = 10u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(b"abc");
        let mut offset = 0;
        let err = read_mfp_bytes(&bytes, &mut offset, "name", 64).unwrap_err();
        assert_eq!(err, "truncated .mfp name");

        // A missing length prefix is a truncated *header*, not a truncated field.
        let mut offset = 0;
        let err = read_mfp_bytes(&[0, 0], &mut offset, "name", 64).unwrap_err();
        assert_eq!(err, "truncated .mfp header");
    }

    /// The limit is reported as a cap violation, not as truncation — an
    /// over-cap field in an otherwise intact container is a policy failure and
    /// needs a different operator response than a damaged file.
    #[test]
    fn read_mfp_bytes_reports_an_over_cap_field_as_a_limit_not_truncation() {
        let bytes = prefixed(&[b'x'; 300]);
        let mut offset = 0;
        let err = read_mfp_bytes(&bytes, &mut offset, "name", 255).unwrap_err();
        assert_eq!(err, ".mfp name exceeds the 255 byte limit");
        // And the same bytes are fine under the cap the payload decoder uses.
        let mut offset = 0;
        assert!(read_mfp_bytes(&bytes, &mut offset, "name", 4096).is_ok());
    }

    #[test]
    fn read_mfp_string_rejects_invalid_utf8() {
        // 0x80 is a continuation byte with no lead byte.
        let bytes = prefixed(&[0x80, 0x81]);
        let mut offset = 0;
        let err = read_mfp_string(&bytes, &mut offset, "name", 64, false).unwrap_err();
        assert_eq!(err, ".mfp name is not valid UTF-8");
    }

    /// The `required` parameter is the whole of the `ident` policy divergence
    /// bug-340 B8 protects: the same bytes must be accepted by one decoder's
    /// reading and rejected by the other's.
    #[test]
    fn required_is_the_only_difference_between_the_two_ident_policies() {
        let bytes = prefixed(b"");

        let mut offset = 0;
        assert_eq!(
            read_mfp_string(&bytes, &mut offset, "ident", 255, false).unwrap(),
            "",
            "the manifest reader's policy: an absent ident is normal",
        );

        let mut offset = 0;
        let err = read_mfp_string(&bytes, &mut offset, "ident", 255, true).unwrap_err();
        assert_eq!(
            err, ".mfp ident must not be empty",
            "the registry's policy: an unidentified package cannot be published",
        );
    }

    #[test]
    fn the_signature_header_rule_accepts_only_unsigned_zero_and_ed25519_64() {
        validate_signature_header(0, 0).unwrap();
        validate_signature_header(1, 64).unwrap();

        assert_eq!(
            validate_signature_header(0, 1).unwrap_err(),
            "unsigned .mfp package must have zero signature length",
        );
        assert_eq!(
            validate_signature_header(1, 63).unwrap_err(),
            "Ed25519 .mfp package must have a 64 byte signature",
        );
        // An unknown type is refused, never treated as "nothing to verify".
        assert_eq!(
            validate_signature_header(2, 64).unwrap_err(),
            "unsupported .mfp signature type 2",
        );
        assert!(validate_signature_header(u16::MAX, 0).is_err());
    }

    #[test]
    fn header_scalar_readers_report_a_truncated_header() {
        let bytes = [1u8, 0, 2, 0, 3, 0, 0, 0];
        assert_eq!(read_u16(&bytes, 0).unwrap(), 1);
        assert_eq!(read_u16(&bytes, 2).unwrap(), 2);
        assert_eq!(read_u32(&bytes, 4).unwrap(), 3);
        assert_eq!(read_u64(&bytes, 0).unwrap(), 0x0000_0003_0002_0001);

        for err in [
            read_u16(&bytes, 7).unwrap_err(),
            read_u32(&bytes, 5).unwrap_err(),
            read_u64(&bytes, 1).unwrap_err(),
        ] {
            assert_eq!(err, "truncated .mfp header");
        }
    }

    /// A hostile offset must not wrap into a valid-looking slice bound
    /// (PKG-07). The underlying `checked_*_at` guards this; the test pins that
    /// the `.mfp`-flavoured wrappers did not lose it.
    #[test]
    fn header_scalar_readers_reject_an_offset_that_would_wrap() {
        let bytes = [0u8; 8];
        assert!(read_u16(&bytes, usize::MAX).is_err());
        assert!(read_u32(&bytes, usize::MAX - 1).is_err());
        assert!(read_u64(&bytes, usize::MAX - 3).is_err());
    }
}
