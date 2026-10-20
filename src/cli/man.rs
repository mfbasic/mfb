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
    self, registry, DefaultValue, Implementation, Parameter, RegistryConstant, RegistryFunction,
    RegistryPackage,
};
use crate::docs::man::{self, ManTopic};
use crate::docs::render;
use crate::types::ParameterType;

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
                    print_markdown(&render_no_types_markdown(package));
                    return Ok(());
                }
                // A constant is a value, not a function, and has no page of its
                // own; its row is on the package overview.
                if function.is_none() && package.constants().iter().any(|c| c.name == *page_name) {
                    return Err(format!(
                        "`{}::{page_name}` is a constant, not a function, and has no page of its own\n\nRun `mfb man {}` to see its value.",
                        package.import_name(),
                        package.import_name(),
                    ));
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
    // Then every narrative guide topic, in the same sorted order the index lists
    // them. `--all` is the whole manual a developer asked for, and a registry-only
    // sweep omitted all 32 guide pages — the `variable` topic every package page
    // links to for the value model among them.
    let mut topics: Vec<_> = man::topics().iter().collect();
    topics.sort_by_key(|topic| topic.name);
    for topic in topics {
        md.push_str(&man_rule());
        md.push_str(&render_topic_all_markdown(topic));
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

/// Every error any of a function's implementations declares, in first-seen order,
/// each paired with the numbers of the overloads that declare it.
///
/// The number is the 1-based position in `function.implementations`, which is the
/// same sequence [`render_function_markdown`] numbers the `## Overloads` list from.
/// One enumeration feeds both, so the table's cross-reference and the numbered
/// signatures cannot disagree (bug-558).
fn function_errors_by_overload(function: &RegistryFunction) -> Vec<(&'static str, Vec<usize>)> {
    let mut rows: Vec<(&'static str, Vec<usize>)> = Vec::new();
    for (index, implementation) in function.implementations.iter().enumerate() {
        let overload = index + 1;
        for &name in &implementation.errors {
            match rows.iter_mut().find(|(existing, _)| *existing == name) {
                Some((_, overloads)) => {
                    if !overloads.contains(&overload) {
                        overloads.push(overload);
                    }
                }
                None => rows.push((name, vec![overload])),
            }
        }
    }
    rows
}

/// The union of every error any of a function's implementations declares, in first-
/// seen order.
fn function_errors(function: &RegistryFunction) -> Vec<&'static str> {
    function_errors_by_overload(function)
        .into_iter()
        .map(|(name, _)| name)
        .collect()
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

    render_constants(&mut md, package);

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

/// The Constants section of a package overview: every registered constant in
/// registration order, with its type, its value as a program would write it, and a
/// Description column when any constant carries a message. Omitted when the
/// package registers none.
fn render_constants(md: &mut String, package: &RegistryPackage) {
    let constants = package.constants();
    if constants.is_empty() {
        return;
    }
    let pkg = package.import_name();
    let described = constants.iter().any(|c| c.message.is_some());
    md.push_str("## Constants\n\n");
    if described {
        md.push_str("| Constant | Type | Value | Description |\n| --- | --- | --- | --- |\n");
    } else {
        md.push_str("| Constant | Type | Value |\n| --- | --- | --- |\n");
    }
    for constant in constants {
        let ty = constant_type_name(package, constant);
        let value = match (constant.value, constant.components) {
            (Some(value), _) => value.to_string(),
            (None, Some(components)) => format!("{ty}[{}]", components.join(", ")),
            (None, None) => String::new(),
        };
        md.push_str(&format!(
            "| `{pkg}::{}` | `{ty}` | `{value}` |",
            constant.name
        ));
        if described {
            md.push_str(&format!(" {} |", constant.message.unwrap_or("")));
        }
        md.push('\n');
    }
    md.push('\n');
}

/// A constant's type as source spells it: a record the package declares is
/// qualified (`vector::Float3`); a scalar type (`Integer`, `Float`) is not.
fn constant_type_name(package: &RegistryPackage, constant: &RegistryConstant) -> String {
    let declared = package
        .records()
        .iter()
        .any(|record| record.name == constant.type_name);
    if declared {
        format!("{}::{}", package.import_name(), constant.type_name)
    } else {
        constant.type_name.to_string()
    }
}

/// The page `mfb man <pkg> types` shows for a package with no public types. It
/// points at the overview by what that page lists, so a constants-only package
/// (`errorCode`) is not told to list functions it does not have.
fn render_no_types_markdown(package: &RegistryPackage) -> String {
    let has_functions = package.functions().iter().any(|f| !f.internal_only);
    let has_constants = !package.constants().is_empty();
    let lists = match (has_functions, has_constants) {
        (true, true) => "its functions and constants",
        (false, true) => "its constants",
        _ => "its functions",
    };
    format!(
        "The `{}` package has no public types.\n\nRun `mfb man {}` to list {lists}.\n",
        package.import_name(),
        package.import_name(),
    )
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
            // The record's own prose, above its field table, in the same place a
            // resource's description already goes. `RegistryRecord::description` is
            // documented by AGENTS.md as a man-content field and by its own doc comment
            // as the source of the `DOC … END DOC` round-trip; it was only ever the
            // second. Thirty-eight records across eight packages carried prose no page
            // rendered — `canvas::Transform`'s all-zero-is-identity rule among them,
            // which `mfb spec` §"Paint is a value, not ambient state" calls the one
            // consequence worth stating explicitly.
            if !record.description.is_empty() {
                md.push_str(record.description);
                md.push_str("\n\n");
            }
            md.push_str("| Field | Type | Description |\n| --- | --- | --- |\n");
            for prop in &record.props {
                md.push_str(&format!(
                    "| `{}` | `{}` | {} |\n",
                    prop.name,
                    prop.ty.display(),
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
            // plan-172-K: what it INCLUDES comes first, as the members do.
            for included in &union.includes {
                md.push_str(&format!("- everything in `{pkg}::{included}`\n"));
            }
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
        // bug-523: every resource page said what the handle is and that it
        // closes at the end of its binding's scope, and nothing about the three
        // shapes a `RES` value actually has. Two of the three are true of ALL
        // of them, so they are stated once here rather than eleven times; the
        // third is per-resource and is DERIVED from the `sendable` bit below,
        // never restated in prose (that is the drift bug-522 recorded).
        md.push_str(
            "Each of these is a `RES` handle. Passing one to a call, or copying a record \
             that holds one, gives a second name for the same open handle — an alias, not \
             a copy — and closing through either name closes it. Binding it to a new `RES` \
             name (`RES g = f`) instead hands the handle to `g`; `f` cannot be used again. \
             A handle closes itself when the binding that holds it leaves scope. A handle may be a \
             field of a record and an element of a collection — written \
             `List OF RES <Type>`, never a bare `List OF <Type>`. `mfb man variable` \
             covers all of that in one place. Whether a particular handle may be handed \
             to another thread is stated with it.\n\n",
        );
        for resource in resources {
            md.push_str(&format!("#### {pkg}::{}\n\n", resource.name));
            md.push_str(resource.description);
            md.push_str("\n\n");
            if resource.sendable {
                md.push_str("May be handed to another thread with `thread::transfer`.\n\n");
            } else {
                match resource.unsendable_reason {
                    Some(reason) => md.push_str(&format!(
                        "Stays on the thread that opened it: {reason}.\n\n"
                    )),
                    None => md.push_str("Stays on the thread that opened it.\n\n"),
                }
            }
        }
    }

    md
}

/// The MFBASIC declaration of one overload — `pkg::name(p AS Type, [opt AS Type]) AS Return`
/// (optional/defaulted parameters are bracketed). Matches the hand-written man pages'
/// `## Overloads` convention.
/// An overload's return type as a READER can write it: `ParameterType::Arg(n)`
/// resolved to the n-th parameter's own type.
///
/// bug-616: `Arg(n)` is how a descriptor says "this overload returns the same type
/// as argument n" (`math::abs` echoes its operand; `collections::reduceRight` folds
/// into its `initial`). It is an internal placeholder — `ParameterType::name`
/// renders it `Arg0`, no page defines it, and no program can spell it — so it must
/// be substituted before it reaches a rendered page. 93 occurrences shipped before
/// this existed.
///
/// The substitution is the parameter's type VERBATIM, which is why a generic
/// parameter correctly yields the type variable (`reduceRight(…) AS U`) rather than
/// a concrete type: `U` is the honest answer at declaration level, and the
/// Description prose explains the binding.
///
/// Deliberately at the rendering layer, not on `ParameterType::name`, whose output
/// diagnostics and IR depend on (this bug's Non-goals). The walk is recursive so a
/// future nested `Arg` (`List OF Arg0`) resolves too, though no descriptor builds
/// one today — every `Arg(n)` in `src/codegen/builtins/` is a bare top-level return.
fn resolved_return_type(implementation: &Implementation) -> ParameterType {
    fn walk(ty: &ParameterType, params: &[Parameter]) -> ParameterType {
        match ty {
            // An out-of-range index cannot happen from a descriptor, but render the
            // placeholder unchanged rather than panicking in `mfb man` if one ever
            // does — the sweep test is what catches it.
            ParameterType::Arg(n) => params
                .get(*n)
                .map(|param| walk(&param.ty, params))
                .unwrap_or_else(|| ty.clone()),
            ParameterType::ListOf(elem) => ParameterType::list_of(walk(elem, params)),
            ParameterType::SetOf(elem) => ParameterType::set_of(walk(elem, params)),
            ParameterType::MapOf(key, value) => {
                ParameterType::map_of(walk(key, params), walk(value, params))
            }
            ParameterType::MapEntryOf(key, value) => {
                ParameterType::map_entry_of(walk(key, params), walk(value, params))
            }
            ParameterType::ResultOf(success) => ParameterType::result_of(walk(success, params)),
            ParameterType::Res(inner) => ParameterType::res(walk(inner, params)),
            ParameterType::UserOf(name, args) => {
                ParameterType::UserOf(*name, args.iter().map(|arg| walk(arg, params)).collect())
            }
            ParameterType::Func(fn_params, ret, isolated) => {
                let fn_params = fn_params.iter().map(|p| walk(p, params)).collect();
                let ret = walk(ret, params);
                if *isolated {
                    ParameterType::func_isolated(fn_params, ret)
                } else {
                    ParameterType::func(fn_params, ret)
                }
            }
            ParameterType::ThreadHandle {
                worker,
                msg,
                res,
                out,
            } => ParameterType::thread_handle(
                *worker,
                walk(msg, params),
                walk(res, params),
                walk(out, params),
            ),
            other => other.clone(),
        }
    }
    walk(&implementation.return_type, &implementation.params)
}

fn render_declaration(pkg: &str, name: &str, implementation: &Implementation) -> String {
    let params = implementation
        .params
        .iter()
        .map(|param| {
            let decl = format!("{} AS {}", param.name, param.ty.display());
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
    // Every member renders as `pkg::name(...)`, including the unqualified-global
    // packages. `testing` and `general` are the globals holders — `testing` holds the
    // globals for testing, `general` the globals for the language — so a program calls
    // their members bare and never needs the prefix; on their own pages the package name
    // is the documentation grouping. `testing::expectEqual` / `general::len` here is
    // intended: bug-607 reported it as a defect and was closed as NOT A BUG. See
    // `RegistryPackage::unqualified_global` (`src/codegen/registry/mod.rs`).
    format!(
        "`{pkg}::{name}({params}) AS {}`",
        resolved_return_type(implementation).display()
    )
}

/// Whether `function`'s forms differ ONLY by return type — every implementation
/// spells the same parameter list, so no argument can select between them and the
/// call's expected (contextual) type has to. `encoding::utf8Encode` is the one
/// such member in the language today; the test is written on the descriptor
/// rather than on that name so a second one renders the same note (bug-530).
fn is_return_type_overload_set(function: &RegistryFunction) -> bool {
    let mut implementations = function.implementations.iter();
    let Some(first) = implementations.next() else {
        return false;
    };
    let mut any_other = false;
    for other in implementations {
        any_other = true;
        if other.params.len() != first.params.len() {
            return false;
        }
        let same = other.params.iter().zip(first.params.iter()).all(|(a, b)| {
            a.name == b.name
                && a.ty == b.ty
                && matches!(a.default, DefaultValue::None)
                    == matches!(b.default, DefaultValue::None)
        });
        if !same {
            return false;
        }
    }
    any_other
}

/// One row of a function page's Parameters table (bug-591).
struct ParameterRow<'a> {
    param: &'a Parameter,
    /// The 1-based overload numbers — matching the Overloads section — that declare
    /// exactly this rendered parameter.
    overloads: Vec<usize>,
}

/// The rows of a function page's Parameters table (bug-591).
///
/// This used to be the union of every overload's parameters de-duplicated by NAME,
/// first occurrence winning. That is right when overloads only ADD parameters
/// (`process::spawn`'s `cwd`, `env`, `envReplace`) and silently wrong when two
/// overloads declare the SAME name with a different type or description:
/// `collections::get`'s map overload takes `index AS K`, "the key to look up", and
/// the page showed overload 1's `index AS Integer`, "the list index, zero-based", as
/// if it described both. bug-563 corrected that description in the descriptor and
/// the correction was invisible, because this table never rendered it.
///
/// Rows are grouped by how a parameter RENDERS — name, type, description, aliases
/// and whether it is optional — because those are exactly the cells a reader sees.
/// Order is each name's first appearance across the overloads, then each distinct
/// form's first appearance, so a member whose overloads never disagree gets the
/// same rows in the same order as before.
fn parameter_rows(function: &RegistryFunction) -> Vec<ParameterRow<'_>> {
    let mut names: Vec<&'static str> = Vec::new();
    let mut rows: Vec<ParameterRow<'_>> = Vec::new();
    for (index, implementation) in function.implementations.iter().enumerate() {
        let number = index + 1;
        for param in &implementation.params {
            if !names.contains(&param.name) {
                names.push(param.name);
            }
            match rows
                .iter_mut()
                .find(|row| renders_identically(row.param, param))
            {
                Some(row) => {
                    if !row.overloads.contains(&number) {
                        row.overloads.push(number);
                    }
                }
                None => rows.push(ParameterRow {
                    param,
                    overloads: vec![number],
                }),
            }
        }
    }
    // Stable, so distinct forms of one name keep their first-appearance order.
    rows.sort_by_key(|row| names.iter().position(|name| *name == row.param.name));
    rows
}

/// Whether two parameters produce the same Parameters-table cells.
fn renders_identically(a: &Parameter, b: &Parameter) -> bool {
    a.name == b.name
        && a.ty.display() == b.ty.display()
        && a.desc == b.desc
        && a.aliases == b.aliases
        && is_optional_parameter(a) == is_optional_parameter(b)
}

fn is_optional_parameter(param: &Parameter) -> bool {
    matches!(
        param.default,
        DefaultValue::Fill { .. } | DefaultValue::Optional
    )
}

/// Whether any name has more than one rendered form, i.e. whether a reader needs
/// to be told which overload each row belongs to.
fn parameters_differ_by_overload(rows: &[ParameterRow<'_>]) -> bool {
    rows.iter().enumerate().any(|(i, row)| {
        rows[..i]
            .iter()
            .any(|earlier| earlier.param.name == row.param.name)
    })
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
        // Numbered, because the Errors table below cross-references these numbers
        // (bug-558). `function_errors_by_overload` numbers the SAME iteration, so
        // the two can only agree.
        for (index, implementation) in function.implementations.iter().enumerate() {
            md.push_str(&format!(
                "{}. **{}**\n",
                index + 1,
                render_declaration(pkg, function.name, implementation)
            ));
        }
        md.push('\n');
        // A set whose forms share one parameter list cannot be told apart from the
        // arguments, so the reader needs the contextual-type rule HERE — beside the
        // signatures — rather than several paragraphs down (bug-530).
        if is_return_type_overload_set(function) {
            md.push_str(
                "These overloads take the same arguments and differ only by what they \
                 return, so a call needs an expected type to choose one — a \
                 `LET … AS` annotation, a typed parameter slot, or the enclosing \
                 function's return type. A call with no expected type is a \
                 compile-time error.\n\n",
            );
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

    render_function_errors_table(&mut md, function);

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
///
/// Only a reference whose first part is a PACKAGE is one. A description may hold
/// other things spelled `a::b` — `canvas::pollMouse`'s explains that `overID` reads
/// `"save::label"` — and those are not links to anything.
fn render_see_also(md: &mut String, package: &RegistryPackage, function: &RegistryFunction) {
    let current = format!("{}::{}", package.import_name(), function.name);
    let referenced: Vec<String> = referenced_functions(function.desc, &current)
        .into_iter()
        .filter(|reference| {
            reference
                .split_once("::")
                .is_some_and(|(owner, _)| registry().resolve_package(owner).is_some())
        })
        .collect();
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

/// The Parameters table. Optional (defaulted) parameters are flagged, alias spellings are
/// listed if present, and a single-overload member gets a trailing Returns line.
///
/// When two overloads declare the same parameter name in different forms, the table gains
/// an **Overloads** column naming which numbered signature each row belongs to — the same
/// remedy bug-558 applied to the Errors table (bug-591). When no name differs, the column
/// is omitted and the table is exactly the one it has always been, so single-overload pages
/// and overload sets that only add parameters render byte for byte as before.
fn render_parameters(md: &mut String, function: &RegistryFunction) {
    // Return type(s) are shown in the Overloads/Declaration section above; a single
    // trailing "Returns" line is added only when there is exactly one overload (with
    // multiple overloads the returns can differ, so one line would be misleading).
    let rows = parameter_rows(function);
    let per_overload = parameters_differ_by_overload(&rows);
    let single = function.implementations.len() == 1;
    let return_type = function
        .implementations
        .first()
        .map(|implementation| resolved_return_type(implementation).display());

    if rows.is_empty() {
        if single {
            if let Some(return_type) = &return_type {
                md.push_str(&format!(
                    "Takes no arguments and returns `{return_type}`.\n\n"
                ));
            }
        }
        return;
    }

    let has_aliases = rows.iter().any(|row| !row.param.aliases.is_empty());

    md.push_str("## Parameters\n\n");
    let mut header = String::from("| Parameter | Type |");
    let mut rule = String::from("| --- | --- |");
    if has_aliases {
        header.push_str(" Alternate |");
        rule.push_str(" --- |");
    }
    if per_overload {
        header.push_str(" Overloads |");
        rule.push_str(" --- |");
    }
    header.push_str(" Description |\n");
    rule.push_str(" --- |\n");
    md.push_str(&header);
    md.push_str(&rule);

    for row in &rows {
        let param = row.param;
        let name = if is_optional_parameter(param) {
            format!("`{}` (opt)", param.name)
        } else {
            format!("`{}`", param.name)
        };
        let mut line = format!("| {name} | `{}` |", param.ty.display());
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
            line.push_str(&format!(" {aliases} |"));
        }
        if per_overload {
            let numbers = row
                .overloads
                .iter()
                .map(|number| number.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            line.push_str(&format!(" {numbers} |"));
        }
        line.push_str(&format!(" {} |\n", param.desc));
        md.push_str(&line);
    }
    md.push('\n');

    if single {
        if let Some(return_type) = &return_type {
            md.push_str(&format!("Returns `{return_type}`.\n\n"));
        }
    }
}

/// The `errorCode` value an error name resolves to, and the sort key of every
/// Errors table. An unknown name sorts first and renders blank rather than
/// dropping the row.
fn error_code(name: &str) -> &'static str {
    registry::runtime_error(name)
        .map(|(code, _)| code)
        .unwrap_or("")
}

/// The Errors table of a FUNCTION page.
///
/// A member with more than one overload gets an extra **Overloads** column naming
/// which numbered signature(s) raise each error, because the union alone told a
/// reader of `tcp::poll`'s scalar form to handle an `ErrTimeout` only the list form
/// raises (bug-558). A single-overload member has nothing to cross-reference — the
/// column would read `1` on every row of the 93 single-overload pages that render
/// an Errors table — so its table is the three-column one it has always been, byte
/// for byte.
fn render_function_errors_table(md: &mut String, function: &RegistryFunction) {
    let mut rows = function_errors_by_overload(function);
    if rows.is_empty() {
        return;
    }
    if function.implementations.len() < 2 {
        let names: Vec<&'static str> = rows.into_iter().map(|(name, _)| name).collect();
        render_errors_table(md, &names);
        return;
    }

    rows.sort_by_key(|(name, _)| error_code(name));
    md.push_str("## Errors\n\n");
    md.push_str("| Code | Name | Overloads | Message |\n| --- | --- | --- | --- |\n");
    for (name, overloads) in &rows {
        let (code, message) = registry::runtime_error(name).unwrap_or(("", ""));
        let numbers = overloads
            .iter()
            .map(|overload| overload.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        md.push_str(&format!(
            "| `{code}` | `{name}` | {numbers} | {message} |\n"
        ));
    }
    md.push('\n');

    // An overload that declares no errors is absent from every row, and absence is
    // easy to read as an oversight rather than as a promise. `canvas::getSize()` —
    // the no-argument form, which reads the runtime's own record — is the one such
    // signature in the rendered surface today, and its page carried a two-row
    // Errors table over it, so say it outright.
    let infallible: Vec<String> = (1..=function.implementations.len())
        .filter(|overload| !rows.iter().any(|(_, o)| o.contains(overload)))
        .map(|overload| overload.to_string())
        .collect();
    if !infallible.is_empty() {
        let (label, verb) = if infallible.len() == 1 {
            ("Overload", "raises")
        } else {
            ("Overloads", "raise")
        };
        md.push_str(&format!(
            "{label} {} {verb} no errors.\n\n",
            infallible.join(", ")
        ));
    }
}

/// Render an Errors table for a set of `errorCode` names, resolving each to its
/// `(code, message)` and ordering by code. No-op when `names` is empty.
fn render_errors_table(md: &mut String, names: &[&'static str]) {
    if names.is_empty() {
        return;
    }
    let mut ordered = names.to_vec();
    ordered.sort_by_key(|name| error_code(name));

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
mod tests;
