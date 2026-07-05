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

    if !report.native_resources.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Native resources:");
        for resource in &report.native_resources {
            let visibility = if resource.exported {
                "exported"
            } else {
                "package-private"
            };
            let close = if resource.close_may_fail {
                format!("close {}, may fail", resource.close_op)
            } else {
                format!("close {}", resource.close_op)
            };
            let _ = writeln!(
                out,
                "  {} ({}) in {} at {}:{} (native, {}, {}, {})",
                resource.resource_type,
                visibility,
                resource.package,
                resource.path,
                resource.line,
                close,
                if resource.sendable {
                    "thread-sendable"
                } else {
                    "not thread-sendable"
                },
                "no pointer exposed",
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

#[cfg(test)]
mod tests {
    use super::super::report::testsupport::*;
    use super::*;

    #[test]
    fn renders_full_report_sections() {
        let out = render(&full_report());
        assert!(out.starts_with("Audit: demo 2.1.0 (program)\n"));
        assert!(out.contains("Root: path/to/root\n"));
        assert!(out.contains("Language: 1\n"));
        assert!(out.contains("Entry: main\n"));
        assert!(out.contains("Lockfile: present (projectHash mismatch) [--locked]\n"));
        assert!(out.contains("Summary:\n"));
        assert!(out.contains("  errors: 1\n"));
        assert!(out.contains("  warnings: 1\n"));
        assert!(out.contains("  infos: 1\n"));
        // dependencies: pin + resolved vs empty-requested (*) + no-resolve
        assert!(out.contains("Dependencies:\n"));
        assert!(out.contains("  alpha 1.2.0 pin ok -> 1.2.3\n"));
        assert!(out.contains("  beta * missing\n"));
        // packages
        assert!(out.contains("Packages:\n"));
        assert!(out.contains(
            "  alpha 1.2.3 verifier=ok signature=signed exports=3 imports=2 cleanups=1\n"
        ));
        // permissions grouped by capability header
        assert!(out.contains("Permissions:\n"));
        assert!(out.contains("  filesystem\n"));
        assert!(out.contains("    fs.open at main.mfb:12\n"));
        assert!(out.contains("  terminal\n"));
        // resources: may fail + standard, and native + no-fail
        assert!(out.contains("Resources:\n"));
        assert!(out.contains("  File file at main.mfb:11 (standard, close fs.close, may fail)\n"));
        assert!(out.contains("  Native handle at main.mfb:20 (native, close pkg.close)\n"));
        // native links
        assert!(out.contains("Native links:\n"));
        assert!(out.contains("  pkg sym close=closeFn may_fail=true\n"));
        // native resources: exported+sendable+mayfail and private+not-sendable
        assert!(out.contains("Native resources:\n"));
        assert!(out.contains("  Db (exported) in pkg at lib.mfb:5 (native, close pkg.close, may fail, thread-sendable, no pointer exposed)\n"));
        assert!(out.contains("  Cursor (package-private) in pkg at lib.mfb:9 (native, close pkg.free, not thread-sendable, no pointer exposed)\n"));
        // control flow: fallible + trap + call, and a pure fn is filtered out
        assert!(out.contains("Control flow:\n"));
        assert!(out.contains("  doWork at main.mfb:10 (fallible)\n"));
        assert!(out.contains("    trap err -> recovers\n"));
        assert!(out.contains("    fallible call fs.open at main.mfb:12 -> trap\n"));
        assert!(!out.contains("  pure at"));
        // findings: with path+line, with path only, with none
        assert!(out.contains("Findings:\n"));
        assert!(out.contains("  warning AUDIT-LOCK-STALE stale lock (mfb.lock)\n"));
        assert!(out.contains(
            "  info AUDIT-RESOURCE-CLOSE-MAY-FAIL resource close may fail (main.mfb:11)\n"
        ));
        assert!(out.contains("  error AUDIT-DEP-MISSING dep missing\n"));
    }

    #[test]
    fn renders_minimal_report_omits_optional_sections() {
        let mut report = empty_report();
        report.project.entry = None;
        report.project.language_version = String::new();
        let out = render(&report);
        assert!(out.contains("Lockfile: absent\n"));
        assert!(!out.contains("Language:"));
        assert!(!out.contains("Entry:"));
        assert!(!out.contains("Dependencies:"));
        assert!(!out.contains("Packages:"));
        assert!(!out.contains("Permissions:"));
        assert!(!out.contains("Resources:"));
        assert!(!out.contains("Native links:"));
        assert!(!out.contains("Native resources:"));
        assert!(!out.contains("Control flow:"));
        assert!(!out.contains("Findings:"));
    }

    #[test]
    fn lockfile_state_variants() {
        let base = LockfileSummary {
            path: "mfb.lock".to_string(),
            present: true,
            locked: false,
            version: None,
            project_hash_matches: Some(true),
        };
        assert_eq!(lockfile_state(&base), "present (projectHash matches)");

        let mut none_match = LockfileSummary {
            project_hash_matches: None,
            ..LockfileSummary {
                path: "mfb.lock".to_string(),
                present: true,
                locked: false,
                version: None,
                project_hash_matches: None,
            }
        };
        assert_eq!(lockfile_state(&none_match), "present");
        none_match.locked = true;
        assert_eq!(lockfile_state(&none_match), "present [--locked]");

        let absent = LockfileSummary {
            path: "mfb.lock".to_string(),
            present: false,
            locked: false,
            version: None,
            project_hash_matches: None,
        };
        assert_eq!(lockfile_state(&absent), "absent");
    }
}
