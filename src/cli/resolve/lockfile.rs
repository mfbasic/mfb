//! plan-176-L: the dependency resolver's lockfile code. Resolution itself (`update`,
//! `install`, `resolve`, `select_node`) lives in `cli/resolve/mod.rs`.

use super::*;

const LOCKFILE_VERSION: i64 = 1;

/// One resolved dependency written to `mfb.lock`.
pub(crate) struct LockedPackage {
    pub(crate) name: String,
    pub(crate) ident: String,
    pub(crate) requested: String,
    pub(crate) selected: String,
    pub(crate) hash: String,
    /// The pinned owner ident key (metadata form) — the trust anchor `install`
    /// verifies against offline (plan-23 §3.5); no signing-key status/window.
    pub(crate) ident_key: String,
    pub(crate) ident_fingerprint: String,
    pub(crate) state: String,
}

/// The resolved lock: the registry it targets, the pinned log checkpoint, and
/// the selected packages.
pub(crate) struct Lock {
    pub(crate) project_hash: String,
    pub(crate) repo_fingerprint: String,
    pub(crate) checkpoint_size: i64,
    pub(crate) checkpoint_root: String,
    pub(crate) packages: Vec<LockedPackage>,
}

pub(super) fn lock_path(project_dir: &Path) -> PathBuf {
    project_dir.join("mfb.lock")
}

/// Write `mfb.lock` with a byte-stable formatting so re-resolving an unchanged
/// project reproduces the file exactly.
pub(crate) fn write_lock(project_dir: &Path, lock: &Lock) -> Result<(), String> {
    let path = lock_path(project_dir);
    fs::write(&path, render_lock(lock))
        .map_err(|err| format!("failed to write '{}': {err}", path.display()))
}

pub(super) fn render_lock(lock: &Lock) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"lockfileVersion\": {LOCKFILE_VERSION},\n"));
    out.push_str(&format!(
        "  \"projectHash\": {},\n",
        json_string(&lock.project_hash)
    ));
    out.push_str(&format!(
        "  \"repoFingerprint\": {},\n",
        json_string(&lock.repo_fingerprint)
    ));
    out.push_str("  \"checkpoint\": {\n");
    out.push_str(&format!("    \"size\": {},\n", lock.checkpoint_size));
    out.push_str(&format!(
        "    \"rootHash\": {}\n",
        json_string(&lock.checkpoint_root)
    ));
    out.push_str("  },\n");
    out.push_str("  \"packages\": [");
    for (index, package) in lock.packages.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("\n    {\n");
        out.push_str(&format!(
            "      \"name\": {},\n",
            json_string(&package.name)
        ));
        out.push_str(&format!(
            "      \"ident\": {},\n",
            json_string(&package.ident)
        ));
        out.push_str(&format!(
            "      \"requested\": {},\n",
            json_string(&package.requested)
        ));
        out.push_str(&format!(
            "      \"selected\": {},\n",
            json_string(&package.selected)
        ));
        out.push_str(&format!(
            "      \"hash\": {},\n",
            json_string(&package.hash)
        ));
        out.push_str(&format!(
            "      \"identKey\": {},\n",
            json_string(&package.ident_key)
        ));
        out.push_str(&format!(
            "      \"identFingerprint\": {},\n",
            json_string(&package.ident_fingerprint)
        ));
        out.push_str(&format!(
            "      \"state\": {}\n",
            json_string(&package.state)
        ));
        out.push_str("    }");
    }
    if lock.packages.is_empty() {
        out.push_str("]\n");
    } else {
        out.push_str("\n  ]\n");
    }
    out.push_str("}\n");
    out
}

/// Read an existing `mfb.lock`, if present.
pub(crate) fn read_lock(project_dir: &Path) -> Result<Option<Lock>, String> {
    let path = lock_path(project_dir);
    if !path.is_file() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let value = crate::json::parse_json_bounded(&contents)
        .map_err(|_| format!("'{}' is not valid JSON", path.display()))?;
    let object = value
        .get::<std::collections::HashMap<String, JsonValue>>()
        .ok_or_else(|| "mfb.lock must be a JSON object".to_string())?;
    let string_field = |name: &str| -> String {
        object
            .get(name)
            .and_then(|value| value.get::<String>())
            .cloned()
            .unwrap_or_default()
    };
    let checkpoint = object
        .get("checkpoint")
        .and_then(|value| value.get::<std::collections::HashMap<String, JsonValue>>());
    let checkpoint_size = checkpoint
        .and_then(|object| object.get("size"))
        .and_then(|value| value.get::<f64>())
        .map(|size| *size as i64)
        .unwrap_or(0);
    let checkpoint_root = checkpoint
        .and_then(|object| object.get("rootHash"))
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    let packages = object
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .map(|packages| {
            packages
                .iter()
                .filter_map(|entry| {
                    let object = entry.get::<std::collections::HashMap<String, JsonValue>>()?;
                    let get = |name: &str| {
                        object
                            .get(name)
                            .and_then(|value| value.get::<String>())
                            .cloned()
                            .unwrap_or_default()
                    };
                    Some(LockedPackage {
                        name: get("name"),
                        ident: get("ident"),
                        requested: get("requested"),
                        selected: get("selected"),
                        hash: get("hash"),
                        ident_key: get("identKey"),
                        ident_fingerprint: get("identFingerprint"),
                        state: get("state"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(Lock {
        project_hash: string_field("projectHash"),
        repo_fingerprint: string_field("repoFingerprint"),
        checkpoint_size,
        checkpoint_root,
        packages,
    }))
}

pub(super) fn print_lock_diff(previous: Option<&Lock>, next: &Lock) {
    let old: BTreeMap<&str, &LockedPackage> = previous
        .map(|lock| {
            lock.packages
                .iter()
                .map(|p| (p.ident.as_str(), p))
                .collect()
        })
        .unwrap_or_default();
    println!("Resolution:");
    for package in &next.packages {
        match old.get(package.ident.as_str()) {
            // bug-489: name/version/state are all registry-authored.
            None => println!(
                "  + {} {} ({})",
                crate::terminal_safe::safe(&package.name),
                crate::terminal_safe::safe(&package.selected),
                crate::terminal_safe::safe(&package.state)
            ),
            Some(before) if before.selected != package.selected => println!(
                "  ~ {} {} -> {} ({})",
                crate::terminal_safe::safe(&package.name),
                crate::terminal_safe::safe(&before.selected),
                crate::terminal_safe::safe(&package.selected),
                crate::terminal_safe::safe(&package.state)
            ),
            Some(_) => println!(
                "    {} {} ({})",
                crate::terminal_safe::safe(&package.name),
                crate::terminal_safe::safe(&package.selected),
                crate::terminal_safe::safe(&package.state)
            ),
        }
    }
    if let Some(previous) = previous {
        for package in &previous.packages {
            if !next.packages.iter().any(|p| p.ident == package.ident) {
                println!(
                    "  - {} {}",
                    crate::terminal_safe::safe(&package.name),
                    crate::terminal_safe::safe(&package.selected)
                );
            }
        }
    }
}
