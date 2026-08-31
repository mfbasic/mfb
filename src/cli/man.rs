//! `mfb man` — render a builtin package or function's man page directly from the
//! **clean-room registry** (`crate::codegen::registry`) descriptor: a package's
//! `intro`/`desc`, each function's `intro`/`desc`/`example`, and every
//! `Implementation`'s parameters / return type / errors. This replaced the legacy
//! renderer that read the static `src/docs/man/**` Markdown (that tree was retired
//! to `planning/old_man`).
//!
//! man is registry-generic: it renders every package off its registry descriptor,
//! reading the same fields for each.
//!
//! `mfb man <package> types` renders the package's consolidated *types* page — its
//! exported records (with field tables), unions and enums (with their variants), and
//! resources (opaque handles, shown with a description) — the man analogue of the
//! old `mfb man <package> types` record-type page.

use std::io::IsTerminal;

use crate::cli::spec::detect_terminal_width;
use crate::codegen::registry::{
    self, registry, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::docs::man::{self, ManTopic};
use crate::docs::render;

pub(crate) fn show_man(args: &[String]) -> Result<(), String> {
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
            } else {
                print_markdown(&render_index_markdown());
            }
            Ok(())
        }
        [name] => {
            // A real built-in package wins; otherwise fall back to the prose guide
            // topics (`errors`, `flow`, `types`, ...), so existing `mfb man <package>`
            // behavior is untouched and the topics only fill names no package claims.
            if let Some(package) = registry().resolve_package(name) {
                if all {
                    print_markdown(&render_package_all_markdown(package));
                } else {
                    print_markdown(&render_package_markdown(package));
                }
                Ok(())
            } else if let Some(topic) = man::topic(name) {
                if all {
                    print_markdown(&render_topic_all_markdown(topic));
                } else {
                    print_markdown(&render_topic_overview(topic));
                }
                Ok(())
            } else {
                Err(unknown_topic_error(name))
            }
        }
        [name, page_name] => {
            if all {
                return Err("mfb man --all cannot be combined with a function".to_string());
            }
            if let Some(package) = registry().resolve_package(name) {
                // `types` is a reserved page name (matching the old `mfb man <pkg> types`).
                // A package with public types renders its consolidated types page; a
                // function literally named `types` still wins the lookup below when the
                // package has none; otherwise a friendly no-types message is the answer.
                if *page_name == "types" && has_public_types(package) {
                    print_markdown(&render_types_markdown(package));
                    return Ok(());
                }
                // Internal-only members (`astrings::readSpans`, …) are not user-
                // callable, so they get no man page — an unknown-function error is
                // the truthful answer.
                let function = package
                    .function(page_name)
                    .filter(|function| !function.internal_only);
                if *page_name == "types" && function.is_none() {
                    print_markdown(&format!(
                        "The `{}` package has no public types.\n\nRun `mfb man {}` to list its functions.\n",
                        package.import_name(),
                        package.import_name(),
                    ));
                    return Ok(());
                }
                let function = function.ok_or_else(|| {
                    format!(
                        "unknown {} function `{page_name}`\n\nRun `mfb man {}` to list functions.",
                        package.import_name(),
                        package.import_name(),
                    )
                })?;
                print_markdown(&render_function_markdown(package, function));
                Ok(())
            } else if let Some(topic) = man::topic(name) {
                let page = man::page(topic, page_name).ok_or_else(|| {
                    format!(
                        "unknown {name} topic page `{page_name}`\n\nRun `mfb man {name}` to list pages.",
                    )
                })?;
                print_markdown(page.page);
                Ok(())
            } else {
                Err(unknown_topic_error(name))
            }
        }
        [_, _, _, ..] => Err("Usage: mfb man <package> [function] [--all]".to_string()),
    }
}

/// A full-width horizontal rule matching the separators `mfb man --all` uses.
fn man_rule() -> String {
    format!("\n\n{}\n\n", "─".repeat(detect_terminal_width()))
}

/// `mfb man --all`: the whole registry manual — every package overview followed by
/// all of its function pages and its `types` page, in registration order, as one
/// document.
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
            md.push_str(&man_rule());
        }
        md.push_str(&render_package_all_markdown(package));
    }
    md
}

/// Bare `mfb man`: a friendly index — every builtin package (sorted, with its
/// one-line intro), then every non-builtin guide topic (sorted, with the summary
/// line from its overview).
fn render_index_markdown() -> String {
    let mut md = String::new();
    md.push_str("# mfb man\n\nThe MFBASIC reference manual.\n\n");

    md.push_str("## Builtin packages\n\n");
    md.push_str("| Package | Summary |\n| --- | --- |\n");
    // `unqualified_global` packages have no writable `IMPORT <pkg>` spelling and
    // are skipped here for the same reason `mfb man --all` skips them.
    let mut packages: Vec<_> = registry()
        .packages()
        .iter()
        .filter(|package| !package.is_unqualified_global())
        .collect();
    packages.sort_by_key(|package| package.import_name());
    for package in packages {
        md.push_str(&format!(
            "| `{}` | {} |\n",
            package.import_name(),
            package.intro(),
        ));
    }
    md.push('\n');

    md.push_str("## Guide topics\n\n");
    md.push_str("| Topic | Summary |\n| --- | --- |\n");
    let mut topics: Vec<_> = man::topics().iter().collect();
    topics.sort_by_key(|topic| topic.name);
    for topic in topics {
        md.push_str(&format!(
            "| `{}` | {} |\n",
            topic.name,
            topic_summary(topic)
        ));
    }
    md.push('\n');

    md.push_str(
        "Run `mfb man <package>` for a package overview, `mfb man <package> <function>` \
         for one function, `mfb man <package> types` for a package's types, or \
         `mfb man <topic>` for a guide.\n",
    );
    md
}

/// A guide topic's one-line summary: the first non-empty, non-heading line of its
/// overview (the line right under the `# <name>` title).
fn topic_summary(topic: &ManTopic) -> &'static str {
    topic
        .overview
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("")
}

/// `mfb man <package> --all`: the package overview, the full page for every
/// function it documents, and — when the package exposes public types — its
/// consolidated `types` page, each separated by a full-width rule.
fn render_package_all_markdown(package: &RegistryPackage) -> String {
    let mut md = render_package_markdown(package);
    // Internal-only members are not user-callable and render no page. Pages come
    // in name order, matching the overview's sorted Functions table.
    let mut functions: Vec<_> = package
        .functions()
        .iter()
        .filter(|f| !f.internal_only)
        .collect();
    functions.sort_by_key(|function| function.name);
    for function in functions {
        md.push_str(&man_rule());
        md.push_str(&render_function_markdown(package, function));
    }
    // The consolidated `mfb man <pkg> types` page, appended so `--all` is a complete
    // rendering of the package's public surface (functions AND types).
    if has_public_types(package) {
        md.push_str(&man_rule());
        md.push_str(&render_types_markdown(package));
    }
    md
}

/// The error for a first positional that names neither a built-in package nor a
/// guide topic. Phrased around "unknown package" since that is the common case.
fn unknown_topic_error(name: &str) -> String {
    format!("mfb man: unknown package `{name}`")
}

/// `mfb man <topic> --all`: a guide topic's overview followed by every one of its
/// sub-pages, each separated by a full-width rule — the topic analogue of
/// `mfb man <package> --all`.
fn render_topic_all_markdown(topic: &ManTopic) -> String {
    let mut md = render_topic_overview(topic);
    for page in &topic.pages {
        md.push_str(&man_rule());
        md.push_str(page.page);
    }
    md
}

/// A guide topic's overview with its generated substitutions applied: the
/// `optimizations` topic's pass table is rendered from the optimizer's own
/// landed-row catalog (`optimizer::catalog`) at display time, so the man page
/// and the compiler can never disagree about which passes exist.
fn render_topic_overview(topic: &ManTopic) -> String {
    const CATALOG_MARKER: &str = "{{optimizer-catalog}}";
    if topic.overview.contains(CATALOG_MARKER) {
        topic.overview.replace(
            CATALOG_MARKER,
            crate::optimizer::catalog::render_markdown_table().trim_end(),
        )
    } else {
        topic.overview.to_string()
    }
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

    // Internal-only members (`astrings::readSpans`, …) are not user-callable and
    // must not be advertised. The listing is sorted by name, not registration order.
    let mut functions: Vec<_> = package
        .functions()
        .iter()
        .filter(|function| !function.internal_only)
        .collect();
    functions.sort_by_key(|function| function.name);
    if !functions.is_empty() {
        md.push_str("## Functions\n\n");
        md.push_str("| Function | Summary |\n| --- | --- |\n");
        for function in &functions {
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

    // Point at the consolidated types page when the package exports any types.
    if has_public_types(package) {
        md.push_str("## See also\n\n");
        md.push_str(&format!(
            "- `mfb man {} types` — the package's types (records, unions, enums, resources)\n\n",
            package.import_name(),
        ));
    }

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

    let records: Vec<_> = package.records().iter().filter(|r| r.export).collect();
    if !records.is_empty() {
        md.push_str("## Records\n\n");
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
        md.push_str("## Unions\n\n");
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
        md.push_str("## Enums\n\n");
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
        md.push_str("## Resources\n\n");
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
            let decl = format!("{} AS {}", param.name, public_type_name(&param.ty));
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
        public_type_name(&implementation.return_type)
    )
}

fn public_type_name(ty: &crate::types::ParameterType) -> String {
    ty.name().replace('.', "::")
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
    // The Parameters table is the UNION of every overload's parameters (§man). The
    // return type(s) are shown in the Overloads/Declaration section above; a single
    // trailing "Returns" line is added only when there is exactly one overload (with
    // multiple overloads the returns can differ, so one line would be misleading).
    let params = union_parameters(function);
    let single = function.implementations.len() == 1;
    let return_type = function
        .implementations
        .first()
        .map(|implementation| public_type_name(&implementation.return_type));

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
                public_type_name(&param.ty),
                param.desc
            ));
        } else {
            md.push_str(&format!(
                "| {name} | `{}` | {} |\n",
                public_type_name(&param.ty),
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
    use std::io::Write;
    let style = render::Style {
        width: detect_terminal_width(),
        color: std::io::stdout().is_terminal(),
    };
    // A closed stdout (`mfb man --all | head`) is a normal way to stop reading,
    // not an error — `println!` would panic on the broken pipe.
    if let Err(err) = writeln!(std::io::stdout(), "{}", render::render(markdown, &style)) {
        if err.kind() != std::io::ErrorKind::BrokenPipe {
            eprintln!("error: failed writing man output: {err}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn renders_a_csv_function_from_the_clean_room_registry() {
        assert!(show_man(&s(&["csv", "parse"])).is_ok());
        assert!(show_man(&s(&["csv", "readRow"])).is_ok());
        assert!(show_man(&s(&["csv", "stringify"])).is_ok());
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
        // Every `csv` member is now fallible (dialect validation raises
        // ErrInvalidFormat), so use a total `bits` op instead: `band` declares no
        // errors.
        let package = registry().resolve_package("bits").unwrap();
        let md = render_function_markdown(package, package.function("band").unwrap());
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
    fn function_types_use_public_package_qualification() {
        // plan-110-E: `poll` moved to `tcp` with the rest of the stream surface.
        let package = registry().resolve_package("tcp").unwrap();
        let function = package.function("poll").unwrap();
        let md = render_function_markdown(package, function);

        assert!(md.contains("sock AS tcp::Socket"));
        // `tcp` declares the list form as `List OF RES tcp::Socket` -- net's
        // original omitted the `RES`, which no source spelling of a resource list
        // may do (§15.6). The qualification this test guards is unaffected.
        assert!(md.contains("socks AS List OF RES tcp::Socket"));
        assert!(md.contains("| `sock` | `tcp::Socket` |"));
        assert!(md.contains("| `socks` | `List OF RES tcp::Socket` |"));
        assert!(!md.contains("tcp.Socket"));
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
        assert!(show_man(&s(&["csv"])).is_ok());
    }

    #[test]
    fn renders_a_types_page_with_records_and_unions() {
        assert!(show_man(&s(&["json", "types"])).is_ok());
        let package = registry().resolve_package("json").unwrap();
        let md = render_types_markdown(package);
        assert!(md.starts_with("# Types\n"));
        assert!(md.contains("## Package\n\njson"));
        // Category headings group the types; individual types render one level deeper.
        assert!(md.contains("## Records"));
        assert!(md.contains("#### json::JsonObj"));
        assert!(md.contains("| Field | Type | Description |"));
        // The exported union renders under a Unions heading with its variants.
        assert!(md.contains("## Unions"));
        assert!(md.contains("#### json::Json"));
        assert!(md.contains("- `JsonNull`"));
        // json has no exported enums/resources, so those headings are excluded.
        assert!(!md.contains("## Enums"));
        assert!(!md.contains("## Resources"));
        // Internal (non-export) records are omitted from the public page.
        assert!(!md.contains("__json_Node"));
        assert!(!md.contains("__json_StringNode"));
    }

    #[test]
    fn types_page_renders_a_record_only_package() {
        let package = registry().resolve_package("csv").unwrap();
        let md = render_types_markdown(package);
        assert!(md.contains("## Records"));
        assert!(md.contains("#### csv::CsvReader"));
        assert!(md.contains("#### csv::CsvRow"));
        // A record-only package excludes the other category headings.
        assert!(!md.contains("## Unions"));
        assert!(!md.contains("## Enums"));
        assert!(!md.contains("## Resources"));
    }

    #[test]
    fn types_page_lists_enums_and_resources() {
        use crate::codegen::registry::{EnumVariant, RegistryEnum, RegistryResource};
        use crate::codegen::resource::ResourceKind;

        let mut package = RegistryPackage::new("demo", "i", "d");
        package.add_enum(RegistryEnum {
            name: "Stream",
            export: true,
            variants: vec![EnumVariant {
                name: "StdOut",
                description: "standard output",
                advisory: None,
            }],
        });
        package.add_resource(RegistryResource {
            name: "Handle",
            export: true,
            description: "An opaque demo handle.",
            close_function: "demo.close",
            sendable: true,
            live_slots: &[],
            close_may_fail: true,
            kind: ResourceKind::Builtin,
        });

        assert!(has_public_types(&package));
        let md = render_types_markdown(&package);
        assert!(md.contains("## Enums"));
        assert!(md.contains("#### demo::Stream"));
        assert!(md.contains("- `StdOut` — standard output"));
        // A resource renders as its name + description only (no close op leaks out).
        assert!(md.contains("## Resources"));
        assert!(md.contains("#### demo::Handle"));
        assert!(md.contains("An opaque demo handle."));
        // No exported records/unions → those headings are excluded.
        assert!(!md.contains("## Records"));
        assert!(!md.contains("## Unions"));
        assert!(!md.contains("demo.close"));
    }

    #[test]
    fn types_shows_a_friendly_message_when_the_package_has_no_public_types() {
        // `encoding` declares no records/unions/enums/resources (and no function
        // named `types`), so `mfb man encoding types` prints the friendly
        // no-public-types message instead of an unknown-function error.
        assert!(!has_public_types(
            registry().resolve_package("encoding").unwrap()
        ));
        assert!(show_man(&s(&["encoding", "types"])).is_ok());
    }

    #[test]
    fn bare_man_renders_the_index_of_packages_and_topics() {
        assert!(show_man(&s(&[])).is_ok());
        let md = render_index_markdown();
        assert!(md.contains("## Builtin packages"));
        assert!(md.contains("| `csv` |"));
        assert!(md.contains("## Guide topics"));
        assert!(md.contains("| `errors` |"));
        // Both listings are sorted by name.
        assert!(md.find("| `bits` |").unwrap() < md.find("| `csv` |").unwrap());
        assert!(md.find("| `errors` |").unwrap() < md.find("| `flow` |").unwrap());
        // `unqualified_global` packages have no importable spelling and are skipped.
        assert!(!md.contains("| `testing` |"));
    }

    #[test]
    fn package_overview_sorts_functions_and_links_its_types_page() {
        let package = registry().resolve_package("csv").unwrap();
        let md = render_package_markdown(package);
        // The Functions table is sorted by name: `parse` before `readRow` before
        // `stringify`.
        let parse = md.find("| `csv::parse` |").unwrap();
        let read_row = md.find("| `csv::readRow` |").unwrap();
        let stringify = md.find("| `csv::stringify` |").unwrap();
        assert!(parse < read_row && read_row < stringify);
        // csv exports records, so the overview points at its types page.
        assert!(md.contains("## See also"));
        assert!(md.contains("`mfb man csv types`"));
        // A package with no public types gets no such pointer.
        let encoding = registry().resolve_package("encoding").unwrap();
        assert!(!render_package_markdown(encoding).contains("## See also"));
    }

    #[test]
    fn rejects_unknown_package() {
        let err = show_man(&s(&["definitely-not-a-package"])).unwrap_err();
        assert!(err.contains("unknown package"));
    }

    #[test]
    fn renders_a_guide_topic_overview() {
        // A first positional that is not a package falls back to a guide topic,
        // rendering its `package.md` overview.
        assert!(show_man(&s(&["errors"])).is_ok());
        assert!(show_man(&s(&["flow"])).is_ok());
        assert!(show_man(&s(&["types"])).is_ok());
    }

    #[test]
    fn renders_a_guide_topic_sub_page() {
        // A second positional under a guide topic selects one of its sub-pages
        // (`flow/for.md`, `tour/01_c.md` -> `c`).
        assert!(show_man(&s(&["flow", "for"])).is_ok());
        assert!(show_man(&s(&["tour", "c"])).is_ok());
    }

    #[test]
    fn rejects_unknown_topic_sub_page() {
        let err = show_man(&s(&["flow", "definitely-not-a-page"])).unwrap_err();
        assert!(err.contains("unknown flow topic page"));
    }

    #[test]
    fn topic_all_renders_the_overview_and_every_sub_page() {
        let topic = man::topic("flow").unwrap();
        let md = render_topic_all_markdown(topic);

        assert!(md.starts_with(topic.overview));
        for page in &topic.pages {
            assert!(md.contains(page.page), "missing flow page `{}`", page.name);
        }
    }

    #[test]
    fn all_renders_the_whole_registry_manual() {
        assert!(show_man(&s(&["--all"])).is_ok());
        let md = render_all_markdown();
        // Every package overview appears, each with its functions expanded.
        assert!(md.contains("# csv\n"));
        assert!(md.contains("# parse\n"));
    }

    #[test]
    fn all_renders_one_package_in_full() {
        assert!(show_man(&s(&["csv", "--all"])).is_ok());
        let package = registry().resolve_package("csv").unwrap();
        let md = render_package_all_markdown(package);
        // The package overview followed by each function's full page.
        assert!(md.starts_with("# csv\n"));
        assert!(md.contains("# parse\n"));
        assert!(md.contains("# stringify\n"));
    }

    #[test]
    fn all_rejects_a_function_argument() {
        let err = show_man(&s(&["csv", "parse", "--all"])).unwrap_err();
        assert!(err.contains("--all cannot be combined with a function"));
    }

    #[test]
    fn rejects_unknown_option() {
        let err = show_man(&s(&["--bogus"])).unwrap_err();
        assert!(err.contains("unknown option"));
    }

    #[test]
    fn rejects_missing_function() {
        assert!(show_man(&s(&["csv", "nope"]))
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
