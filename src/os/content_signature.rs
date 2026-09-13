//! The executable content signature: the `contentSignature` field of the
//! `mfb-signing-v1` blob (`./mfb spec package-manager signing`).
//!
//! A signed build's one-off signing key signs
//! `"MFB-EXECUTABLE-v1\0" || SHA-256(covered bytes)`, where the covered bytes are
//! the whole image up to [`Binary::covered_end`] with the `.mfbsign` section body
//! replaced by zeros. The blob is written with a fixed-length placeholder in the
//! signature's place, so filling it moves nothing: every linker calls [`seal`] on
//! its finished image — on Mach-O before the ad-hoc code signature is computed,
//! since that signature hashes the blob too.
//!
//! [`seal`] locates the section with the same reader `mfb info` verifies with
//! (`crate::os::inspect`), so the range signed and the range checked cannot drift
//! apart.
//!
//! [`Binary::covered_end`]: crate::os::inspect::Binary::covered_end

use std::ops::Range;

use sha2::{Digest, Sha256};

use crate::arch::image::ExecutableSigning;

/// The `contentSignature` value a blob carries until [`seal`] fills it: the
/// base64url encoding of 64 zero bytes, the exact length of a real signature.
pub(crate) const CONTENT_SIGNATURE_PLACEHOLDER: &str = concat!(
    "AAAAAAAAAA",
    "AAAAAAAAAA",
    "AAAAAAAAAA",
    "AAAAAAAAAA",
    "AAAAAAAAAA",
    "AAAAAAAAAA",
    "AAAAAAAAAA",
    "AAAAAAAAAA",
    "AAAAAA"
);

/// SHA-256 over `bytes[..covered_end]` with the `blob` range read as zeros, or
/// `None` when the blob does not lie inside the covered range.
pub(crate) fn content_digest(
    bytes: &[u8],
    blob: &Range<usize>,
    covered_end: usize,
) -> Option<[u8; 32]> {
    if blob.start > blob.end || blob.end > covered_end || covered_end > bytes.len() {
        return None;
    }
    const ZEROS: [u8; 4096] = [0; 4096];
    let mut hasher = Sha256::new();
    hasher.update(&bytes[..blob.start]);
    let mut remaining = blob.len();
    while remaining > 0 {
        let chunk = remaining.min(ZEROS.len());
        hasher.update(&ZEROS[..chunk]);
        remaining -= chunk;
    }
    hasher.update(&bytes[blob.end..covered_end]);
    Some(hasher.finalize().into())
}

/// Fill the placeholder in `bytes`' `.mfbsign` blob with the content signature.
/// A no-op without a signing key — the blob is then carried as given.
pub(crate) fn seal(bytes: &mut [u8], signing: &ExecutableSigning) -> Result<(), String> {
    let Some(signing_private) = signing.signing_private.as_deref() else {
        return Ok(());
    };
    let binary = crate::os::inspect::inspect(bytes)
        .ok_or("content signature: the linked image does not read back as an MFBasic binary")?;
    let blob = binary
        .signing
        .ok_or("content signature: the linked image has no .mfbsign section")?;
    let slot = placeholder_offset(&bytes[blob.clone()])
        .ok_or("content signature: the signing metadata has no contentSignature placeholder")?;
    let digest = content_digest(bytes, &blob, binary.covered_end)
        .ok_or("content signature: the .mfbsign section lies outside the signed range")?;
    let signature = mfb_repository::crypto::sign(
        signing_private,
        &mfb_repository::crypto::executable_signing_input(&digest),
    )?;
    let encoded = mfb_repository::crypto::encode_bytes(&signature);
    if encoded.len() != CONTENT_SIGNATURE_PLACEHOLDER.len() {
        return Err("content signature: unexpected signature length".to_string());
    }
    let at = blob.start + slot;
    bytes[at..at + encoded.len()].copy_from_slice(encoded.as_bytes());
    Ok(())
}

/// Offset of the placeholder value inside `blob`, when the blob carries exactly
/// one `"contentSignature":"<placeholder>"` field. The proof and attestation are
/// embedded as JSON strings, so their quotes are escaped and cannot match.
fn placeholder_offset(blob: &[u8]) -> Option<usize> {
    let field = b"\"contentSignature\":\"";
    let needle = [
        field.as_slice(),
        CONTENT_SIGNATURE_PLACEHOLDER.as_bytes(),
        b"\"",
    ]
    .concat();
    let mut matches = blob
        .windows(needle.len())
        .enumerate()
        .filter(|(_, window)| *window == needle.as_slice())
        .map(|(at, _)| at + field.len());
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_placeholder_is_a_zero_signature_of_the_real_length() {
        assert_eq!(
            CONTENT_SIGNATURE_PLACEHOLDER,
            mfb_repository::crypto::encode_bytes(&[0; 64])
        );
    }

    #[test]
    fn the_digest_ignores_the_blob_and_nothing_else() {
        let bytes: Vec<u8> = (0..10_000u32).map(|n| n as u8).collect();
        let blob = 4_000..9_000;
        let digest = content_digest(&bytes, &blob, bytes.len()).unwrap();

        let mut inside = bytes.clone();
        inside[4_000] ^= 1;
        inside[8_999] ^= 1;
        assert_eq!(content_digest(&inside, &blob, bytes.len()), Some(digest));

        for at in [0, 3_999, 9_000, 9_999] {
            let mut outside = bytes.clone();
            outside[at] ^= 1;
            assert_ne!(content_digest(&outside, &blob, bytes.len()), Some(digest));
        }

        // Past `covered_end` is outside the signature.
        let short = content_digest(&bytes, &blob, 9_500).unwrap();
        let mut tail = bytes.clone();
        tail[9_600] ^= 1;
        assert_eq!(content_digest(&tail, &blob, 9_500), Some(short));
    }

    #[test]
    fn a_blob_outside_the_covered_range_has_no_digest() {
        let bytes = vec![0u8; 100];
        assert_eq!(content_digest(&bytes, &(90..110), 100), None);
        assert_eq!(content_digest(&bytes, &(10..20), 15), None);
        assert_eq!(content_digest(&bytes, &(10..20), 101), None);
    }

    #[test]
    fn the_placeholder_is_found_only_when_it_is_unique() {
        let field = format!("\"contentSignature\":\"{CONTENT_SIGNATURE_PLACEHOLDER}\"");
        let one = format!("{{\"a\":1,{field}}}");
        assert_eq!(
            placeholder_offset(one.as_bytes()),
            Some(one.find(CONTENT_SIGNATURE_PLACEHOLDER).unwrap())
        );
        assert_eq!(placeholder_offset(b"{\"contentSignature\":\"AAAA\"}"), None);
        let two = format!("{{{field},{field}}}");
        assert_eq!(placeholder_offset(two.as_bytes()), None);
        // An escaped copy inside an embedded JSON string does not count.
        let escaped = format!(
            "{{\"proof\":\"{{\\\"contentSignature\\\":\\\"{CONTENT_SIGNATURE_PLACEHOLDER}\\\"}}\",{field}}}"
        );
        assert!(placeholder_offset(escaped.as_bytes()).is_some());
    }

    #[test]
    fn sealing_without_a_key_leaves_the_image_untouched() {
        let signing = ExecutableSigning {
            metadata: Vec::new(),
            signing_private: None,
        };
        let mut bytes = b"not even an image".to_vec();
        seal(&mut bytes, &signing).expect("no key, nothing to do");
        assert_eq!(bytes, b"not even an image");
    }
}
