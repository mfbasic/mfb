//! Experimental `mfb man2` — render a builtin function's man page directly from
//! the **clean-room registry** (`crate::codegen::registry`) descriptor: a package's
//! `intro`/`desc`, each function's `intro`/`desc`/`example`, and every
//! `Implementation`'s parameters / return type / errors — rather than from the
//! static `src/docs/man/**` Markdown.
//!
//! man2 is registry-generic: it renders any package that has migrated onto the
//! clean-room registry (today `csv`), reading the same fields off every descriptor.
//!
//! `mfb man2 <package> types` renders the package's consolidated *types* page — its
//! exported records (with field tables), unions and enums (with their variants), and
//! resources (opaque handles, shown with a description) — the man2 analogue of the
//! old `mfb man <package> types` record-type page.

use std::io::IsTerminal;

use crate::cli::spec::detect_terminal_width;
use crate::codegen::registry::{
    self, registry, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::docs::render;

pub(crate) fn show_man2(args: &[String]) -> Result<(), String> {
    let mut all = false;
    let mut positional: Vec<&str> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--all" => all = true,
            other if other.starts_with("--") => {
                return Err(format!("unknown option `{other}`"));
            }
            other => positional.push(other),
        }
    }

    match positional.as_slice() {
        [] => {
            if all {
                print_markdown(&render_all_markdown());
                Ok(())
            } else {
                Err("Usage: mfb man2 <package> [function] [--all]".to_string())
            }
        }
        [package] => {
            let package = lookup_package(package)?;
            if all {
                print_markdown(&render_package_all_markdown(package));
            } else {
                print_markdown(&render_package_markdown(package));
            }
            Ok(())
        }
        [package, function_name] => {
            if all {
                return Err("mfb man2 --all cannot be combined with a function".to_string());
            }
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
        [_, _, _, ..] => Err("Usage: mfb man2 <package> [function] [--all]".to_string()),
    }
}

/// A full-width horizontal rule matching the separators `mfb man --all` uses.
fn man2_rule() -> String {
    format!("\n\n{}\n\n", "─".repeat(detect_terminal_width()))
}

/// `mfb man2 --all`: the whole registry manual — every package overview followed by
/// all of its function pages, in registration order, as one document.
fn render_all_markdown() -> String {
    let mut md = String::new();
    // `unqualified_global` packages (`testing`, and later `general`) are bare-name
    // builtins with no writable `IMPORT <pkg>` spelling, so rendering a `# testing` /
    // `testing::expect` page would advertise a spelling users cannot write — skip them.
    for package in registry()
        .packages()
        .iter()
        .filter(|package| !package.is_unqualified_global())
    {
        if !md.is_empty() {
            md.push_str(&man2_rule());
        }
        md.push_str(&render_package_all_markdown(package));
    }
    md
}

/// `mfb man2 <package> --all`: the package overview followed by the full page for
/// every function it documents, each separated by a full-width rule.
fn render_package_all_markdown(package: &RegistryPackage) -> String {
    let mut md = render_package_markdown(package);
    for function in package.functions() {
        md.push_str(&man2_rule());
        md.push_str(&render_function_markdown(package, function));
    }
    md
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

    let records: Vec<_> = package.records().iter().filter(|r| r.export).collect();
    if !records.is_empty() {
        md.push_str("### Records\n\n");
        for record in records {
            md.push_str(&format!("#### {pkg}::{}\n\n", record.name));
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
    }

    let unions: Vec<_> = package.unions().iter().filter(|u| u.export).collect();
    if !unions.is_empty() {
        md.push_str("### Unions\n\n");
        for union in unions {
            md.push_str(&format!("#### {pkg}::{}\n\n", union.name));
            md.push_str("A union of:\n\n");
            for variant in &union.variants {
                md.push_str(&format!("- `{}` — {}\n", variant.name, variant.description));
            }
            md.push('\n');
        }
    }

    let enums: Vec<_> = package.enums().iter().filter(|e| e.export).collect();
    if !enums.is_empty() {
        md.push_str("### Enums\n\n");
        for r#enum in enums {
            md.push_str(&format!("#### {pkg}::{}\n\n", r#enum.name));
            md.push_str("An enum of:\n\n");
            for variant in &r#enum.variants {
                md.push_str(&format!("- `{}` — {}\n", variant.name, variant.description));
            }
            md.push('\n');
        }
    }

    let resources: Vec<_> = package.resources().iter().filter(|r| r.export).collect();
    if !resources.is_empty() {
        md.push_str("### Resources\n\n");
        for resource in resources {
            md.push_str(&format!("#### {pkg}::{}\n\n", resource.name));
            md.push_str(resource.description);
            md.push_str("\n\n");
        }
    }

    md
}

/// The MFBASIC declaration of one overload — `pkg::name(p AS Type, [opt AS Type]) AS Return`
/// (optional/defaulted parameters are bracketed). Matches the hand-written man pages'
/// `## Overloads` convention.
fn render_declaration(pkg: &str, name: &str, implementation: &Implementation) -> String {
    let params = implementation
        .params
        .iter()
        .map(|param| {
            let decl = format!("{} AS {}", param.name, param.ty.name());
            if matches!(
                param.default,
                DefaultValue::Fill { .. } | DefaultValue::Optional
            ) {
                format!("[{decl}]")
            } else {
                decl
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "`{pkg}::{name}({params}) AS {}`",
        implementation.return_type.name()
    )
}

/// The union of every parameter across all overloads, de-duplicated by name (first
/// occurrence wins, declaration order preserved), so the Parameters table covers every
/// argument any overload accepts.
fn union_parameters(function: &RegistryFunction) -> Vec<&Parameter> {
    let mut seen = std::collections::HashSet::new();
    let mut params = Vec::new();
    for implementation in &function.implementations {
        for param in &implementation.params {
            if seen.insert(param.name) {
                params.push(param);
            }
        }
    }
    params
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

    let pkg = package.import_name();
    if function.implementations.len() > 1 {
        md.push_str("## Overloads\n\n");
        for implementation in &function.implementations {
            md.push_str(&format!(
                "**{}**\n\n",
                render_declaration(pkg, function.name, implementation)
            ));
        }
    } else if let Some(implementation) = function.implementations.first() {
        md.push_str("## Declaration\n\n");
        md.push_str(&format!(
            "**{}**\n\n",
            render_declaration(pkg, function.name, implementation)
        ));
    }

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
    // The Parameters table is the UNION of every overload's parameters (§man2). The
    // return type(s) are shown in the Overloads/Declaration section above; a single
    // trailing "Returns" line is added only when there is exactly one overload (with
    // multiple overloads the returns can differ, so one line would be misleading).
    let params = union_parameters(function);
    let single = function.implementations.len() == 1;
    let return_type = function
        .implementations
        .first()
        .map(|implementation| implementation.return_type.name());

    if params.is_empty() {
        if single {
            if let Some(return_type) = &return_type {
                md.push_str(&format!(
                    "Takes no arguments and returns `{return_type}`.\n\n"
                ));
            }
        }
        return;
    }

    let has_aliases = params.iter().any(|p| !p.aliases.is_empty());

    md.push_str("## Parameters\n\n");
    if has_aliases {
        md.push_str("| Parameter | Type | Alternate | Description |\n| --- | --- | --- | --- |\n");
    } else {
        md.push_str("| Parameter | Type | Description |\n| --- | --- | --- |\n");
    }

    for param in &params {
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

    if single {
        if let Some(return_type) = &return_type {
            md.push_str(&format!("Returns `{return_type}`.\n\n"));
        }
    }
}

/// Render an Errors table for a set of `errorCode` names, resolving each to its
/// `(code, message)` and ordering by code. No-op when `names` is empty.
fn render_errors_table(md: &mut String, names: &[&'static str]) {
    if names.is_empty() {
        return;
    }
    let mut ordered = names.to_vec();
    ordered.sort_by_key(|name| {
        registry::runtime_error(name)
            .map(|(code, _)| code)
            .unwrap_or("")
    });

    md.push_str("## Errors\n\n");
    md.push_str("| Code | Name | Message |\n| --- | --- | --- |\n");
    for name in ordered {
        let (code, message) = registry::runtime_error(name).unwrap_or(("", ""));
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
        // A single-overload member renders a Declaration section above Parameters.
        assert!(md.contains("## Declaration"));
        assert!(md.contains("`csv::parse("));
        assert!(md.contains(") AS List OF List OF String`"));
        assert!(!md.contains("## Overloads"));
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
    fn multi_overload_function_renders_overloads_and_union_parameters() {
        let package = registry().resolve_package("process").unwrap();
        let function = package.function("spawn").unwrap();
        assert!(function.implementations.len() > 1);
        let md = render_function_markdown(package, function);
        // Multiple overloads → an Overloads section with one declaration per implementation,
        // and NOT the single-overload Declaration section.
        assert!(md.contains("## Overloads"));
        assert!(!md.contains("## Declaration"));
        assert!(md.contains("`process::spawn(args AS List OF String) AS"));
        assert!(md.contains("cwd AS String"));
        assert!(md.contains("envReplace AS Boolean"));
        // The Parameters table is the UNION of every overload's parameters.
        assert!(md.contains("## Parameters"));
        assert!(md.contains("`args`"));
        assert!(md.contains("`cwd`"));
        assert!(md.contains("`env`"));
        assert!(md.contains("`envReplace`"));
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
        // Category headings group the types; individual types render one level deeper.
        assert!(md.contains("### Records"));
        assert!(md.contains("#### json::JsonObj"));
        assert!(md.contains("| Field | Type | Description |"));
        // The exported union renders under a Unions heading with its variants.
        assert!(md.contains("### Unions"));
        assert!(md.contains("#### json::Json"));
        assert!(md.contains("- `JsonNull`"));
        // json has no exported enums/resources, so those headings are excluded.
        assert!(!md.contains("### Enums"));
        assert!(!md.contains("### Resources"));
        // Internal (non-export) records are omitted from the public page.
        assert!(!md.contains("__json_Node"));
        assert!(!md.contains("__json_StringNode"));
    }

    #[test]
    fn types_page_renders_a_record_only_package() {
        let package = registry().resolve_package("csv").unwrap();
        let md = render_types_markdown(package);
        assert!(md.contains("### Records"));
        assert!(md.contains("#### csv::CsvReader"));
        assert!(md.contains("#### csv::CsvRow"));
        // A record-only package excludes the other category headings.
        assert!(!md.contains("### Unions"));
        assert!(!md.contains("### Enums"));
        assert!(!md.contains("### Resources"));
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
        assert!(md.contains("### Enums"));
        assert!(md.contains("#### demo::Stream"));
        assert!(md.contains("- `StdOut` — standard output"));
        // A resource renders as its name + description only (no close op leaks out).
        assert!(md.contains("### Resources"));
        assert!(md.contains("#### demo::Handle"));
        assert!(md.contains("An opaque demo handle."));
        // No exported records/unions → those headings are excluded.
        assert!(!md.contains("### Records"));
        assert!(!md.contains("### Unions"));
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
    fn all_renders_the_whole_registry_manual() {
        assert!(show_man2(&s(&["--all"])).is_ok());
        let md = render_all_markdown();
        // Every package overview appears, each with its functions expanded.
        assert!(md.contains("# csv\n"));
        assert!(md.contains("# parse\n"));
    }

    #[test]
    fn all_renders_one_package_in_full() {
        assert!(show_man2(&s(&["csv", "--all"])).is_ok());
        let package = registry().resolve_package("csv").unwrap();
        let md = render_package_all_markdown(package);
        // The package overview followed by each function's full page.
        assert!(md.starts_with("# csv\n"));
        assert!(md.contains("# parse\n"));
        assert!(md.contains("# stringify\n"));
    }

    #[test]
    fn all_rejects_a_function_argument() {
        let err = show_man2(&s(&["csv", "parse", "--all"])).unwrap_err();
        assert!(err.contains("--all cannot be combined with a function"));
    }

    #[test]
    fn rejects_unknown_option() {
        let err = show_man2(&s(&["--bogus"])).unwrap_err();
        assert!(err.contains("unknown option"));
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
