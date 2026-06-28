//! Assembles an [`AuditReport`] from the project manifest, parsed source, and
//! installed packages. All collection is offline and reuses the same project,
//! package, and `.mfp` helpers that builds use (via `crate::`).

use std::collections::{BTreeMap, HashMap, HashSet};
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
use project::{collect_native_resources, project_summary};
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
        native_links: Vec::new(),
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
