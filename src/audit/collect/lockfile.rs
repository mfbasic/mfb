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
            version: None,
            project_hash_matches: None,
        };
    }

    let mut version = None;
    let mut project_hash_matches = None;
    if let Ok(contents) = std::fs::read_to_string(&lock_path) {
        if let Ok(value) = contents.parse::<JsonValue>() {
            if let Some(object) = value.get::<HashMap<String, JsonValue>>() {
                version = object
                    .get("lockfileVersion")
                    .and_then(|value| value.get::<f64>())
                    .map(|value| *value as i64);
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
        version,
        project_hash_matches,
    }
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
        assert!(summary.version.is_none());
        assert!(summary.project_hash_matches.is_none());
    }
}
