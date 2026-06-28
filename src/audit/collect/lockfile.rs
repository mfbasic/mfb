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
