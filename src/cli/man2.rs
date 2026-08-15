//! Experimental `mfb man2` — render a builtin function's man page directly from
//! the **clean-room registry** (`crate::codegen::registry`) descriptor: a package's
//! `intro`/`desc`, each function's `intro`/`desc`/`example`, and every
//! `Implementation`'s parameters / return type / errors — rather than from the
//! static `src/docs/man/**` Markdown or the legacy `target::shared::registry`.
//!
//! man2 is registry-generic: it renders any package that has migrated onto the
//! clean-room registry (today `csv`), reading the same fields off every descriptor.
//!
//! `mfb man2 <package> types` renders the package's consolidated *types* page — its
//! exported records (with field tables), unions and enums (with their variants), and
//! resources (opaque handles, shown with a description) — the man2 analogue of the
//! old `mfb man <package> types` record-type page.

use std::io::IsTerminal;

use crate::builtins::errorcode;
use crate::cli::spec::detect_terminal_width;
use crate::codegen::registry::{registry, DefaultValue, RegistryFunction, RegistryPackage};
use crate::docs::render;

pub(crate) fn show_man2(args: &[String]) -> Result<(), String> {
    let positional: Vec<&str> = args.iter().map(String::as_str).collect();
    match positional.as_slice() {
        [package, function_name] => {
            let package = lookup_package(package)?;
            // `types` is a reserved page name (matching the old `mfb man <pkg> types`),
            // intercepted only for packages that actually declare a public record or
            // union; otherwise it falls through to the normal function lookup below.
            if *function_name == "types" && has_public_types(package) {
                print_markdown(&render_types_markdown(package));
                return Ok(());
            }
            let function = package.function(function_name).ok_or_else(|| {
                format!(
                    "unknown {} function `{function_name}`\n\nRun `mfb man2 {}` to list functions.",
                    package.import_name(),
                    package.import_name(),
                )
            })?;
            print_markdown(&render_function_markdown(package, function));
            Ok(())
        }
        [package] => {
            let package = lookup_package(package)?;
            print_markdown(&render_package_markdown(package));
            Ok(())
        }
        [] | [_, _, _, ..] => Err("Usage: mfb man2 <package> [function]".to_string()),
    }
}

/// Resolve a package name to its clean-room descriptor, or a user-facing error.
fn lookup_package(package: &str) -> Result<&'static RegistryPackage, String> {
    registry()
        .resolve_package(package)
        .ok_or_else(|| format!("mfb man2: unknown package `{package}`"))
}

/// The union of every error any of a function's implementations declares, in first-
/// seen order.
fn function_errors(function: &RegistryFunction) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    for implementation in &function.implementations {
        for &name in &implementation.errors {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// Build a package-overview Markdown page: `intro` summary, `desc` description, a
/// listing of every member with its own `intro`, and the union of every declared
/// error.
fn render_package_markdown(package: &RegistryPackage) -> String {
    let mut md = String::new();

    md.push_str(&format!("# {}\n\n", package.import_name()));
    if !package.intro().is_empty() {
        md.push_str(package.intro());
        md.push_str("\n\n");
    }
    if !package.desc().is_empty() {
        md.push_str("## Description\n\n");
        md.push_str(package.desc());
        md.push_str("\n\n");
    }

    if !package.functions().is_empty() {
        md.push_str("## Functions\n\n");
        md.push_str("| Function | Summary |\n| --- | --- |\n");
        for function in package.functions() {
            md.push_str(&format!(
                "| `{}::{}` | {} |\n",
                package.import_name(),
                function.name,
                function.intro,
            ));
        }
        md.push('\n');
    }

    let mut names: Vec<&'static str> = Vec::new();
    for function in package.functions() {
        for name in function_errors(function) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    render_errors_table(&mut md, &names);

    md
}

/// Whether the package declares at least one *exported* record, union, enum, or
/// resource — i.e. it has a public types page worth rendering.
fn has_public_types(package: &RegistryPackage) -> bool {
    package.records().iter().any(|record| record.export)
        || package.unions().iter().any(|union| union.export)
        || package.enums().iter().any(|r#enum| r#enum.export)
        || package.resources().iter().any(|resource| resource.export)
}

/// Build the consolidated *types* page for a package: every exported record (with a
/// field table), union (with its variants), enum (with its variants), and resource
/// (an opaque handle, shown with its description), each under a `pkg::Name` heading.
/// Internal (non-`export`) types are omitted — they are not part of the package's
/// public surface.
fn render_types_markdown(package: &RegistryPackage) -> String {
    let mut md = String::new();
    let pkg = package.import_name();

    md.push_str("# Types\n\n");
    md.push_str(&format!("The `{pkg}` package types.\n\n"));

    md.push_str("## Package\n\n");
    md.push_str(pkg);
    md.push_str("\n\n");

    md.push_str("## Types\n\n");

    for record in package.records().iter().filter(|record| record.export) {
        md.push_str(&format!("### {pkg}::{}\n\n", record.name));
        md.push_str("| Field | Type | Description |\n| --- | --- | --- |\n");
        for prop in &record.props {
            md.push_str(&format!(
                "| `{}` | `{}` | {} |\n",
                prop.name,
                prop.ty.name(),
                prop.description,
            ));
        }
        md.push('\n');
    }

    for union in package.unions().iter().filter(|union| union.export) {
        md.push_str(&format!("### {pkg}::{}\n\n", union.name));
        md.push_str("A union of:\n\n");
        for variant in &union.variants {
            md.push_str(&format!("- `{}` — {}\n", variant.name, variant.description));
        }
        md.push('\n');
    }

    for r#enum in package.enums().iter().filter(|r#enum| r#enum.export) {
        md.push_str(&format!("### {pkg}::{}\n\n", r#enum.name));
        md.push_str("An enum of:\n\n");
        for variant in &r#enum.variants {
            md.push_str(&format!("- `{}` — {}\n", variant.name, variant.description));
        }
        md.push('\n');
    }

    for resource in package
        .resources()
        .iter()
        .filter(|resource| resource.export)
    {
        md.push_str(&format!("### {pkg}::{}\n\n", resource.name));
        md.push_str(resource.description);
        md.push_str("\n\n");
    }

    md
}

/// Build a Markdown man page for one function purely from its descriptor.
fn render_function_markdown(package: &RegistryPackage, function: &RegistryFunction) -> String {
    let mut md = String::new();

    md.push_str(&format!("# {}\n\n", function.name));
    if !function.intro.is_empty() {
        md.push_str(function.intro);
        md.push_str("\n\n");
    }

    md.push_str("## Package\n\n");
    md.push_str(package.import_name());
    md.push_str("\n\n");

    render_parameters(&mut md, function);

    if !function.desc.is_empty() {
        md.push_str("## Description\n\n");
        md.push_str(function.desc);
        md.push_str("\n\n");
    }

    render_errors_table(&mut md, &function_errors(function));

    if !function.example.is_empty() {
        md.push_str("## Examples\n\n");
        md.push_str(function.example);
        md.push_str("\n\n");
    }

    render_see_also(&mut md, package, function);

    md
}

/// Collect every `package::function` reference that appears in the Description and
/// list it under "See also", excluding the current member and collapsing duplicates.
fn render_see_also(md: &mut String, package: &RegistryPackage, function: &RegistryFunction) {
    let current = format!("{}::{}", package.import_name(), function.name);
    let referenced = referenced_functions(function.desc, &current);
    if referenced.is_empty() {
        return;
    }
    md.push_str("## See also\n\n");
    for reference in referenced {
        md.push_str(&format!("- `{reference}`\n"));
    }
    md.push('\n');
}

/// Scan `text` for `package::function` references and return the unique set, in
/// sorted order, excluding `current`. A reference is an identifier run, `::`, and
/// another identifier run; identifier characters are ASCII alphanumerics and `_`.
fn referenced_functions(text: &str, current: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut refs: Vec<String> = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] != b':' || bytes[i + 1] != b':' {
            i += 1;
            continue;
        }
        let mut start = i;
        while start > 0 && is_ident(bytes[start - 1]) {
            start -= 1;
        }
        let mut end = i + 2;
        while end < bytes.len() && is_ident(bytes[end]) {
            end += 1;
        }
        if start < i && end > i + 2 {
            let reference = format!("{}::{}", &text[start..i], &text[i + 2..end]);
            if reference != current && !refs.contains(&reference) {
                refs.push(reference);
            }
        }
        i = end.max(i + 2);
    }
    refs.sort();
    refs
}

/// The parameter table, taken from the first implementation's parameters. Optional
/// (defaulted) parameters are flagged, alias spellings are listed if present, and the fixed
/// return type is shown.
fn render_parameters(md: &mut String, function: &RegistryFunction) {
    let Some(implementation) = function.implementations.first() else {
        return;
    };
    if implementation.params.is_empty() {
        md.push_str(&format!(
            "Takes no arguments and returns `{}`.\n\n",
            implementation.return_type.name(),
        ));
        return;
    }

    let has_aliases = implementation.params.iter().any(|p| !p.aliases.is_empty());

    md.push_str("## Parameters\n\n");
    if has_aliases {
        md.push_str("| Parameter | Type | Alternate | Description |\n| --- | --- | --- | --- |\n");
    } else {
        md.push_str("| Parameter | Type | Description |\n| --- | --- | --- |\n");
    }

    for param in &implementation.params {
        let optional = matches!(
            param.default,
            DefaultValue::Fill { .. } | DefaultValue::Optional
        );
        let name = if optional {
            format!("`{}` (opt)", param.name)
        } else {
            format!("`{}`", param.name)
        };

        if has_aliases {
            let aliases = if param.aliases.is_empty() {
                "—".to_string()
            } else {
                param
                    .aliases
                    .iter()
                    .map(|alias| format!("`{alias}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            md.push_str(&format!(
                "| {name} | `{}` | {aliases} | {} |\n",
                param.ty.name(),
                param.desc
            ));
        } else {
            md.push_str(&format!(
                "| {name} | `{}` | {} |\n",
                param.ty.name(),
                param.desc
            ));
        }
    }
    md.push('\n');

    md.push_str(&format!(
        "Returns `{}`.\n\n",
        implementation.return_type.name()
    ));
}

/// Render an Errors table for a set of `errorCode` names, resolving each to its
/// `(code, message)` and ordering by code. No-op when `names` is empty.
fn render_errors_table(md: &mut String, names: &[&'static str]) {
    if names.is_empty() {
        return;
    }
    let mut ordered = names.to_vec();
    ordered.sort_by_key(|name| {
        errorcode::runtime_error(name)
            .map(|(code, _)| code)
            .unwrap_or("")
    });

    md.push_str("## Errors\n\n");
    md.push_str("| Code | Name | Message |\n| --- | --- | --- |\n");
    for name in ordered {
        let (code, message) = errorcode::runtime_error(name).unwrap_or(("", ""));
        md.push_str(&format!("| `{code}` | `{name}` | {message} |\n"));
    }
    md.push('\n');
}

fn print_markdown(markdown: &str) {
    let style = render::Style {
        width: detect_terminal_width(),
        color: std::io::stdout().is_terminal(),
    };
    println!("{}", render::render(markdown, &style));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn renders_a_csv_function_from_the_clean_room_registry() {
        assert!(show_man2(&s(&["csv", "parse"])).is_ok());
        assert!(show_man2(&s(&["csv", "readRow"])).is_ok());
        assert!(show_man2(&s(&["csv", "stringify"])).is_ok());
    }

    #[test]
    fn markdown_includes_summary_description_and_parameters() {
        let package = registry().resolve_package("csv").unwrap();
        let function = package.function("parse").unwrap();
        let md = render_function_markdown(package, function);
        assert!(md.starts_with("# parse\n"));
        assert!(md.contains("## Package\n\ncsv"));
        assert!(md.contains("## Description"));
        // The parameter table reflects the descriptor's params, aliases, and optionals.
        assert!(md.contains("## Parameters"));
        assert!(md.contains("`value`"));
        assert!(md.contains("`text`")); // alias of `value`
        assert!(md.contains("`delimiter` (opt)")); // Fill-defaulted
        assert!(md.contains("Returns `List OF List OF String`."));
        assert!(md.contains("## Examples"));
        // The Errors table is rendered from the descriptor's declared errors.
        assert!(md.contains("## Errors"));
        assert!(md.contains("`77050003`"));
        assert!(md.contains("`ErrInvalidFormat`"));
    }

    #[test]
    fn a_member_with_no_declared_errors_omits_the_errors_section() {
        let package = registry().resolve_package("csv").unwrap();
        // stringify declares no errors.
        let md = render_function_markdown(package, package.function("stringify").unwrap());
        assert!(!md.contains("## Errors"));
    }

    #[test]
    fn no_function_renders_the_package_overview() {
        let package = registry().resolve_package("csv").unwrap();
        let md = render_package_markdown(package);
        assert!(md.starts_with("# csv\n"));
        assert!(md.contains("## Description"));
        assert!(md.contains("## Functions"));
        assert!(md.contains("| `csv::parse` |"));
        assert!(md.contains("| `csv::readRow` |"));
        assert!(show_man2(&s(&["csv"])).is_ok());
    }

    #[test]
    fn renders_a_types_page_with_records_and_unions() {
        assert!(show_man2(&s(&["json", "types"])).is_ok());
        let package = registry().resolve_package("json").unwrap();
        let md = render_types_markdown(package);
        assert!(md.starts_with("# Types\n"));
        assert!(md.contains("## Package\n\njson"));
        // Exported records render as `pkg::Name` with a field table.
        assert!(md.contains("### json::JsonObj"));
        assert!(md.contains("| Field | Type | Description |"));
        // The exported union renders with its variants.
        assert!(md.contains("### json::Json"));
        assert!(md.contains("- `JsonNull`"));
        // Internal (non-export) records are omitted from the public page.
        assert!(!md.contains("__json_Node"));
        assert!(!md.contains("__json_StringNode"));
    }

    #[test]
    fn types_page_renders_a_record_only_package() {
        let package = registry().resolve_package("csv").unwrap();
        let md = render_types_markdown(package);
        assert!(md.contains("### csv::CsvReader"));
        assert!(md.contains("### csv::CsvRow"));
    }

    #[test]
    fn types_page_lists_enums_and_resources() {
        use crate::builtins::resource::ResourceKind;
        use crate::codegen::registry::{EnumVariant, RegistryEnum, RegistryResource};

        let mut package = RegistryPackage::new("demo", "i", "d");
        package.add_enum(RegistryEnum {
            name: "Stream",
            export: true,
            variants: vec![EnumVariant {
                name: "StdOut",
                description: "standard output",
            }],
        });
        package.add_resource(RegistryResource {
            name: "Handle",
            export: true,
            description: "An opaque demo handle.",
            close_function: "demo.close",
            sendable: true,
            close_may_fail: true,
            kind: ResourceKind::Builtin,
        });

        assert!(has_public_types(&package));
        let md = render_types_markdown(&package);
        assert!(md.contains("### demo::Stream"));
        assert!(md.contains("- `StdOut` — standard output"));
        // A resource renders as its name + description only (no close op leaks out).
        assert!(md.contains("### demo::Handle"));
        assert!(md.contains("An opaque demo handle."));
        assert!(!md.contains("demo.close"));
    }

    #[test]
    fn types_is_a_normal_function_lookup_when_the_package_has_no_public_types() {
        // `encoding` declares no records/unions/enums/resources, so it has no public
        // types page — `types` falls through to a function lookup.
        assert!(!has_public_types(
            registry().resolve_package("encoding").unwrap()
        ));
        assert!(show_man2(&s(&["encoding", "types"]))
            .unwrap_err()
            .contains("unknown encoding function"));
    }

    #[test]
    fn rejects_unknown_package() {
        let err = show_man2(&s(&["definitely-not-a-package"])).unwrap_err();
        assert!(err.contains("unknown package"));
    }

    #[test]
    fn rejects_missing_function() {
        assert!(show_man2(&s(&["csv", "nope"]))
            .unwrap_err()
            .contains("unknown csv function"));
    }

    #[test]
    fn referenced_functions_dedupes_sorts_and_drops_self() {
        let text = "call collections::hasKey then collections::getOr, again \
                    collections::getOr, and List OF T has no ref; self collections::get";
        assert_eq!(
            referenced_functions(text, "collections::get"),
            vec![
                "collections::getOr".to_string(),
                "collections::hasKey".to_string()
            ]
        );
    }
}
