//! The provenance marker every executable the built-in linker emits carries
//! (plan-43). Both output formats wrap the *same* descriptor bytes in their own
//! blessed vendor-note mechanism — an ELF `PT_NOTE` whose note name is
//! `MFBasic\0`, and a Mach-O `LC_NOTE` whose `data_owner` is `MFBasic\0` — so a
//! tool that knows the owner string can read the descriptor out of either.
//!
//! The marker is unconditional: it does not depend on the separate
//! `signing_metadata` feature (`.mfb_sign` / `__MFB,__sign`), and both may be
//! present at once.

/// The vendor-note owner: the ELF note *name* and the Mach-O `data_owner`.
/// Exactly 8 bytes including the NUL, which keeps the ELF note name 4-aligned
/// (no `Elf64_Nhdr` name padding) and fits a Mach-O `data_owner[16]` field.
pub(crate) const MFB_NOTE_OWNER: &[u8; 8] = b"MFBasic\0";

/// The ELF note `type`. A single reserved vendor value for v1 — the descriptor's
/// own fields carry any discrimination a reader needs.
pub(crate) const MFB_NOTE_TYPE: u32 = 1;

/// Descriptor layout version (byte 4 of the descriptor).
const MFB_NOTE_VERSION: u16 = 1;

/// Size of [`mfb_note_descriptor`]'s output. 16 bytes keeps the ELF `descsz` a
/// multiple of 4 (no note padding) and is trivially 8-aligned for Mach-O.
pub(crate) const MFB_NOTE_DESCRIPTOR_SIZE: usize = 16;

/// The versioned descriptor both formats carry verbatim. Fixed little-endian
/// layout, v1:
///
/// | off | size | field              | value                    |
/// |-----|------|--------------------|--------------------------|
/// | 0   | 4    | inner magic        | `b"MFB1"`                |
/// | 4   | 2    | descriptor version | [`MFB_NOTE_VERSION`]     |
/// | 6   | 2    | flags              | 0 (reserved)             |
/// | 8   | 2    | compiler major     | crate version major      |
/// | 10  | 2    | compiler minor     | crate version minor      |
/// | 12  | 2    | compiler patch     | crate version patch      |
/// | 14  | 2    | pad                | 0                        |
///
/// The `MFBasic\0` string is the note *name* / `data_owner`, not part of this
/// payload.
pub(crate) fn mfb_note_descriptor() -> Vec<u8> {
    let (major, minor, patch) = compiler_version();
    let mut bytes = Vec::with_capacity(MFB_NOTE_DESCRIPTOR_SIZE);
    bytes.extend_from_slice(b"MFB1");
    bytes.extend_from_slice(&MFB_NOTE_VERSION.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes()); // flags — reserved
    bytes.extend_from_slice(&major.to_le_bytes());
    bytes.extend_from_slice(&minor.to_le_bytes());
    bytes.extend_from_slice(&patch.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes()); // pad
    debug_assert_eq!(bytes.len(), MFB_NOTE_DESCRIPTOR_SIZE);
    bytes
}

/// The compiler's `(major, minor, patch)` from the crate version.
fn compiler_version() -> (u16, u16, u16) {
    let mut parts = env!("CARGO_PKG_VERSION").split('.');
    let major = version_component(parts.next());
    let minor = version_component(parts.next());
    let patch = version_component(parts.next());
    (major, minor, patch)
}

/// One dotted component of the crate version as a `u16`. Takes the leading digit
/// run so a pre-release/build suffix (`0.2.0-rc1`) still yields `0`; a component
/// too large for the field saturates rather than wrapping into a wrong version.
fn version_component(component: Option<&str>) -> u16 {
    let digits: String = component
        .unwrap_or("0")
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    digits.parse().unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_matches_the_fixed_v1_layout() {
        let descriptor = mfb_note_descriptor();
        assert_eq!(descriptor.len(), MFB_NOTE_DESCRIPTOR_SIZE);
        assert_eq!(&descriptor[0..4], b"MFB1");
        assert_eq!(u16::from_le_bytes([descriptor[4], descriptor[5]]), 1);
        // flags and pad are reserved and must stay zero in v1.
        assert_eq!(u16::from_le_bytes([descriptor[6], descriptor[7]]), 0);
        assert_eq!(u16::from_le_bytes([descriptor[14], descriptor[15]]), 0);
    }

    #[test]
    fn descriptor_carries_the_crate_version() {
        let descriptor = mfb_note_descriptor();
        let mut parts = env!("CARGO_PKG_VERSION").split('.');
        let expect = |bytes: [u8; 2], part: Option<&str>| {
            assert_eq!(
                u16::from_le_bytes(bytes),
                part.unwrap().parse::<u16>().unwrap()
            );
        };
        expect([descriptor[8], descriptor[9]], parts.next());
        expect([descriptor[10], descriptor[11]], parts.next());
        expect([descriptor[12], descriptor[13]], parts.next());
    }

    #[test]
    fn owner_is_the_nul_terminated_eight_byte_vendor_string() {
        assert_eq!(MFB_NOTE_OWNER.len(), 8);
        assert_eq!(&MFB_NOTE_OWNER[..7], b"MFBasic");
        assert_eq!(MFB_NOTE_OWNER[7], 0);
    }

    #[test]
    fn version_component_takes_the_leading_digit_run() {
        assert_eq!(version_component(Some("7")), 7);
        assert_eq!(version_component(Some("0-rc1")), 0);
        assert_eq!(version_component(Some("12-alpha.3")), 12);
        assert_eq!(version_component(None), 0);
    }
}
