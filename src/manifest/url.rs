use std::path::PathBuf;

pub(crate) fn package_file_url_path(url: &str) -> Result<PathBuf, String> {
    let Some(path) = url.strip_prefix("file://") else {
        return Err("mfb pkg add currently supports only file:// URLs ending in .mfp".to_string());
    };

    if path.is_empty() {
        return Err("file:// URL must include an absolute package path".to_string());
    }
    if path.contains('?') || path.contains('#') {
        return Err("file:// package URLs must not include query strings or fragments".to_string());
    }

    let path = PathBuf::from(percent_decode_path(path)?);
    if !path.is_absolute() {
        return Err("file:// package URL must resolve to an absolute path".to_string());
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some("mfp") {
        return Err("file:// package URL must point to a .mfp file".to_string());
    }
    if !path.is_file() {
        return Err(format!("package file '{}' does not exist", path.display()));
    }

    Ok(path)
}

pub(super) fn percent_decode_path(path: &str) -> Result<String, String> {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err("file:// URL contains an incomplete percent escape".to_string());
            }
            let high = hex_value(bytes[index + 1])
                .ok_or_else(|| "file:// URL contains an invalid percent escape".to_string())?;
            let low = hex_value(bytes[index + 2])
                .ok_or_else(|| "file:// URL contains an invalid percent escape".to_string())?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8(decoded).map_err(|_| "file:// URL path is not valid UTF-8".to_string())
}

pub(super) fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A malformed percent escape is rejected, not silently decoded.
    ///
    /// `percent_decode_path` is the only thing between a `file://` package URL
    /// and a path the rest of `mfb pkg add` treats as trusted. Each case below is
    /// a distinct way the escape can be wrong, and they differ in WHERE they
    /// fail — truncated at the end, a bad digit in either nibble, and a byte
    /// sequence that decodes but is not text. A decoder that let any of them
    /// through would hand back a path built from garbage bytes.
    #[test]
    fn a_malformed_percent_escape_is_rejected() {
        for (input, want) in [
            ("/tmp/a%", "incomplete percent escape"),
            ("/tmp/a%2", "incomplete percent escape"),
            ("/tmp/a%zz", "invalid percent escape"),
            ("/tmp/a%2z", "invalid percent escape"),
            ("/tmp/a%ff", "not valid UTF-8"),
        ] {
            let err = percent_decode_path(input).err().unwrap_or_default();
            assert!(
                err.contains(want),
                "`{input}` must be rejected with {want:?}; got {err:?}"
            );
        }
    }

    /// A well-formed escape decodes, in either digit case.
    #[test]
    fn a_well_formed_percent_escape_decodes() {
        assert_eq!(
            percent_decode_path("/tmp/a%20b%2Fc").as_deref(),
            Ok("/tmp/a b/c")
        );
        assert_eq!(
            percent_decode_path("/tmp/plain").as_deref(),
            Ok("/tmp/plain")
        );
    }

    /// Every rejection `package_file_url_path` makes before touching the disk.
    ///
    /// The order matters as much as the set: the scheme, emptiness, query and
    /// fragment checks all run BEFORE the percent decode, so a URL that is not a
    /// `file://` package cannot reach the decoder at all.
    #[test]
    fn a_url_that_is_not_an_absolute_mfp_path_is_rejected_before_the_disk() {
        for (url, want) in [
            ("https://example.test/x.mfp", "only file:// URLs"),
            ("file://", "must include an absolute package path"),
            ("file:///tmp/x.mfp?v=1", "query strings or fragments"),
            ("file:///tmp/x.mfp#frag", "query strings or fragments"),
            ("file://relative/x.mfp", "must resolve to an absolute path"),
            ("file:///tmp/x.txt", "must point to a .mfp file"),
        ] {
            let err = package_file_url_path(url).err().unwrap_or_default();
            assert!(
                err.contains(want),
                "`{url}` must be rejected with {want:?}; got {err:?}"
            );
        }
    }
}
