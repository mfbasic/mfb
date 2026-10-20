use super::*;

fn s(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

/// bug-526: a rendered signature has to be a declaration that COMPILES.
///
/// `mfb man tls poll` printed
/// `tls::poll(socks AS List OF tls::Socket, …)` in both the Overloads block
/// and the Parameters table, and that spelling is refused —
/// `TYPE_RESOURCE_REQUIRES_RES`, "Collection element type `tls.Socket` is a
/// resource; mark it `RES`". The same page's Description and example had it
/// right, so the page carried both the correct form and an uncompilable one,
/// and the uncompilable one was in the two places a reader looks first.
///
/// The cause is that a `ParameterType` built by PARSING a string and one
/// built by CONSTRUCTION are not interchangeable: `parse` strips the `RES `
/// marker off a collection element, so a descriptor written
/// `ListOf(named(T))` has no marker to render, while `tcp::poll`'s
/// `list_of(Res(named(T)))` keeps one and still unifies against the same
/// concrete argument. The difference is invisible until something renders
/// it — which is exactly what this test does.
///
/// Scoped to collection ELEMENTS because that is what the diagnostic
/// enforces: `List OF RES T`, never `List OF T`. A bare top-level resource
/// parameter (`sock AS tls::Socket`) renders without `RES` across the whole
/// tree and is a separate question.
#[test]
fn every_rendered_signature_marks_a_resource_collection_element_res() {
    use crate::codegen::resource::is_builtin_resource_type;
    use crate::types::ParameterType;

    /// Walk `ty`, reporting each collection element that names a built-in
    /// resource without a `RES` marker.
    fn check(ty: &ParameterType, bad: &mut Vec<String>) {
        match ty {
            ParameterType::ListOf(inner) | ParameterType::SetOf(inner) => {
                element(inner, bad);
            }
            ParameterType::MapOf(key, value) | ParameterType::MapEntryOf(key, value) => {
                element(key, bad);
                element(value, bad);
            }
            ParameterType::ResultOf(inner) | ParameterType::Res(inner) => check(inner, bad),
            ParameterType::Stateful { base, .. } => check(base, bad),
            _ => {}
        }
    }

    /// One collection element position.
    fn element(ty: &ParameterType, bad: &mut Vec<String>) {
        match ty {
            // Marked: correct. Keep walking for a nested collection.
            ParameterType::Res(inner) => check(inner, bad),
            ParameterType::Stateful { base, .. } => element(base, bad),
            other => {
                if is_builtin_resource_type(other) {
                    bad.push(other.name().into_owned());
                }
                check(other, bad);
            }
        }
    }

    let mut failures = Vec::new();
    for package in registry().packages() {
        for function in package.functions() {
            for implementation in function.implementations() {
                let mut bad = Vec::new();
                for param in &implementation.params {
                    check(&param.ty, &mut bad);
                }
                check(&implementation.return_type, &mut bad);
                if !bad.is_empty() {
                    failures.push(format!(
                        "{} — unmarked resource element(s) {bad:?}",
                        render_declaration(package.import_name(), function.name, implementation)
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "a rendered signature must be a declaration that compiles, and a \
             collection of a resource must carry the `RES` marker \
             (`List OF RES T`, never `List OF T` — TYPE_RESOURCE_REQUIRES_RES). \
             Build the parameter with `ParameterType::list_of(ParameterType::\
             Res(..))` as `tcp::poll` does, not `ListOf(named(..))`, which drops \
             the marker (bug-526):\n{}",
        failures.join("\n")
    );
}

/// bug-523: every resource type page states, for every built-in resource,
/// whether the handle may cross a thread — and states it from the
/// `sendable` bit, not from prose.
///
/// That is the whole point of deriving it. bug-522 is what prose costs: two
/// pages restated the transferable set, bug-464 flipped three bits, and one
/// of the two sentences went a year saying the opposite of what the compiler
/// enforced. A line rendered from the bit cannot disagree with it.
///
/// The reason a handle stays put IS prose, because it is per-resource; this
/// asserts every non-sendable row has one, so a new one cannot render the
/// bare line by omission.
#[test]
fn every_resource_page_states_transferability_from_the_sendable_bit() {
    let mut seen = 0;
    for package in registry().packages() {
        if !package.resources().iter().any(|r| r.export) {
            continue;
        }
        let md = render_types_markdown(package);
        // Stated once for the whole section, not eleven times.
        assert!(
            md.contains("may be a field of a record and an element of a collection"),
            "{}'s types page must state the record-field and collection-element \
                 shapes a RES handle has (bug-523)",
            package.import_name()
        );
        assert!(
            md.contains("`List OF RES <Type>`, never a bare `List OF <Type>`"),
            "{}'s types page must state the RES-marker rule before a reader \
                 meets TYPE_RESOURCE_REQUIRES_RES (bug-523)",
            package.import_name()
        );
        for resource in package.resources().iter().filter(|r| r.export) {
            seen += 1;
            let name = format!("{}::{}", package.import_name(), resource.name);
            if resource.sendable {
                assert!(
                    md.contains("May be handed to another thread with `thread::transfer`."),
                    "{name} is sendable, so its page must say so"
                );
            } else {
                let reason = resource.unsendable_reason.unwrap_or_else(|| {
                    panic!(
                        "{name} is not thread-sendable and carries no \
                             `unsendable_reason`; the type page would state the bare \
                             fact with no reason (bug-523)"
                    )
                });
                assert!(
                    md.contains(&format!("Stays on the thread that opened it: {reason}.")),
                    "{name}'s page must give its reason for staying put"
                );
            }
        }
    }
    assert_eq!(
        seen, 11,
        "the built-in resource census is 11 exported handles; a new one needs a \
             verdict here and in `codegen::resource`'s sendability table"
    );
}

#[test]
fn renders_a_csv_function_from_the_clean_room_registry() {
    assert!(show_man(&s(&["csv", "parse"])).is_ok());
    assert!(show_man(&s(&["csv", "readRow"])).is_ok());
    assert!(show_man(&s(&["csv", "stringify"])).is_ok());
}

#[test]
fn markdown_includes_summary_description_and_parameters() {
    // `csv::stringify` (one form). `csv::parse` was the example until bug-809
    // gave it a `File` overload; it still carries the alias checked below.
    let package = registry().resolve_package("csv").unwrap();
    let function = package.function("stringify").unwrap();
    let md = render_function_markdown(package, function);
    assert!(md.starts_with("# stringify\n"));
    assert!(md.contains("## Package\n\ncsv"));
    assert!(md.contains("## Description"));
    // A single-overload member renders a Declaration section above Parameters.
    assert!(md.contains("## Declaration"));
    assert!(md.contains("`csv::stringify("));
    assert!(md.contains(") AS String`"));
    assert!(!md.contains("## Overloads"));
    // The parameter table reflects the descriptor's params, aliases, and optionals.
    assert!(md.contains("## Parameters"));
    assert!(md.contains("`value`"));
    assert!(md.contains("`delimiter` (opt)")); // Fill-defaulted
    assert!(md.contains("Returns `String`."));
    let parse = render_function_markdown(package, package.function("parse").unwrap());
    assert!(parse.contains("`text`")); // alias of `value`
    assert!(md.contains("## Examples"));
    // The Errors table is rendered from the descriptor's declared errors.
    assert!(md.contains("## Errors"));
    assert!(md.contains("`77050003`"));
    assert!(md.contains("`ErrInvalidFormat`"));
}

/// bug-553: `tcp`/`tls` gained real `errors:` lists, and the page has to grow
/// an Errors table WITHOUT losing anything it already rendered. The `errors`
/// field feeds nothing but this table, so the rest of the page is the control.
#[test]
fn a_tcp_member_that_gained_errors_still_renders_its_other_sections() {
    let package = registry().resolve_package("tcp").unwrap();
    let function = package.function("read").unwrap();
    let md = render_function_markdown(package, function);

    // The new table, resolved from the declared names to code + message.
    assert!(md.contains("## Errors"));
    assert!(md.contains("`ErrConnectionClosed`"));
    assert!(md.contains("`ErrTimeout`"));
    // ...and NOT an error the lowering cannot raise: `ErrEncoding` lives
    // inside `lower_net_read_helper`'s `if text` arm, which `tcp::read` does
    // not select.
    assert!(!md.contains("`ErrEncoding`"));

    // Everything the page rendered before is still there.
    assert!(md.starts_with("# read\n"));
    assert!(md.contains("## Package\n\ntcp"));
    assert!(md.contains("## Description"));
    assert!(md.contains("## Declaration"));
    assert!(md.contains("`tcp::read(sock AS tcp::Socket"));
    assert!(md.contains("## Parameters"));
    assert!(md.contains("`maxBytes`"));
    assert!(md.contains("Returns `List OF Byte`."));
    assert!(md.contains("## Examples"));
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

/// The row of an Errors table whose Name cell is `name`.
fn errors_row<'a>(md: &'a str, name: &str) -> &'a str {
    md.lines()
        .find(|line| line.starts_with("| `") && line.contains(&format!("| `{name}` |")))
        .unwrap_or_else(|| panic!("no Errors row for {name}:\n{md}"))
}

/// bug-558: the Errors table rendered the UNION of every overload's declared
/// errors under one heading while the Overloads block listed the signatures
/// separately, so nothing on the page said which signature raised what.
/// `tcp::poll`'s scalar form — listed first, and the common one — read as if it
/// raised `ErrTimeout`; readiness on one socket is a QUERY and an expired wait
/// is `FALSE`, so a program written from that page carried a handler that can
/// never run. The overloads are numbered and the table names the numbers.
#[test]
fn a_multi_overload_errors_table_names_the_overloads_that_raise_each_error() {
    let package = registry().resolve_package("tcp").unwrap();
    let function = package.function("poll").unwrap();
    assert_eq!(function.implementations.len(), 2);
    let md = render_function_markdown(package, function);

    // The signatures are numbered, in descriptor order.
    assert!(
        md.contains("1. **`tcp::poll(sock AS tcp::Socket"),
        "the scalar form is overload 1:\n{md}"
    );
    assert!(
        md.contains("2. **`tcp::poll(socks AS List OF RES tcp::Socket"),
        "the list form is overload 2:\n{md}"
    );

    // ...and the table cross-references those same numbers.
    assert!(
        md.contains("| Code | Name | Overloads | Message |"),
        "a multi-overload Errors table carries the Overloads column:\n{md}"
    );
    // `ErrTimeout` and `ErrOutOfMemory` are the LIST form's alone.
    assert!(
        errors_row(&md, "ErrTimeout").contains("| 2 |"),
        "ErrTimeout belongs to overload 2 only:\n{md}"
    );
    assert!(
        errors_row(&md, "ErrOutOfMemory").contains("| 2 |"),
        "ErrOutOfMemory belongs to overload 2 only:\n{md}"
    );
    // Both forms validate their arguments and both reject a closed handle.
    assert!(
        errors_row(&md, "ErrInvalidArgument").contains("| 1, 2 |"),
        "ErrInvalidArgument belongs to both overloads:\n{md}"
    );
    assert!(
        errors_row(&md, "ErrResourceClosed").contains("| 1, 2 |"),
        "ErrResourceClosed belongs to both overloads:\n{md}"
    );
}

/// The numbers in the Errors column and the numbers on the Overloads list come
/// from ONE enumeration of `implementations`, so a cross-reference cannot point
/// at a signature that is not there. Checked over every multi-overload member
/// in the registry rather than on one page: two enumerations would agree on
/// `tcp::poll` and drift somewhere else.
#[test]
fn every_errors_overload_number_names_a_rendered_signature() {
    for package in registry().packages() {
        for function in package.functions() {
            if function.internal_only || function.implementations.len() < 2 {
                continue;
            }
            let md = render_function_markdown(package, function);
            for (name, overloads) in function_errors_by_overload(function) {
                for overload in overloads {
                    assert!(
                        overload >= 1 && overload <= function.implementations.len(),
                        "{}::{} cites overload {overload} for {name}, but it has {} \
                             signatures",
                        package.import_name(),
                        function.name,
                        function.implementations.len(),
                    );
                    assert!(
                        md.contains(&format!(
                            "{overload}. **{}",
                            render_declaration(
                                package.import_name(),
                                function.name,
                                &function.implementations[overload - 1]
                            )
                        )),
                        "{}::{}'s Overloads list must number {overload} the way the \
                             Errors table cites it:\n{md}",
                        package.import_name(),
                        function.name,
                    );
                }
            }
        }
    }
}

/// The worst shape bug-558 names: one overload declares errors and its sibling
/// declares NONE, so the page showed an Errors table over an infallible
/// signature. `canvas::getSize()` (no argument) reads the runtime's own record
/// and cannot fail; `canvas::getSize(image)` can. Absence from every row is the
/// table's way of saying so, and absence reads as an oversight — so the page
/// says it in words too.
#[test]
fn an_overload_that_declares_no_errors_is_named_as_infallible() {
    let package = registry().resolve_package("canvas").unwrap();
    let function = package.function("getSize").unwrap();
    assert_eq!(function.implementations.len(), 2);
    let md = render_function_markdown(package, function);

    assert!(md.contains("1. **`canvas::getSize(image AS canvas::Image)"));
    assert!(md.contains("2. **`canvas::getSize()"));
    assert!(errors_row(&md, "ErrResourceClosed").contains("| 1 |"));
    assert!(errors_row(&md, "ErrOutOfMemory").contains("| 1 |"));
    assert!(
        md.contains("Overload 2 raises no errors."),
        "the infallible signature must be named, not merely absent:\n{md}"
    );
}

/// bug-558's containment argument, and the thing most likely to go wrong: the
/// column is suppressed where it carries no information. A single-overload
/// member has one signature to name, so its Errors table keeps its three
/// columns and its page does not change at all — which is 449 of the 543
/// rendered function pages, plus every package overview and types page.
#[test]
fn a_single_overload_member_errors_table_is_unchanged() {
    let package = registry().resolve_package("fs").unwrap();
    let function = package.function("close").unwrap();
    assert_eq!(function.implementations.len(), 1);
    let md = render_function_markdown(package, function);

    assert!(md.contains("## Errors"));
    assert!(
        md.contains("| Code | Name | Message |\n| --- | --- | --- |\n"),
        "a single-overload member keeps the three-column table:\n{md}"
    );
    assert!(!md.contains("Overloads"), "no column, no heading:\n{md}");
    assert!(!md.contains("raises no errors"));
}

/// The package OVERVIEW's Errors table is the union over every member, so it
/// has no single signature to number and must keep three columns too.
#[test]
fn a_package_overview_errors_table_has_no_overload_column() {
    let package = registry().resolve_package("tcp").unwrap();
    let md = render_package_markdown(package);
    assert!(md.contains("## Errors"));
    assert!(md.contains("| Code | Name | Message |\n| --- | --- | --- |\n"));
    assert!(!md.contains("| Code | Name | Overloads | Message |"));
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

/// The lines of a rendered page's Parameters table, header and rule included.
fn parameters_table(md: &str) -> Vec<&str> {
    let Some(start) = md.find("## Parameters\n\n") else {
        return Vec::new();
    };
    md[start + "## Parameters\n\n".len()..]
        .lines()
        .take_while(|line| line.starts_with('|'))
        .collect()
}

/// bug-591, TOTAL over the registry: every overload's every parameter must appear
/// on one Parameters row with its OWN name and its OWN rendered type.
///
/// The table used to de-duplicate by name, so a second overload's same-named
/// parameter vanished — `collections::get`'s map overload showed overload 1's
/// `index AS Integer` in place of its own `index AS K`. Asserted for every
/// multi-overload member rather than for `get` alone, so the next member to reuse
/// a parameter name in a new form is caught without anyone listing it here.
#[test]
fn every_overloads_parameters_appear_in_its_parameters_table() {
    let mut checked = 0;
    for package in registry().packages() {
        for function in package.functions() {
            if function.implementations.len() < 2 {
                continue;
            }
            let md = render_function_markdown(package, function);
            let table = parameters_table(&md);
            for (index, implementation) in function.implementations.iter().enumerate() {
                for param in &implementation.params {
                    let name = format!("| `{}`", param.name);
                    let ty = format!("`{}`", param.ty.display());
                    assert!(
                        table
                            .iter()
                            .any(|line| line.starts_with(&name) && line.contains(&ty)),
                        "{}::{} overload {}: parameter `{}` AS {} is missing from its \
                             Parameters table:\n{}",
                        package.import_name(),
                        function.name,
                        index + 1,
                        param.name,
                        param.ty.display(),
                        table.join("\n")
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(
        checked > 0,
        "the registry has multi-overload members to check"
    );
}

/// bug-591: `collections::get`, the page that surfaced it, now shows the map
/// overload's key as its own row, with its own type and its own description, and
/// tells the reader which overload each row belongs to.
#[test]
fn collections_get_shows_the_map_key_as_its_own_parameter_row() {
    let package = registry().resolve_package("collections").unwrap();
    let function = package.function("get").unwrap();
    let md = render_function_markdown(package, function);
    let table = parameters_table(&md);
    assert!(
        table[0].contains(" Overloads |"),
        "same-named parameters differ, so the column is required:\n{}",
        table.join("\n")
    );
    assert!(
        table.iter().any(|line| line.starts_with("| `index`")
            && line.contains("`Integer`")
            && line.contains("zero-based")),
        "the list overload's index row:\n{}",
        table.join("\n")
    );
    assert!(
        table.iter().any(|line| line.starts_with("| `index`")
            && line.contains("`K`")
            && line.contains("The key to look up")),
        "the map overload's key row, with its own prose:\n{}",
        table.join("\n")
    );
}

/// bug-591 containment: a member whose overloads never disagree about a parameter
/// keeps its table exactly as it was — no Overloads column. Every single-overload
/// member qualifies by construction; asserted over the whole registry.
#[test]
fn a_parameters_table_with_no_disagreement_has_no_overloads_column() {
    for package in registry().packages() {
        for function in package.functions() {
            if function.implementations.len() != 1 {
                continue;
            }
            let md = render_function_markdown(package, function);
            if let Some(header) = parameters_table(&md).first() {
                assert!(
                    !header.contains("Overloads"),
                    "{}::{} has one overload and must not grow an Overloads column",
                    package.import_name(),
                    function.name
                );
            }
        }
    }
}

/// bug-591: every overload number named in a Parameters table must be a signature
/// the page actually renders — the Parameters twin of bug-558's
/// `every_errors_overload_number_names_a_rendered_signature`.
#[test]
fn every_parameters_overload_number_names_a_rendered_signature() {
    for package in registry().packages() {
        for function in package.functions() {
            let md = render_function_markdown(package, function);
            let table = parameters_table(&md);
            let Some(header) = table.first() else {
                continue;
            };
            let columns: Vec<&str> = header.split('|').map(str::trim).collect();
            let Some(column) = columns.iter().position(|c| *c == "Overloads") else {
                continue;
            };
            for line in table.iter().skip(2) {
                let cells: Vec<&str> = line.split('|').map(str::trim).collect();
                for number in cells[column].split(", ") {
                    let n: usize = number.parse().unwrap_or_else(|_| {
                        panic!(
                            "{}::{}: not an overload number: {number:?}",
                            package.import_name(),
                            function.name
                        )
                    });
                    assert!(
                        (1..=function.implementations.len()).contains(&n),
                        "{}::{}: overload {n} is not a rendered signature",
                        package.import_name(),
                        function.name
                    );
                }
            }
        }
    }
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

/// `canvas::CapStyle` and `canvas::JoinStyle` reach the types page with every variant,
/// and the stroke's shape — `cap`, `join`, `miterLimit`, `dashes`, `dashOffset` — is
/// carried by `canvas::Paint`, not by the shapes with ends.
///
/// plan-116-D added user-visible surface out of registry *data*, which no compiler
/// gate reads: an enum registered with a variant missing, or a field on the wrong
/// record, would still build and leave every other test green. Rendering it is the only
/// check those have. plan-163-B moved `cap` from `Line`, `Arc` and `PolyLine` onto
/// `Paint` (a dash has ends on every kind, so the cap belongs to the stroke) — the
/// placement plan-116-D had rejected, and which this test pinned against until then.
#[test]
fn the_stroke_style_enums_and_paint_fields_render() {
    let md = render_types_markdown(registry().resolve_package("canvas").unwrap());

    // One type's entry: from its `#### ` heading to the next one.
    let entry = |heading: &str| -> String {
        let at = md
            .find(heading)
            .unwrap_or_else(|| panic!("canvas exports {heading}"));
        let rest = &md[at..];
        let end = rest[4..].find("\n#### ").map_or(rest.len(), |i| i + 4);
        rest[..end].to_string()
    };

    let cap_style = entry("#### canvas::CapStyle");
    for variant in ["`Butt`", "`Round`", "`Square`"] {
        assert!(
            cap_style.contains(variant),
            "canvas::CapStyle is missing {variant}:\n{cap_style}"
        );
    }
    let join_style = entry("#### canvas::JoinStyle");
    for variant in ["`Round`", "`Miter`", "`Bevel`"] {
        assert!(
            join_style.contains(variant),
            "canvas::JoinStyle is missing {variant}:\n{join_style}"
        );
    }

    let paint = entry("#### canvas::Paint");
    for field in [
        "| `cap` | `CapStyle` |",
        "| `join` | `JoinStyle` |",
        "| `miterLimit` | `Float` |",
        "| `dashes` | `List OF Float` |",
        "| `dashOffset` | `Float` |",
    ] {
        assert!(
            paint.contains(field),
            "canvas::Paint does not carry {field}:\n{paint}"
        );
    }

    // And nowhere else: one source for the value.
    for record in [
        "#### canvas::Line",
        "#### canvas::Arc",
        "#### canvas::PolyLine",
    ] {
        let table = entry(record);
        assert!(
            !table.contains("| `cap` |"),
            "{record} still carries its own cap besides `Paint.cap`:\n{table}"
        );
    }
}

/// A record's own description reaches the page, above its field table.
///
/// Worth a test of its own because the failure is silent in both directions: a
/// renderer that drops the prose still emits a complete-looking type entry, and the
/// prose itself is an `&'static str` no compiler gate reads. `canvas::Transform` is
/// the case that found it — its all-zero-is-identity rule is the one consequence
/// `mfb spec` singles out, and no page showed it.
#[test]
fn a_records_own_description_renders_above_its_fields() {
    let package = registry().resolve_package("canvas").unwrap();
    let md = render_types_markdown(package);
    let at = md
        .find("#### canvas::Transform")
        .expect("canvas exports Transform");
    let table = md[at..]
        .find("| Field |")
        .expect("Transform renders a field table");
    let head = &md[at..at + table];
    assert!(
        head.contains("all-zero"),
        "the record's description should sit between its heading and its field \
             table, but that span is:\n{head}"
    );
    // And an undocumented record is unchanged — a bare heading then the table.
    let json = render_types_markdown(registry().resolve_package("json").unwrap());
    let at = json
        .find("#### json::JsonObj")
        .expect("json exports JsonObj");
    assert!(
        json[at..].starts_with("#### json::JsonObj\n\n| Field |"),
        "a record with an empty description must render exactly as before"
    );
}

#[test]
fn types_page_renders_a_record_only_package() {
    // `regex` (csv was the example until bug-809 removed its two records).
    let package = registry().resolve_package("regex").unwrap();
    let md = render_types_markdown(package);
    assert!(md.contains("## Records"));
    assert!(md.contains("#### regex::Group"));
    assert!(md.contains("#### regex::MatchInfo"));
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
        unsendable_reason: None,
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
    // `bits` declares no records/unions/enums/resources (and no function named
    // `types`), so `mfb man bits types` prints the friendly no-public-types
    // message instead of an unknown-function error.
    //
    // This was `encoding` until plan-123-A gave it a public `Codepage` enum;
    // `bits` is the stable stand-in because its surface is a frozen set of
    // integer operations over `Integer`, with no type of its own to grow.
    assert!(!has_public_types(
        registry().resolve_package("bits").unwrap()
    ));
    assert!(show_man(&s(&["bits", "types"])).is_ok());
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
    // A package that exports records points at its types page (`regex`;
    // csv exported none after bug-809, and gets no pointer).
    let regex = render_package_markdown(registry().resolve_package("regex").unwrap());
    assert!(regex.contains("## See also"));
    assert!(regex.contains("`mfb man regex types`"));
    assert!(!md.contains("`mfb man csv types`"));
    // A package with no public types gets no such pointer. (`bits`, not
    // `encoding`: plan-123-A gave `encoding` a public `Codepage` enum.)
    let bits = registry().resolve_package("bits").unwrap();
    assert!(!render_package_markdown(bits).contains("## See also"));
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

/// The Overloads/Declaration block of a rendered page — everything between
/// that heading and the next one.
fn signature_section(md: &str) -> &str {
    let start = match md
        .find("## Overloads\n")
        .or_else(|| md.find("## Declaration\n"))
    {
        Some(start) => start,
        None => return "",
    };
    let rest = &md[start + 3..];
    match rest.find("\n## ") {
        Some(end) => &rest[..end],
        None => rest,
    }
}

/// bug-530: `encoding::utf8Encode` is the language's one RETURN-TYPE overload
/// — the same `String` argument yields either a `List OF Byte` or a
/// `List OF Integer`, chosen by the expected type at the call site — and its
/// page rendered a single `Declaration` ending `AS List OF Byte`. The second
/// form was invisible in both places a reader looks for a signature, so an
/// ordinary-looking unannotated call failed to compile with nothing on the
/// page predicting it.
#[test]
fn a_return_type_overload_renders_every_form() {
    let package = registry().resolve_package("encoding").unwrap();
    let function = package.function("utf8Encode").unwrap();
    let md = render_function_markdown(package, function);
    assert!(
        md.contains("## Overloads"),
        "a two-form member renders an Overloads block, not a Declaration:\n{md}"
    );
    assert!(!md.contains("## Declaration"));
    assert!(
        md.contains("`encoding::utf8Encode(value AS String) AS List OF Byte`"),
        "the byte form must render:\n{md}"
    );
    assert!(
        md.contains("`encoding::utf8Encode(value AS String) AS List OF Integer`"),
        "the integer form must render:\n{md}"
    );
    // The contextual-type requirement belongs in the signature area, not only
    // in the third paragraph of the description.
    assert!(
        md.contains("differ only by what they return"),
        "the signature area must state that an expected type is required:\n{md}"
    );
}

/// The positive half of the bug-530 pin: the note is emitted for a
/// return-type overload set and for NOTHING else. `encoding::utf8Decode` and
/// (since bug-809) `csv::parse` are two-form members selected by a PARAMETER
/// type, and `csv::stringify` has one form; none may grow the note.
#[test]
fn only_a_return_type_overload_gets_the_expected_type_note() {
    let encoding = registry().resolve_package("encoding").unwrap();
    let decode = render_function_markdown(encoding, encoding.function("utf8Decode").unwrap());
    assert!(decode.contains("## Overloads"));
    assert!(
        !decode.contains("differ only by what they return"),
        "utf8Decode's overloads differ by PARAMETER type; the note must not fire:\n{decode}"
    );

    let csv = registry().resolve_package("csv").unwrap();
    let parse = render_function_markdown(csv, csv.function("parse").unwrap());
    assert!(parse.contains("## Overloads"));
    assert!(!parse.contains("differ only by what they return"));
    let stringify = render_function_markdown(csv, csv.function("stringify").unwrap());
    assert!(stringify.contains("## Declaration"));
    assert!(!stringify.contains("differ only by what they return"));
}

/// The general pin bug-530 asks for: **every** registered form reaches the
/// page. A renderer that de-duplicated, collapsed or dropped a form would
/// print fewer declarations than the member has implementations, which is
/// exactly how the second `utf8Encode` form could go missing again.
#[test]
fn every_member_renders_one_declaration_per_implementation() {
    let mut checked = 0usize;
    for package in registry().packages() {
        let pkg = package.import_name();
        for function in package.functions() {
            if function.internal_only {
                continue;
            }
            let md = render_function_markdown(package, function);
            // Count declarations in the SIGNATURE SECTION only — the
            // description and examples quote calls too, and those are prose.
            let signatures = signature_section(&md);
            let needle = format!("`{pkg}::{}(", function.name);
            let rendered = signatures.matches(&needle).count();
            assert_eq!(
                rendered,
                function.implementations.len(),
                "{pkg}::{} registers {} implementation(s) but its page renders \
                     {rendered} declaration(s)",
                function.name,
                function.implementations.len()
            );
            checked += 1;
        }
    }
    assert!(checked > 400, "the census walked only {checked} members");
}

/// bug-609: `errorCode` registers 52 constants and nothing else, and its page
/// listed none of them — a developer at `mfb man` could not see which names
/// exist. The page lists each constant with its registered value and message.
#[test]
fn a_constants_only_package_lists_every_constant_with_value_and_message() {
    let package = registry().resolve_package("errorCode").unwrap();
    let md = render_package_markdown(package);
    assert!(md.contains("## Constants\n"), "no Constants section:\n{md}");
    assert!(
        md.contains(
            "| `errorCode::ErrPathNotFound` | `Integer` | `77030001` | Filesystem path does not exist. |"
        ),
        "no ErrPathNotFound row:\n{md}"
    );
    assert!(!md.contains("## Functions"));
}

/// bug-609: every registered constant of every package appears on that
/// package's overview, so a constant added later cannot go undiscoverable.
#[test]
fn every_registered_constant_is_listed_on_its_package_page() {
    let mut checked = 0;
    for package in registry()
        .packages()
        .iter()
        .filter(|package| !package.is_unqualified_global())
    {
        let md = render_package_markdown(package);
        for constant in package.constants() {
            let cell = format!("| `{}::{}` |", package.import_name(), constant.name);
            assert!(md.contains(&cell), "{cell} missing from its package page");
            checked += 1;
        }
    }
    assert!(checked > 100, "the census walked only {checked} constants");
}

/// bug-609: a record constant's value renders in the spelling a program
/// writes — a `pkg::Type[...]` construction — with a qualified type.
#[test]
fn a_record_constant_renders_as_a_qualified_record_construction() {
    let vector = registry().resolve_package("vector").unwrap();
    let md = render_package_markdown(vector);
    assert!(
        md.contains(
            "| `vector::zeroFloat3` | `vector::Float3` | `vector::Float3[0.0, 0.0, 0.0]` |"
        ),
        "no zeroFloat3 row:\n{md}"
    );
    let color = registry().resolve_package("color").unwrap();
    let md = render_package_markdown(color);
    assert!(
        md.contains("| `color::black` | `color::Color` | `color::Color[0, 0, 0, 255]` |"),
        "no black row:\n{md}"
    );
}

/// bug-609: the no-types page told the reader to "list its functions" for a
/// package that exports none. It names what the package does have.
#[test]
fn the_no_types_page_names_what_the_package_has() {
    let error_code = registry().resolve_package("errorCode").unwrap();
    let md = render_no_types_markdown(error_code);
    assert!(!md.contains("function"), "{md}");
    assert!(md.contains("to list its constants"), "{md}");
    // A package with functions and no types keeps its original wording.
    let bits = registry().resolve_package("bits").unwrap();
    assert!(!has_public_types(bits) && bits.constants().is_empty());
    assert!(render_no_types_markdown(bits).contains("to list its functions"));
}

/// bug-609: `mfb man errorCode ErrPathNotFound` answered "unknown errorCode
/// function" — true, but it sent the reader to a page that listed no
/// constants. A constant's name is recognised and routed to its package page.
#[test]
fn naming_a_constant_as_a_page_points_at_the_package_page() {
    let err = show_man(&s(&["errorCode", "ErrPathNotFound"])).unwrap_err();
    assert!(err.contains("is a constant"), "{err}");
    assert!(err.contains("mfb man errorCode"), "{err}");
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

/// bug-605: a record's field table spelled a package type the internal way
/// (`color.Color`), because `render_types_markdown` called `prop.ty.name()`
/// while every other render site converted the qualifier. Source writes
/// `color::Color`; the dotted form is field access and does not parse.
#[test]
fn types_page_field_tables_use_public_package_qualification() {
    let canvas = render_types_markdown(registry().resolve_package("canvas").unwrap());
    for want in [
        "`color::Color`",
        // plan-162-C: `Text.fonts` is a list of fonts; the element is still the
        // package-qualified resource.
        "`List OF RES canvas::Font`",
        "`RES canvas::Image`",
    ] {
        assert!(canvas.contains(want), "canvas types page lacks {want}");
    }
    let udp = render_types_markdown(registry().resolve_package("udp").unwrap());
    assert!(
        udp.contains("`net::Address`"),
        "udp types page lacks `net::Address`"
    );
    for page in [&canvas, &udp] {
        for dotted in ["color.Color", "canvas.Font", "canvas.Image", "net.Address"] {
            assert!(!page.contains(dotted), "types page still prints `{dotted}`");
        }
    }
}

/// bug-616: a descriptor says "this overload returns the same type as argument
/// n" with `ParameterType::Arg(n)`. That is an INTERNAL placeholder — a reader
/// cannot write `Arg0`, and no page ever defines it — so it must be resolved to
/// the n-th parameter's own type before it is rendered.
#[test]
fn a_declaration_resolves_an_arg_placeholder_to_the_parameters_type() {
    let math = registry().resolve_package("math").unwrap();
    let md = render_function_markdown(math, math.function("abs").unwrap());
    // The concrete overloads echo their own operand type.
    assert!(md.contains("`math::abs(value AS Float) AS Float`"), "{md}");
    assert!(md.contains("`math::abs(value AS Fixed) AS Fixed`"), "{md}");
    assert!(md.contains("`math::abs(value AS Money) AS Money`"), "{md}");
    assert!(
        md.contains("`math::abs(value AS List OF Integer) AS List OF Integer`"),
        "{md}"
    );

    // A GENERIC parameter resolves to the type variable, not to a concrete
    // type: `reduceRight` folds into its `initial` accumulator, so `Arg(1)`
    // is `U`. This is the case that proves the fix substitutes the
    // parameter's type rather than inventing one.
    let collections = registry().resolve_package("collections").unwrap();
    let md = render_function_markdown(collections, collections.function("reduceRight").unwrap());
    assert!(md.contains(") AS U`"), "{md}");
    assert!(md.contains("Returns `U`."), "{md}");

    // `Arg(0)` over a container parameter keeps the whole container.
    let md = render_function_markdown(collections, collections.function("append").unwrap());
    assert!(
        md.contains("`collections::append(value AS List OF T, item AS T) AS List OF T`"),
        "{md}"
    );
}

/// bug-616, TOTAL over the rendered surface, in the shape of
/// `no_rendered_page_spells_a_dotted_package_type` below: the placeholder is a
/// renderer-layer leak, so the gate is that NO page contains it — not that the
/// handful of pages found by hand were patched. 93 `ArgN` occurrences across 78
/// declaration lines and 15 "Returns"/wrapped lines were rendered before this.
#[test]
fn no_rendered_page_spells_an_arg_placeholder() {
    let mut hits = Vec::new();
    let mut scan = |page: &str, md: &str| {
        for (number, line) in md.lines().enumerate() {
            // `Arg` followed by a digit, at a word boundary — the exact
            // spelling `ParameterType::name` produces for `Arg(n)`.
            let bytes = line.as_bytes();
            let mut from = 0;
            while let Some(offset) = line[from..].find("Arg") {
                let at = from + offset;
                from = at + 3;
                let bounded =
                    at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
                if bounded && bytes.get(from).is_some_and(|b| b.is_ascii_digit()) {
                    hits.push(format!("{page}:{}: {line}", number + 1));
                }
            }
        }
    };
    for package in registry().packages() {
        let pkg = package.import_name();
        scan(
            &format!("{pkg} (overview)"),
            &render_package_markdown(package),
        );
        scan(&format!("{pkg} types"), &render_types_markdown(package));
        for function in package.functions() {
            scan(
                &format!("{pkg} {}", function.name),
                &render_function_markdown(package, function),
            );
        }
    }
    assert!(
        hits.is_empty(),
        "internal `ArgN` placeholders on rendered pages:\n{}",
        hits.join("\n")
    );
}

/// bug-605, TOTAL over the rendered surface: no page a developer reads may spell
/// a package type `<pkg>.<Type>`. Every render site goes through
/// `ParameterType::display()`; a new site that reaches for `name()` fails here
/// instead of shipping dots until someone notices.
#[test]
fn no_rendered_page_spells_a_dotted_package_type() {
    let names: Vec<&str> = registry()
        .packages()
        .iter()
        .map(|package| package.import_name())
        .collect();
    let mut hits = Vec::new();
    let mut scan = |page: &str, md: &str| {
        let bytes = md.as_bytes();
        for name in &names {
            let needle = format!("{name}.");
            let mut from = 0;
            while let Some(offset) = md[from..].find(&needle) {
                let at = from + offset;
                from = at + needle.len();
                let bounded =
                    at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
                let upper_leaf = bytes.get(from).is_some_and(|b| b.is_ascii_uppercase());
                if bounded && upper_leaf {
                    let start = md[..at].rfind('\n').map_or(0, |n| n + 1);
                    let end = md[at..].find('\n').map_or(md.len(), |n| at + n);
                    hits.push(format!("{page}: {}", &md[start..end]));
                }
            }
        }
    };
    for package in registry().packages() {
        let pkg = package.import_name();
        scan(
            &format!("{pkg} (overview)"),
            &render_package_markdown(package),
        );
        scan(&format!("{pkg} types"), &render_types_markdown(package));
        for function in package.functions() {
            scan(
                &format!("{pkg} {}", function.name),
                &render_function_markdown(package, function),
            );
        }
    }
    assert!(
        hits.is_empty(),
        "dotted package types on rendered pages:\n{}",
        hits.join("\n")
    );
}
