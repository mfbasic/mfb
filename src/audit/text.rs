//! Deterministic human-readable rendering of an [`AuditReport`].

use super::report::*;
use std::fmt::Write;

pub fn render(report: &AuditReport) -> String {
    let mut out = String::new();
    let project = &report.project;

    let _ = writeln!(
        out,
        "Audit: {} {} ({})",
        project.name, project.version, project.kind
    );
    let _ = writeln!(out, "Root: {}", project.root);
    if !project.language_version.is_empty() {
        let _ = writeln!(out, "Language: {}", project.language_version);
    }
    if let Some(entry) = &project.entry {
        let _ = writeln!(out, "Entry: {entry}");
    }
    let _ = writeln!(out, "Lockfile: {}", lockfile_state(&report.lockfile));

    let counts = report.counts();
    let _ = writeln!(out);
    let _ = writeln!(out, "Summary:");
    let _ = writeln!(out, "  errors: {}", counts.errors);
    let _ = writeln!(out, "  warnings: {}", counts.warnings);
    let _ = writeln!(out, "  infos: {}", counts.infos);

    if !report.dependencies.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Dependencies:");
        for dependency in &report.dependencies {
            let pin = if dependency.pin { " pin" } else { "" };
            let requested = if dependency.requested_version.is_empty() {
                "*".to_string()
            } else {
                dependency.requested_version.clone()
            };
            let resolved = match &dependency.resolved_version {
                Some(version) => format!(" -> {version}"),
                None => String::new(),
            };
            let _ = writeln!(
                out,
                "  {} {}{} {}{}",
                dependency.name, requested, pin, dependency.status, resolved
            );
        }
    }

    if !report.packages.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Packages:");
        for package in &report.packages {
            let _ = writeln!(
                out,
                "  {} {} verifier={} signature={} exports={} imports={} cleanups={}",
                package.name,
                package.version,
                package.verifier,
                package.signature,
                package.exports,
                package.imports,
                package.cleanups
            );
        }
    }

    if !report.permissions.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Permissions:");
        let mut current = None;
        for permission in &report.permissions {
            if current.as_deref() != Some(permission.capability.as_str()) {
                let _ = writeln!(out, "  {}", permission.capability);
                current = Some(permission.capability.clone());
            }
            let _ = writeln!(
                out,
                "    {} at {}:{}",
                permission.function, permission.path, permission.line
            );
        }
    }

    if !report.resources.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Resources:");
        for resource in &report.resources {
            let close = if resource.close_may_fail {
                format!("close {}, may fail", resource.close_op)
            } else {
                format!("close {}", resource.close_op)
            };
            let kind = if resource.native {
                "native"
            } else {
                "standard"
            };
            let _ = writeln!(
                out,
                "  {} {} at {}:{} ({}, {})",
                resource.resource_type, resource.name, resource.path, resource.line, kind, close
            );
        }
    }

    if !report.native_links.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Native links:");
        for link in &report.native_links {
            let _ = writeln!(
                out,
                "  {} {} close={} may_fail={}",
                link.package, link.symbol, link.close_function, link.may_fail
            );
        }
    }

    let flow: Vec<&FlowFunction> = report
        .source_flow
        .iter()
        .filter(|function| {
            function.fallible || function.trap.is_some() || !function.calls.is_empty()
        })
        .collect();
    if !flow.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Control flow:");
        for function in flow {
            let fallible = if function.fallible { " (fallible)" } else { "" };
            let _ = writeln!(
                out,
                "  {} at {}:{}{}",
                function.function, function.path, function.line, fallible
            );
            if let Some(trap) = &function.trap {
                let _ = writeln!(out, "    trap {} -> {}", trap.name, trap.classification);
            }
            for call in &function.calls {
                let _ = writeln!(
                    out,
                    "    fallible call {} at {}:{} -> {}",
                    call.callee, function.path, call.line, call.propagation
                );
            }
        }
    }

    if !report.findings.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Findings:");
        for finding in &report.findings {
            let location = match (&finding.path, finding.line) {
                (Some(path), Some(line)) => format!(" ({path}:{line})"),
                (Some(path), None) => format!(" ({path})"),
                _ => String::new(),
            };
            let _ = writeln!(
                out,
                "  {} {} {}{}",
                finding.severity.as_str(),
                finding.code,
                finding.message,
                location
            );
        }
    }

    out
}

fn lockfile_state(lockfile: &LockfileSummary) -> String {
    let mut state = if lockfile.present {
        match lockfile.project_hash_matches {
            Some(true) => "present (projectHash matches)".to_string(),
            Some(false) => "present (projectHash mismatch)".to_string(),
            None => "present".to_string(),
        }
    } else {
        "absent".to_string()
    };
    if lockfile.locked {
        state.push_str(" [--locked]");
    }
    state
}
