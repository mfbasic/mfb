//! Deterministic JSON rendering of an [`AuditReport`].
//!
//! `tinyjson` stores object members in a `HashMap`, so serializing through it
//! would produce non-deterministic key ordering. Instead we build an ordered
//! [`Json`] tree (objects keep insertion order) and emit it with a small
//! hand-rolled formatter, matching how the project already writes AST/IR JSON.

use super::report::*;

pub const SCHEMA: &str = "mfb.audit.v1";

enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(&'static str, Json)>),
}

impl Json {
    fn write(&self, out: &mut String, indent: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Json::Int(value) => out.push_str(&value.to_string()),
            Json::Str(value) => write_string(out, value),
            Json::Arr(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push('[');
                let pad = "  ".repeat(indent + 1);
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push('\n');
                    out.push_str(&pad);
                    item.write(out, indent + 1);
                }
                out.push('\n');
                out.push_str(&"  ".repeat(indent));
                out.push(']');
            }
            Json::Obj(fields) => {
                if fields.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push('{');
                let pad = "  ".repeat(indent + 1);
                for (index, (key, value)) in fields.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push('\n');
                    out.push_str(&pad);
                    write_string(out, key);
                    out.push_str(": ");
                    value.write(out, indent + 1);
                }
                out.push('\n');
                out.push_str(&"  ".repeat(indent));
                out.push('}');
            }
        }
    }
}

fn write_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn opt_str(value: &Option<String>) -> Json {
    match value {
        Some(value) => Json::Str(value.clone()),
        None => Json::Null,
    }
}

fn opt_int(value: Option<i64>) -> Json {
    match value {
        Some(value) => Json::Int(value),
        None => Json::Null,
    }
}

fn opt_bool(value: Option<bool>) -> Json {
    match value {
        Some(value) => Json::Bool(value),
        None => Json::Null,
    }
}

pub fn render(report: &AuditReport) -> String {
    let counts = report.counts();
    let project = &report.project;

    let root = Json::Obj(vec![
        ("schema", Json::Str(SCHEMA.to_string())),
        (
            "tool",
            Json::Obj(vec![
                ("name", Json::Str("mfb".to_string())),
                ("version", Json::Str(env!("CARGO_PKG_VERSION").to_string())),
            ]),
        ),
        (
            "project",
            Json::Obj(vec![
                ("name", Json::Str(project.name.clone())),
                ("ident", Json::Str(project.ident.clone())),
                ("version", Json::Str(project.version.clone())),
                ("kind", Json::Str(project.kind.clone())),
                ("root", Json::Str(project.root.clone())),
                ("entry", opt_str(&project.entry)),
                (
                    "languageVersion",
                    Json::Str(project.language_version.clone()),
                ),
            ]),
        ),
        (
            "summary",
            Json::Obj(vec![
                ("errors", Json::Int(counts.errors as i64)),
                ("warnings", Json::Int(counts.warnings as i64)),
                ("infos", Json::Int(counts.infos as i64)),
            ]),
        ),
        (
            "lockfile",
            Json::Obj(vec![
                ("path", Json::Str(report.lockfile.path.clone())),
                ("present", Json::Bool(report.lockfile.present)),
                ("locked", Json::Bool(report.lockfile.locked)),
                ("lockfileVersion", opt_int(report.lockfile.version)),
                (
                    "projectHashMatches",
                    opt_bool(report.lockfile.project_hash_matches),
                ),
            ]),
        ),
        ("dependencies", dependencies(report)),
        ("packages", packages(report)),
        ("sourceFlow", source_flow(report)),
        ("resources", resources(report)),
        ("nativeLinks", native_links(report)),
        ("permissions", permissions(report)),
        ("findings", findings(report)),
    ]);

    let mut out = String::new();
    root.write(&mut out, 0);
    out.push('\n');
    out
}

fn dependencies(report: &AuditReport) -> Json {
    Json::Arr(
        report
            .dependencies
            .iter()
            .map(|dependency| {
                Json::Obj(vec![
                    ("name", Json::Str(dependency.name.clone())),
                    ("ident", Json::Str(dependency.ident.clone())),
                    (
                        "requestedVersion",
                        Json::Str(dependency.requested_version.clone()),
                    ),
                    ("resolvedVersion", opt_str(&dependency.resolved_version)),
                    ("pin", Json::Bool(dependency.pin)),
                    ("source", Json::Str(dependency.source.clone())),
                    ("signature", opt_str(&dependency.signature)),
                    ("contentHash", opt_str(&dependency.content_hash)),
                    ("status", Json::Str(dependency.status.clone())),
                ])
            })
            .collect(),
    )
}

fn packages(report: &AuditReport) -> Json {
    Json::Arr(
        report
            .packages
            .iter()
            .map(|package| {
                Json::Obj(vec![
                    ("name", Json::Str(package.name.clone())),
                    ("version", Json::Str(package.version.clone())),
                    ("path", Json::Str(package.path.clone())),
                    ("verifier", Json::Str(package.verifier.clone())),
                    ("signature", Json::Str(package.signature.clone())),
                    ("contentHash", Json::Str(package.content_hash.clone())),
                    ("exports", Json::Int(package.exports as i64)),
                    ("imports", Json::Int(package.imports as i64)),
                    ("cleanups", Json::Int(package.cleanups as i64)),
                ])
            })
            .collect(),
    )
}

fn source_flow(report: &AuditReport) -> Json {
    Json::Arr(
        report
            .source_flow
            .iter()
            .map(|function| {
                let trap = match &function.trap {
                    Some(trap) => Json::Obj(vec![
                        ("name", Json::Str(trap.name.clone())),
                        ("line", Json::Int(trap.line as i64)),
                        ("classification", Json::Str(trap.classification.clone())),
                    ]),
                    None => Json::Null,
                };
                let calls = Json::Arr(
                    function
                        .calls
                        .iter()
                        .map(|call| {
                            Json::Obj(vec![
                                ("callee", Json::Str(call.callee.clone())),
                                ("line", Json::Int(call.line as i64)),
                                ("propagation", Json::Str(call.propagation.clone())),
                                ("capability", opt_str(&call.capability)),
                            ])
                        })
                        .collect(),
                );
                Json::Obj(vec![
                    ("function", Json::Str(function.function.clone())),
                    ("path", Json::Str(function.path.clone())),
                    ("line", Json::Int(function.line as i64)),
                    ("fallible", Json::Bool(function.fallible)),
                    ("trap", trap),
                    ("calls", calls),
                ])
            })
            .collect(),
    )
}

fn resources(report: &AuditReport) -> Json {
    Json::Arr(
        report
            .resources
            .iter()
            .map(|resource| {
                Json::Obj(vec![
                    ("function", Json::Str(resource.function.clone())),
                    ("name", Json::Str(resource.name.clone())),
                    ("resourceType", Json::Str(resource.resource_type.clone())),
                    ("closeOp", Json::Str(resource.close_op.clone())),
                    ("path", Json::Str(resource.path.clone())),
                    ("line", Json::Int(resource.line as i64)),
                    ("native", Json::Bool(resource.native)),
                    ("closeMayFail", Json::Bool(resource.close_may_fail)),
                ])
            })
            .collect(),
    )
}

fn native_links(report: &AuditReport) -> Json {
    Json::Arr(
        report
            .native_links
            .iter()
            .map(|link| {
                Json::Obj(vec![
                    ("package", Json::Str(link.package.clone())),
                    ("symbol", Json::Str(link.symbol.clone())),
                    ("closeFunction", Json::Str(link.close_function.clone())),
                    ("mayFail", Json::Bool(link.may_fail)),
                ])
            })
            .collect(),
    )
}

fn permissions(report: &AuditReport) -> Json {
    Json::Arr(
        report
            .permissions
            .iter()
            .map(|permission| {
                Json::Obj(vec![
                    ("capability", Json::Str(permission.capability.clone())),
                    ("package", Json::Str(permission.package.clone())),
                    ("function", Json::Str(permission.function.clone())),
                    ("path", Json::Str(permission.path.clone())),
                    ("line", Json::Int(permission.line as i64)),
                    ("kind", Json::Str(permission.kind.clone())),
                ])
            })
            .collect(),
    )
}

fn findings(report: &AuditReport) -> Json {
    Json::Arr(
        report
            .findings
            .iter()
            .map(|finding| {
                let location = match (&finding.path, finding.line) {
                    (Some(path), Some(line)) => Json::Obj(vec![
                        ("path", Json::Str(path.clone())),
                        ("line", Json::Int(line as i64)),
                    ]),
                    (Some(path), None) => Json::Obj(vec![("path", Json::Str(path.clone()))]),
                    _ => Json::Null,
                };
                Json::Obj(vec![
                    ("code", Json::Str(finding.code.clone())),
                    ("severity", Json::Str(finding.severity.as_str().to_string())),
                    ("category", Json::Str(finding.category.clone())),
                    ("message", Json::Str(finding.message.clone())),
                    ("location", location),
                    ("package", opt_str(&finding.package)),
                ])
            })
            .collect(),
    )
}
