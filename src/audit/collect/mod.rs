//! Assembles an [`AuditReport`] from the project manifest, parsed source, and
//! installed packages. All collection is offline and reuses the same project,
//! package, and `.mfp` helpers that builds use (via `crate::`).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tinyjson::JsonValue;

use super::report::*;
use crate::ast::{self, CallArg, ConstructorArg, Expression, Function, Item, Statement};

mod dependencies;
mod findings;
mod lockfile;
mod project;
mod source;

use dependencies::{collect_dependencies, collect_packages};
use findings::{
    dependency_findings, lockfile_findings, package_findings, permission_findings,
    resource_findings, sort_findings,
};
use lockfile::collect_lockfile;
use project::{collect_native_links, collect_native_resources, project_summary};
use source::collect_source;

/// Inputs handed to the collector after the front-end pipeline has run.
pub struct AuditInputs<'a> {
    pub project_dir: &'a Path,
    pub root_display: String,
    pub manifest: &'a HashMap<String, JsonValue>,
    pub ast: &'a ast::AstProject,
    pub kind: String,
    pub entry: Option<String>,
    pub locked: bool,
}

pub fn collect(inputs: &AuditInputs) -> AuditReport {
    let project = project_summary(inputs);
    let dependencies = collect_dependencies(inputs.project_dir, inputs.manifest);
    let packages = collect_packages(inputs.project_dir, inputs.manifest);
    let (source_flow, permissions, resources) = collect_source(inputs.ast);
    let native_links = collect_native_links(&project.name, inputs.ast);
    let native_resources = collect_native_resources(&project.name, inputs.ast);
    let lockfile = collect_lockfile(inputs.project_dir, inputs.manifest, inputs.locked);

    let mut findings = Vec::new();
    lockfile_findings(&lockfile, &dependencies, inputs, &mut findings);
    dependency_findings(&dependencies, &mut findings);
    package_findings(
        inputs.project_dir,
        inputs.manifest,
        &packages,
        &mut findings,
    );
    resource_findings(&resources, &mut findings);
    permission_findings(&permissions, &mut findings);
    sort_findings(&mut findings);

    AuditReport {
        project,
        lockfile,
        dependencies,
        packages,
        source_flow,
        resources,
        native_links,
        native_resources,
        permissions,
        findings,
    }
}

pub(super) fn manifest_string(manifest: &HashMap<String, JsonValue>, key: &str) -> Option<String> {
    manifest
        .get(key)
        .and_then(|value| value.get::<String>())
        .cloned()
}

/// Lowercase hex SHA-256 over a canonical, sorted serialization of the
/// `project.json` `packages[]` request tuples.
pub fn project_hash(manifest: &HashMap<String, JsonValue>) -> String {
    use sha2::{Digest, Sha256};

    let mut tuples: Vec<String> = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .into_iter()
        .flatten()
        .filter_map(crate::manifest::package::project_package_dependency)
        .map(|dependency| {
            format!(
                "{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\n",
                dependency.name,
                dependency.ident,
                dependency.version,
                dependency.pin,
                dependency.source
            )
        })
        .collect();
    tuples.sort();

    let mut hasher = Sha256::new();
    for tuple in tuples {
        hasher.update(tuple.as_bytes());
    }
    crate::cli::pkg::hex_bytes(hasher.finalize().as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package_entry(name: &str, version: &str) -> JsonValue {
        let mut map = HashMap::new();
        map.insert("name".to_string(), JsonValue::String(name.to_string()));
        map.insert(
            "version".to_string(),
            JsonValue::String(version.to_string()),
        );
        JsonValue::Object(map)
    }

    #[test]
    fn manifest_string_reads_present_and_missing_keys() {
        let mut manifest = HashMap::new();
        manifest.insert("name".to_string(), JsonValue::String("demo".to_string()));
        manifest.insert("version".to_string(), JsonValue::Number(1.0));
        assert_eq!(manifest_string(&manifest, "name"), Some("demo".to_string()));
        // present but wrong type -> None
        assert_eq!(manifest_string(&manifest, "version"), None);
        // missing key -> None
        assert_eq!(manifest_string(&manifest, "absent"), None);
    }

    #[test]
    fn project_hash_empty_is_stable_and_lowercase_hex() {
        let manifest = HashMap::new();
        let hash = project_hash(&manifest);
        // SHA-256 hex is 64 chars.
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        // deterministic
        assert_eq!(hash, project_hash(&HashMap::new()));
    }

    #[test]
    fn project_hash_is_order_independent() {
        let mut a = HashMap::new();
        a.insert(
            "packages".to_string(),
            JsonValue::Array(vec![
                package_entry("alpha", "1.0.0"),
                package_entry("beta", "2.0.0"),
            ]),
        );
        let mut b = HashMap::new();
        b.insert(
            "packages".to_string(),
            JsonValue::Array(vec![
                package_entry("beta", "2.0.0"),
                package_entry("alpha", "1.0.0"),
            ]),
        );
        assert_eq!(project_hash(&a), project_hash(&b));
    }

    #[test]
    fn project_hash_differs_when_packages_change() {
        let mut a = HashMap::new();
        a.insert(
            "packages".to_string(),
            JsonValue::Array(vec![package_entry("alpha", "1.0.0")]),
        );
        let mut b = HashMap::new();
        b.insert(
            "packages".to_string(),
            JsonValue::Array(vec![package_entry("alpha", "2.0.0")]),
        );
        assert_ne!(project_hash(&a), project_hash(&b));
        // empty vs populated also differ
        assert_ne!(project_hash(&a), project_hash(&HashMap::new()));
    }
}
