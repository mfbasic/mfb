use super::*;

pub(super) fn collect_lockfile(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
    locked: bool,
) -> LockfileSummary {
    let lock_path = project_dir.join("mfb.lock");
    let display = "mfb.lock".to_string();
    if !lock_path.is_file() {
        return LockfileSummary {
            path: display,
            present: false,
            locked,
            parsed: false,
            version: None,
            project_hash_matches: None,
        };
    }

    let mut version = None;
    let mut project_hash_matches = None;
    // Tracks whether the file actually decoded to a JSON object (bug-281). All
    // three failure modes — unreadable, invalid JSON, valid JSON that is not an
    // object — used to collapse into the same "present, hash unknown" state as a
    // readable lockfile with no `projectHash`, which produced no finding.
    let mut parsed = false;
    if let Ok(contents) = std::fs::read_to_string(&lock_path) {
        if let Ok(value) = contents.parse::<JsonValue>() {
            if let Some(object) = value.get::<HashMap<String, JsonValue>>() {
                parsed = true;
                version = object
                    .get("lockfileVersion")
                    .and_then(|value| value.get::<f64>())
                    .and_then(|value| lockfile_version(*value));
                let stored = object
                    .get("projectHash")
                    .and_then(|value| value.get::<String>())
                    .cloned()
                    .unwrap_or_default();
                project_hash_matches = Some(stored == project_hash(manifest));
            }
        }
    }

    LockfileSummary {
        path: display,
        present: true,
        locked,
        parsed,
        version,
        project_hash_matches,
    }
}

/// A `lockfileVersion` is a non-negative integer. JSON gives us an `f64`, and a
/// raw `as i64` would saturate `1e309` to `i64::MAX` and truncate `1.9` to `1`,
/// reporting a version the lockfile never stated. An out-of-range or fractional
/// value is malformed: report it as absent rather than as a plausible number.
fn lockfile_version(value: f64) -> Option<i64> {
    if !value.is_finite() || value.fract() != 0.0 || value < 0.0 {
        return None;
    }
    // f64 represents every integer up to 2^53 exactly; beyond that a value is
    // not a faithful version number.
    if value > (1i64 << 53) as f64 {
        return None;
    }
    Some(value as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn absent_lockfile_reports_not_present() {
        let dir = tempdir().unwrap();
        let manifest = HashMap::new();
        let summary = collect_lockfile(dir.path(), &manifest, true);
        assert_eq!(summary.path, "mfb.lock");
        assert!(!summary.present);
        assert!(summary.locked);
        assert!(summary.version.is_none());
        assert!(summary.project_hash_matches.is_none());
    }

    #[test]
    fn present_lockfile_reads_version_and_matches_empty_hash() {
        let dir = tempdir().unwrap();
        // For an empty packages set, project_hash is the SHA-256 of no tuples.
        let manifest: HashMap<String, JsonValue> = HashMap::new();
        let hash = project_hash(&manifest);
        let contents = format!("{{\"lockfileVersion\": 1, \"projectHash\": \"{hash}\"}}");
        fs::write(dir.path().join("mfb.lock"), contents).unwrap();
        let summary = collect_lockfile(dir.path(), &manifest, false);
        assert!(summary.present);
        assert_eq!(summary.version, Some(1));
        assert_eq!(summary.project_hash_matches, Some(true));
    }

    #[test]
    fn malformed_lockfile_version_is_reported_absent_not_saturated() {
        let dir = tempdir().unwrap();
        let manifest: HashMap<String, JsonValue> = HashMap::new();
        // `1e309` parses to f64 infinity; `as i64` used to saturate it to
        // i64::MAX and report that as the version.
        for version in ["1e309", "1.9", "-1", "1e300"] {
            fs::write(
                dir.path().join("mfb.lock"),
                format!("{{\"lockfileVersion\": {version}, \"projectHash\": \"x\"}}"),
            )
            .unwrap();
            let summary = collect_lockfile(dir.path(), &manifest, false);
            assert!(summary.present);
            assert_eq!(summary.version, None, "version {version} must be rejected");
        }
        assert_eq!(lockfile_version(0.0), Some(0));
        assert_eq!(lockfile_version(3.0), Some(3));
    }

    #[test]
    fn present_lockfile_with_wrong_hash_reports_mismatch() {
        let dir = tempdir().unwrap();
        let manifest: HashMap<String, JsonValue> = HashMap::new();
        fs::write(
            dir.path().join("mfb.lock"),
            "{\"lockfileVersion\": 2, \"projectHash\": \"deadbeef\"}",
        )
        .unwrap();
        let summary = collect_lockfile(dir.path(), &manifest, false);
        assert!(summary.present);
        assert_eq!(summary.version, Some(2));
        assert_eq!(summary.project_hash_matches, Some(false));
    }

    #[test]
    fn present_lockfile_missing_hash_field_uses_empty_string() {
        let dir = tempdir().unwrap();
        let manifest: HashMap<String, JsonValue> = HashMap::new();
        fs::write(dir.path().join("mfb.lock"), "{\"lockfileVersion\": 1}").unwrap();
        let summary = collect_lockfile(dir.path(), &manifest, false);
        // stored hash defaults to "" and will not match the real hash.
        assert_eq!(summary.project_hash_matches, Some(false));
        assert_eq!(summary.version, Some(1));
    }

    #[test]
    fn present_but_malformed_json_leaves_fields_none() {
        let dir = tempdir().unwrap();
        let manifest: HashMap<String, JsonValue> = HashMap::new();
        fs::write(dir.path().join("mfb.lock"), "not json at all").unwrap();
        let summary = collect_lockfile(dir.path(), &manifest, false);
        assert!(summary.present);
        assert!(!summary.parsed, "unparseable content is not `parsed`");
        assert!(summary.version.is_none());
        assert!(summary.project_hash_matches.is_none());
    }

    #[test]
    fn present_non_object_json_leaves_fields_none() {
        let dir = tempdir().unwrap();
        let manifest: HashMap<String, JsonValue> = HashMap::new();
        fs::write(dir.path().join("mfb.lock"), "[1, 2, 3]").unwrap();
        let summary = collect_lockfile(dir.path(), &manifest, false);
        assert!(summary.present);
        assert!(!summary.parsed, "valid JSON that is not an object is not `parsed`");
        assert!(summary.version.is_none());
        assert!(summary.project_hash_matches.is_none());
    }

    /// A readable JSON object *is* parsed even when its fields are missing or
    /// wrong — `parsed` tracks decodability, not validity (bug-281). Without this
    /// distinction the malformed finding would fire on a merely-stale lockfile.
    #[test]
    fn a_decodable_object_is_parsed_even_with_missing_fields() {
        let dir = tempdir().unwrap();
        let manifest: HashMap<String, JsonValue> = HashMap::new();
        fs::write(dir.path().join("mfb.lock"), "{}").unwrap();
        let summary = collect_lockfile(dir.path(), &manifest, false);
        assert!(summary.present);
        assert!(summary.parsed);
        // No projectHash field: the empty default will not match, so this is a
        // STALE lockfile, not a malformed one.
        assert_eq!(summary.project_hash_matches, Some(false));
    }
}
