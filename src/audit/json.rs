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
            // Escape everything the text renderer escapes (bug-283 A1).
            // Previously only C0 was covered here, so `--format json` piped to a
            // terminal was still spoofable by a crafted package name via DEL, the
            // C1 controls (U+009B is a one-byte CSI on some terminals) or a bidi
            // override — the exact attack bug-210 closed for the text renderer,
            // while the spec claimed the two escaped the same characters.
            //
            // `\uXXXX` rather than the text renderer's `\u{XXXX}` because this
            // has to stay valid JSON; it is lossless either way. Astral-plane
            // code points are not in the unsafe set, so no surrogate pair
            // encoding is needed here.
            c if (c as u32) < 0x20 || crate::terminal_safe::is_terminal_unsafe(c) => {
                out.push_str(&format!("\\u{:04x}", c as u32))
            }
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
        ("libraries", libraries(report)),
        ("resourceFiles", resource_files(report)),
        ("dependencies", dependencies(report)),
        ("packages", packages(report)),
        ("sourceFlow", source_flow(report)),
        ("resources", resources(report)),
        ("nativeLinks", native_links(report)),
        ("nativeResources", native_resources(report)),
        ("permissions", permissions(report)),
        ("findings", findings(report)),
    ]);

    let mut out = String::new();
    root.write(&mut out, 0);
    out.push('\n');
    out
}

/// The manifest's declared native-library locators (bug-283 A3).
fn libraries(report: &AuditReport) -> Json {
    Json::Arr(
        report
            .libraries
            .iter()
            .map(|library| {
                Json::Obj(vec![
                    ("logical", Json::Str(library.logical.clone())),
                    ("os", Json::Str(library.os.clone())),
                    ("arch", opt_str(&library.arch)),
                    ("libc", opt_str(&library.libc)),
                    ("type", Json::Str(library.lib_type.clone())),
                    ("source", Json::Str(library.source.clone())),
                ])
            })
            .collect(),
    )
}

/// The manifest's `resources` entries (bug-283 A3).
fn resource_files(report: &AuditReport) -> Json {
    Json::Arr(
        report
            .resource_files
            .iter()
            .map(|resource| {
                Json::Obj(vec![
                    ("src", Json::Str(resource.src.clone())),
                    ("dst", Json::Str(resource.dst.clone())),
                ])
            })
            .collect(),
    )
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

fn native_resources(report: &AuditReport) -> Json {
    Json::Arr(
        report
            .native_resources
            .iter()
            .map(|resource| {
                Json::Obj(vec![
                    ("package", Json::Str(resource.package.clone())),
                    ("resourceType", Json::Str(resource.resource_type.clone())),
                    ("closeOp", Json::Str(resource.close_op.clone())),
                    ("closeMayFail", Json::Bool(resource.close_may_fail)),
                    ("threadSendable", Json::Bool(resource.sendable)),
                    ("exported", Json::Bool(resource.exported)),
                    ("kind", Json::Str("native".to_string())),
                    ("path", Json::Str(resource.path.clone())),
                    ("line", Json::Int(resource.line as i64)),
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

#[cfg(test)]
mod tests {
    use super::super::report::testsupport::*;
    use super::super::report::*;
    use super::*;
    use std::collections::HashMap;
    use tinyjson::JsonValue;

    fn parse(text: &str) -> HashMap<String, JsonValue> {
        let value: JsonValue = text.parse().expect("valid json");
        value
            .get::<HashMap<String, JsonValue>>()
            .expect("object root")
            .clone()
    }

    fn obj(value: &JsonValue) -> &HashMap<String, JsonValue> {
        value.get::<HashMap<String, JsonValue>>().expect("object")
    }

    fn arr(value: &JsonValue) -> &Vec<JsonValue> {
        value.get::<Vec<JsonValue>>().expect("array")
    }

    fn s(value: &JsonValue) -> &str {
        value.get::<String>().expect("string")
    }

    #[test]
    fn full_report_roundtrips_and_is_parseable() {
        let out = render(&full_report());
        assert!(out.ends_with('\n'));
        let root = parse(&out);

        assert_eq!(s(&root["schema"]), SCHEMA);
        assert_eq!(s(&obj(&root["tool"])["name"]), "mfb");
        assert_eq!(s(&obj(&root["tool"])["version"]), env!("CARGO_PKG_VERSION"));

        let project = obj(&root["project"]);
        assert_eq!(s(&project["name"]), "demo");
        assert_eq!(s(&project["ident"]), "demo.ident");
        assert_eq!(s(&project["entry"]), "main");
        assert_eq!(s(&project["languageVersion"]), "1");

        let summary = obj(&root["summary"]);
        assert_eq!(*summary["errors"].get::<f64>().unwrap() as i64, 1);
        assert_eq!(*summary["warnings"].get::<f64>().unwrap() as i64, 1);
        assert_eq!(*summary["infos"].get::<f64>().unwrap() as i64, 1);

        let lockfile = obj(&root["lockfile"]);
        assert!(*lockfile["present"].get::<bool>().unwrap());
        assert!(*lockfile["locked"].get::<bool>().unwrap());
        assert_eq!(*lockfile["lockfileVersion"].get::<f64>().unwrap() as i64, 1);
        assert!(!*lockfile["projectHashMatches"].get::<bool>().unwrap());

        let deps = arr(&root["dependencies"]);
        assert_eq!(deps.len(), 2);
        let alpha = obj(&deps[0]);
        assert_eq!(s(&alpha["name"]), "alpha");
        assert_eq!(s(&alpha["resolvedVersion"]), "1.2.3");
        assert!(*alpha["pin"].get::<bool>().unwrap());
        assert_eq!(s(&alpha["signature"]), "signed");
        let beta = obj(&deps[1]);
        // opt_str None -> JSON null
        assert!(matches!(beta["resolvedVersion"], JsonValue::Null));
        assert!(matches!(beta["signature"], JsonValue::Null));
        assert!(matches!(beta["contentHash"], JsonValue::Null));

        let packages = arr(&root["packages"]);
        assert_eq!(s(&obj(&packages[0])["verifier"]), "ok");
        assert_eq!(
            *obj(&packages[0])["exports"].get::<f64>().unwrap() as i64,
            3
        );

        let flow = arr(&root["sourceFlow"]);
        assert_eq!(flow.len(), 2);
        let work = obj(&flow[0]);
        assert!(*work["fallible"].get::<bool>().unwrap());
        let trap = obj(&work["trap"]);
        assert_eq!(s(&trap["classification"]), "recovers");
        let call = obj(&arr(&work["calls"])[0]);
        assert_eq!(s(&call["callee"]), "fs.open");
        assert_eq!(s(&call["capability"]), "filesystem");
        // second flow fn has null trap
        assert!(matches!(obj(&flow[1])["trap"], JsonValue::Null));

        let resources = arr(&root["resources"]);
        assert_eq!(resources.len(), 2);
        assert!(*obj(&resources[0])["closeMayFail"].get::<bool>().unwrap());

        let native_links = arr(&root["nativeLinks"]);
        assert_eq!(s(&obj(&native_links[0])["symbol"]), "sym");

        let native_resources = arr(&root["nativeResources"]);
        assert_eq!(s(&obj(&native_resources[0])["kind"]), "native");
        assert!(*obj(&native_resources[0])["threadSendable"]
            .get::<bool>()
            .unwrap());

        let permissions = arr(&root["permissions"]);
        assert_eq!(permissions.len(), 3);

        let findings = arr(&root["findings"]);
        assert_eq!(findings.len(), 3);
        // finding with path+line -> location object with both
        let loc0 = obj(&obj(&findings[0])["location"]);
        assert_eq!(s(&loc0["path"]), "mfb.lock");
        // AUDIT-LOCK-STALE has path but no line
        assert!(!loc0.contains_key("line"));
        let loc1 = obj(&obj(&findings[1])["location"]);
        assert_eq!(*loc1["line"].get::<f64>().unwrap() as i64, 11);
        // finding with neither path nor line -> null location
        assert!(matches!(obj(&findings[2])["location"], JsonValue::Null));
    }

    #[test]
    fn empty_report_emits_empty_arrays_and_objects() {
        let out = render(&empty_report());
        let root = parse(&out);
        assert!(arr(&root["dependencies"]).is_empty());
        assert!(arr(&root["findings"]).is_empty());
        // an empty entry is null via opt_str
        let project = obj(&root["project"]);
        assert_eq!(s(&project["entry"]), "main");
    }

    #[test]
    fn empty_report_without_entry_serializes_null() {
        let mut report = empty_report();
        report.project.entry = None;
        let out = render(&report);
        let root = parse(&out);
        assert!(matches!(obj(&root["project"])["entry"], JsonValue::Null));
    }

    #[test]
    fn write_string_escapes_control_and_special_characters() {
        let mut out = String::new();
        write_string(&mut out, "a\"b\\c\nd\re\tf\u{0001}g");
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\\re\\tf\\u0001g\"");
    }

    #[test]
    fn json_escaping_survives_roundtrip_in_message() {
        let mut report = empty_report();
        report.findings.push(Finding {
            code: "X".to_string(),
            category: "lint".to_string(),
            severity: Severity::Info,
            message: "quote\"and\\slash\tand\nnewline".to_string(),
            path: None,
            line: None,
            package: None,
        });
        let out = render(&report);
        let root = parse(&out);
        let finding = obj(&arr(&root["findings"])[0]);
        assert_eq!(s(&finding["message"]), "quote\"and\\slash\tand\nnewline");
    }

    #[test]
    fn opt_helpers_map_none_to_null_and_some_to_value() {
        let mut int_holder = String::new();
        Json::Null.write(&mut int_holder, 0);
        assert_eq!(int_holder, "null");

        // opt_int / opt_bool via a small object
        let obj = Json::Obj(vec![
            ("a", opt_int(Some(7))),
            ("b", opt_int(None)),
            ("c", opt_bool(Some(true))),
            ("d", opt_bool(None)),
            ("e", opt_str(&Some("hi".to_string()))),
        ]);
        let mut out = String::new();
        obj.write(&mut out, 0);
        let parsed = parse(&out);
        assert_eq!(*parsed["a"].get::<f64>().unwrap() as i64, 7);
        assert!(matches!(parsed["b"], JsonValue::Null));
        assert!(*parsed["c"].get::<bool>().unwrap());
        assert!(matches!(parsed["d"], JsonValue::Null));
        assert_eq!(parsed["e"].get::<String>().unwrap(), "hi");
    }

    /// The JSON renderer escapes the same characters the text renderer does
    /// (bug-283 A1).
    ///
    /// bug-210 hardened the text output against a crafted package name spoofing
    /// the terminal, but the JSON writer escaped only C0, so `--format json`
    /// piped to a terminal stayed vulnerable to DEL, the C1 controls (U+009B is
    /// a one-byte CSI on some terminals) and bidi overrides -- while the spec
    /// claimed parity. Escaping is `\uXXXX` here rather than the text
    /// renderer's `\u{XXXX}` so the output stays valid JSON.
    #[test]
    fn write_string_escapes_the_terminal_unsafe_set() {
        for (raw, needle) in [
            ("a\u{1b}[31mb", "\\u001b"),   // C0 ESC
            ("a\u{7f}b", "\\u007f"),       // DEL
            ("a\u{9b}b", "\\u009b"),       // C1 CSI
            ("a\u{202e}b", "\\u202e"),     // RLO bidi override
            ("a\u{feff}b", "\\ufeff"),     // BOM / zero-width no-break space
        ] {
            let mut out = String::new();
            write_string(&mut out, raw);
            assert!(
                out.contains(needle),
                "expected {needle} in {out} for input {raw:?}"
            );
            // Still valid JSON, and still parses back to the original text.
            let parsed = parse(&format!("{{\"v\": {out}}}"));
            assert_eq!(parsed["v"].get::<String>().unwrap(), raw);
        }

        // Ordinary text is untouched, including non-ASCII that is not a control
        // or a bidi override.
        let mut out = String::new();
        write_string(&mut out, "caf\u{e9} \u{65e5}");
        assert_eq!(out, "\"caf\u{e9} \u{65e5}\"");
    }
}
