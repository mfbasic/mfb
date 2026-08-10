//! Experimental `mfb man2` — render a builtin function's man page directly from
//! the descriptor [`REGISTRY`] metadata (`doc_intro` / `doc_desc` / `errors` /
//! overload parameters) rather than from the static `src/docs/man/**` Markdown.
//!
//! This is a testbed for registry-driven documentation and is deliberately wired
//! for the `collections` package only. Once the approach is proven it can widen
//! to every package (every `BuiltinModule` carries the same fields).

use std::io::IsTerminal;

use crate::codegen::registry::{
    BuiltinFunction, BuiltinModule, DefaultValue, ReturnType, REGISTRY,
};
use crate::builtins::errorcode;
use crate::cli::spec::detect_terminal_width;
use crate::docs::render;

pub(crate) fn show_man2(args: &[String]) -> Result<(), String> {
    let positional: Vec<&str> = args.iter().map(String::as_str).collect();
    match positional.as_slice() {
        ["collections", function_name] => {
            let module = REGISTRY
                .module("collections")
                .expect("collections is a registered builtin package");
            let function = lookup(module, function_name).ok_or_else(|| {
                format!(
                    "unknown collections function `{function_name}`\n\nRun `mfb man collections` to list functions."
                )
            })?;
            print_markdown(&render_function_markdown(module, function));
            Ok(())
        }
        ["collections"] => {
            let module = REGISTRY
                .module("collections")
                .expect("collections is a registered builtin package");
            print_markdown(&render_package_markdown(module));
            Ok(())
        }
        [package, ..] => Err(format!(
            "mfb man2 is wired for the `collections` package only (got `{package}`)"
        )),
        [] => Err("Usage: mfb man2 collections <function>".to_string()),
    }
}

/// Resolve a bare function name (`get`) to its descriptor entry, matching either
/// the qualified call name (`collections.get`) or the documentation slug.
fn lookup<'a>(module: &'a BuiltinModule, function_name: &str) -> Option<&'a BuiltinFunction> {
    let qualified = format!("{}.{function_name}", module.name);
    module.function(&qualified).or_else(|| {
        module
            .functions
            .iter()
            .find(|f| f.doc_slug == function_name)
    })
}

/// Build a package-overview Markdown page from the module descriptor: its
/// `doc_intro` summary, `doc_desc` description, a listing of every member with
/// its own `doc_intro`, and the union of every declared error.
fn render_package_markdown(module: &BuiltinModule) -> String {
    let mut md = String::new();

    md.push_str(&format!("# {}\n\n", module.name));
    if !module.doc_intro.is_empty() {
        md.push_str(module.doc_intro);
        md.push_str("\n\n");
    }
    if !module.doc_desc.is_empty() {
        md.push_str("## Description\n\n");
        md.push_str(module.doc_desc);
        md.push_str("\n\n");
    }

    if !module.functions.is_empty() {
        md.push_str("## Functions\n\n");
        md.push_str("| Function | Summary |\n| --- | --- |\n");
        for function in module.functions {
            md.push_str(&format!(
                "| `{}::{}` | {} |\n",
                module.name, function.doc_slug, function.doc_intro
            ));
        }
        md.push('\n');
    }

    render_package_errors(&mut md, module);

    md
}

/// The union of every error declared by any member, ordered by code, resolved to
/// `(code, message)`. The aggregate Errors table for a package overview.
fn render_package_errors(md: &mut String, module: &BuiltinModule) {
    let mut names: Vec<&'static str> = Vec::new();
    for function in module.functions {
        for &name in function.errors {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    if names.is_empty() {
        return;
    }
    names.sort_by_key(|name| {
        errorcode::runtime_error(name)
            .map(|(code, _)| code)
            .unwrap_or("")
    });

    md.push_str("## Errors\n\n");
    md.push_str("| Code | Name | Message |\n| --- | --- | --- |\n");
    for name in names {
        let (code, message) = errorcode::runtime_error(name).unwrap_or(("", ""));
        md.push_str(&format!("| `{code}` | `{name}` | {message} |\n"));
    }
    md.push('\n');
}

/// Build a Markdown man page for one function purely from its descriptor.
fn render_function_markdown(module: &BuiltinModule, function: &BuiltinFunction) -> String {
    let mut md = String::new();

    md.push_str(&format!("# {}\n\n", function.doc_slug));
    if !function.doc_intro.is_empty() {
        md.push_str(function.doc_intro);
        md.push_str("\n\n");
    }

    md.push_str("## Package\n\n");
    md.push_str(module.name);
    md.push_str("\n\n");

    render_parameters(&mut md, function);

    if !function.doc_desc.is_empty() {
        md.push_str("## Description\n\n");
        md.push_str(function.doc_desc);
        md.push_str("\n\n");
    }

    render_errors(&mut md, function);
    render_see_also(&mut md, module, function);

    md
}

/// Collect every `package::function` reference that appears in the Description
/// and list it under "See also". The current function's own qualified name is
/// excluded, and duplicates are collapsed, so a page points only at the *other*
/// members it mentions.
fn render_see_also(md: &mut String, module: &BuiltinModule, function: &BuiltinFunction) {
    let current = format!("{}::{}", module.name, function.doc_slug);
    let referenced = referenced_functions(function.doc_desc, &current);
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

/// The parameter table, taken from the first overload's parameters. Optional
/// (defaulted) parameters are flagged, and any alias spellings are listed. The
/// return type is shown only when it is a fixed nominal (`collections` members
/// are argument-dependent, so this is usually omitted).
fn render_parameters(md: &mut String, function: &BuiltinFunction) {
    let Some(overload) = function.overloads.first() else {
        return;
    };
    if overload.params.is_empty() {
        if let ReturnType::Fixed(ret) = overload.return_type {
            md.push_str(&format!("Takes no arguments and returns `{ret}`.\n\n"));
        }
        return;
    }

    md.push_str("## Parameters\n\n");
    md.push_str("| Parameter | Type | Also accepted as |\n| --- | --- | --- |\n");
    for param in overload.params {
        let optional = matches!(
            param.default,
            DefaultValue::Fill { .. } | DefaultValue::Optional
        );
        let name = if optional {
            format!("`{}` (optional)", param.name)
        } else {
            format!("`{}`", param.name)
        };
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
        md.push_str(&format!("| {name} | `{}` | {aliases} |\n", param.ty.name()));
    }
    md.push('\n');

    if let ReturnType::Fixed(ret) = overload.return_type {
        md.push_str(&format!("Returns `{ret}`.\n\n"));
    }
}

/// The Errors table, resolving each declared `errorCode` name to its `(code,
/// message)` from the single `ERRORCODE_CONSTANTS` table.
fn render_errors(md: &mut String, function: &BuiltinFunction) {
    if function.errors.is_empty() {
        return;
    }
    md.push_str("## Errors\n\n");
    md.push_str("| Code | Name | Message |\n| --- | --- | --- |\n");
    for &name in function.errors {
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
    fn renders_a_collections_function_from_the_registry() {
        // `get` carries doc_intro, doc_desc, and two declared errors.
        assert!(show_man2(&s(&["collections", "get"])).is_ok());
        // `findLastIndex` — the source-generic member added to the descriptor.
        assert!(show_man2(&s(&["collections", "findLastIndex"])).is_ok());
        // An infallible member with no errors table.
        assert!(show_man2(&s(&["collections", "append"])).is_ok());
    }

    #[test]
    fn markdown_includes_summary_description_and_errors() {
        let module = REGISTRY.module("collections").unwrap();
        let function = lookup(module, "get").unwrap();
        let md = render_function_markdown(module, function);
        assert!(md.starts_with("# get\n"));
        assert!(md.contains("## Package\n\ncollections"));
        assert!(md.contains("## Description"));
        assert!(md.contains("## Errors"));
        // Errors are resolved to their code + runtime message.
        assert!(md.contains("`ErrIndexOutOfRange`"));
        assert!(md.contains("`ErrNotFound`"));
        // The parameter table reflects the descriptor's params and aliases.
        assert!(md.contains("## Parameters"));
        assert!(md.contains("`value`"));
        assert!(md.contains("`collection`")); // alias of `value`
    }

    #[test]
    fn see_also_lists_referenced_members_and_excludes_self() {
        let module = REGISTRY.module("collections").unwrap();
        let md = render_function_markdown(module, lookup(module, "get").unwrap());
        // get's description points at getOr and hasKey.
        assert!(md.contains("## See also"));
        assert!(md.contains("- `collections::getOr`"));
        assert!(md.contains("- `collections::hasKey`"));
        // It does not list itself.
        assert!(!md.contains("- `collections::get`"));
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

    #[test]
    fn an_infallible_member_omits_the_errors_section() {
        let module = REGISTRY.module("collections").unwrap();
        let md = render_function_markdown(module, lookup(module, "append").unwrap());
        assert!(!md.contains("## Errors"));
    }

    #[test]
    fn lookup_matches_qualified_name_and_slug() {
        let module = REGISTRY.module("collections").unwrap();
        assert!(lookup(module, "get").is_some());
        assert!(lookup(module, "collections.get").is_none()); // bare slug only
        assert!(lookup(module, "definitely-not-a-fn").is_none());
    }

    #[test]
    fn rejects_non_collections_package() {
        let err = show_man2(&s(&["io", "print"])).unwrap_err();
        assert!(err.contains("wired for the `collections` package only"));
    }

    #[test]
    fn no_function_renders_the_package_overview() {
        // `mfb man2 collections` renders the package page from the registry.
        assert!(show_man2(&s(&["collections"])).is_ok());
    }

    #[test]
    fn package_markdown_has_intro_description_functions_and_aggregate_errors() {
        let module = REGISTRY.module("collections").unwrap();
        let md = render_package_markdown(module);
        assert!(md.starts_with("# collections\n"));
        // doc_intro one-liner.
        assert!(md.contains("Sequence and map helper functions"));
        // doc_desc description.
        assert!(md.contains("## Description"));
        assert!(md.contains("package-qualified helpers for `List` and `Map`"));
        // Function listing with per-member summaries.
        assert!(md.contains("## Functions"));
        assert!(md.contains("| `collections::get` |"));
        // Aggregate Errors table: the union across members, ordered by code.
        assert!(md.contains("## Errors"));
        assert!(md.contains("`ErrIndexOutOfRange`"));
        assert!(md.contains("`ErrNotFound`"));
        assert!(md.contains("`ErrOverflow`")); // from sum
        let index_pos = md.find("77050001").unwrap();
        let overflow_pos = md.find("77050010").unwrap();
        assert!(index_pos < overflow_pos, "errors ordered by code");
    }

    #[test]
    fn rejects_missing_function() {
        assert!(show_man2(&s(&["collections", "nope"]))
            .unwrap_err()
            .contains("unknown collections function"));
    }
}
