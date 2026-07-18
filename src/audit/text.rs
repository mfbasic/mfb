//! Deterministic human-readable rendering of an [`AuditReport`].

use super::report::*;
use std::borrow::Cow;
use std::fmt::Write;

/// Escapes terminal-unsafe code points in a string bound for the operator's
/// terminal — C0/C1 controls **and** Unicode bidi/format overrides (bug-210).
/// See [`crate::terminal_safe`] for why each class is escaped. Names, versions,
/// and paths in a report come from untrusted manifests and `.mfp` headers.
fn safe(value: &str) -> Cow<'_, str> {
    crate::terminal_safe::safe(value)
}

pub fn render(report: &AuditReport) -> String {
    let mut out = String::new();
    let project = &report.project;

    let _ = writeln!(
        out,
        "Audit: {} {} ({})",
        safe(&project.name),
        safe(&project.version),
        safe(&project.kind)
    );
    let _ = writeln!(out, "Root: {}", safe(&project.root));
    if !project.language_version.is_empty() {
        let _ = writeln!(out, "Language: {}", safe(&project.language_version));
    }
    if let Some(entry) = &project.entry {
        let _ = writeln!(out, "Entry: {}", safe(entry));
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
                safe(&dependency.requested_version).into_owned()
            };
            let resolved = match &dependency.resolved_version {
                Some(version) => format!(" -> {}", safe(version)),
                None => String::new(),
            };
            let _ = writeln!(
                out,
                "  {} {}{} {}{}",
                safe(&dependency.name),
                requested,
                pin,
                safe(&dependency.status),
                resolved
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
                safe(&package.name),
                safe(&package.version),
                safe(&package.verifier),
                safe(&package.signature),
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
                let _ = writeln!(out, "  {}", safe(&permission.capability));
                current = Some(permission.capability.clone());
            }
            let _ = writeln!(
                out,
                "    {} at {}:{}",
                safe(&permission.function),
                safe(&permission.path),
                permission.line
            );
        }
    }

    if !report.resources.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Resources:");
        for resource in &report.resources {
            let close = if resource.close_may_fail {
                format!("close {}, may fail", safe(&resource.close_op))
            } else {
                format!("close {}", safe(&resource.close_op))
            };
            let kind = if resource.native {
                "native"
            } else {
                "standard"
            };
            let _ = writeln!(
                out,
                "  {} {} at {}:{} ({}, {})",
                safe(&resource.resource_type),
                safe(&resource.name),
                safe(&resource.path),
                resource.line,
                kind,
                close
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
                safe(&link.package),
                safe(&link.symbol),
                safe(&link.close_function),
                link.may_fail
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
                format!("close {}, may fail", safe(&resource.close_op))
            } else {
                format!("close {}", safe(&resource.close_op))
            };
            let _ = writeln!(
                out,
                "  {} ({}) in {} at {}:{} (native, {}, {}, {})",
                safe(&resource.resource_type),
                visibility,
                safe(&resource.package),
                safe(&resource.path),
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
                safe(&function.function),
                safe(&function.path),
                function.line,
                fallible
            );
            if let Some(trap) = &function.trap {
                let _ = writeln!(
                    out,
                    "    trap {} -> {}",
                    safe(&trap.name),
                    trap.classification
                );
            }
            for call in &function.calls {
                let _ = writeln!(
                    out,
                    "    fallible call {} at {}:{} -> {}",
                    safe(&call.callee),
                    safe(&function.path),
                    call.line,
                    call.propagation
                );
            }
        }
    }

    if !report.findings.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Findings:");
        for finding in &report.findings {
            let location = match (&finding.path, finding.line) {
                (Some(path), Some(line)) => format!(" ({}:{line})", safe(path)),
                (Some(path), None) => format!(" ({})", safe(path)),
                _ => String::new(),
            };
            let _ = writeln!(
                out,
                "  {} {} {}{}",
                finding.severity.as_str(),
                safe(&finding.code),
                safe(&finding.message),
                location
            );
        }
    }

    out
}

fn lockfile_state(lockfile: &LockfileSummary) -> String {
    let mut state = if !lockfile.present {
        "absent".to_string()
    } else if !lockfile.parsed {
        // A bare "present" for a file that could not be decoded read as healthier
        // than a mismatch, which is backwards (bug-281).
        "present (unreadable)".to_string()
    } else {
        match lockfile.project_hash_matches {
            Some(true) => "present (projectHash matches)".to_string(),
            Some(false) => "present (projectHash mismatch)".to_string(),
            None => "present".to_string(),
        }
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
    fn untrusted_names_cannot_inject_terminal_control_sequences() {
        // A typosquatted package whose `.mfp` name erases the line it printed on
        // and forges an "ok" row beneath it.
        let mut report = full_report();
        report.dependencies[0].name = "legit\u{1b}[2K\rmalicious\nbeta 9.9.9 ok".to_string();
        report.packages[0].version = "1.0.0\u{7}".to_string();
        let out = render(&report);
        assert!(!out.contains('\u{1b}'), "no raw ESC reaches the terminal");
        assert!(!out.contains('\r'));
        assert!(!out.contains('\u{7}'));
        assert!(out.contains("legit\\u{001b}[2K\\u{000d}malicious\\u{000a}beta 9.9.9 ok"));
        assert!(out.contains("1.0.0\\u{0007}"));
        // The escaped name stays on the dependency's own line: the injected row
        // is not a line of its own.
        assert!(!out.lines().any(|line| line == "beta 9.9.9 ok"));
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
            parsed: true,
            version: None,
            project_hash_matches: Some(true),
        };
        assert_eq!(lockfile_state(&base), "present (projectHash matches)");

        let mut none_match = LockfileSummary {
            path: "mfb.lock".to_string(),
            present: true,
            locked: false,
            parsed: true,
            version: None,
            project_hash_matches: None,
        };
        assert_eq!(lockfile_state(&none_match), "present");
        none_match.locked = true;
        assert_eq!(lockfile_state(&none_match), "present [--locked]");

        // A file that could not be decoded says so rather than reading as a
        // healthy "present" (bug-281).
        let malformed = LockfileSummary {
            path: "mfb.lock".to_string(),
            present: true,
            locked: true,
            parsed: false,
            version: None,
            project_hash_matches: None,
        };
        assert_eq!(lockfile_state(&malformed), "present (unreadable) [--locked]");

        let absent = LockfileSummary {
            path: "mfb.lock".to_string(),
            present: false,
            locked: false,
            parsed: false,
            version: None,
            project_hash_matches: None,
        };
        assert_eq!(lockfile_state(&absent), "absent");
    }
}
