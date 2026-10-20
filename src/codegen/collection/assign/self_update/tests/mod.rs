use std::collections::BTreeSet;

use super::*;
use crate::codegen::registry::{registry, self_update_shaped};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, CodeTarget};

mod census;
use census::*;

/// Rows with no registry function behind them: the operator self-updates, the
/// unqualified `toString` (plan-146-A), and `&` on two `AttributedString`s,
/// which reaches NIR as a call to `#astrings_concat` (plan-143 findings F5).
const NON_REGISTRY: &[&str] = &["&", "toString", "&@AttributedString"];

/// The `@AttributedString` row spelling of a Tier-B transform: the
/// `AttributedString` "overload" of a `strings::` member, typed by
/// `strings::resolve_return_type` rather than registered (plan-143 Correction 1).
fn tier_b_row(member: &str) -> String {
    format!("{}@AttributedString", member.replacen('.', "::", 1))
}

/// Every function with at least one self-update-shaped overload, as `pkg::name`.
/// An `internal_only` member is outside it: user source can never call it
/// (`astrings::writeSpans`, plan-146-A Correction A1).
fn shaped_functions() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for package in registry().packages() {
        for function in package.functions() {
            if !function.internal_only && function.implementations.iter().any(self_update_shaped) {
                out.insert(format!("{}::{}", package.import_name(), function.name));
            }
        }
    }
    out
}

/// plan-145-A: every field kind a record can hold, classified by the compiler —
/// `(kind, class)`, the kind spelled as its type renders (`vector.Float3`).
///
/// The population is read off a program that imports every importable package
/// (in app mode: `canvas` requires it) and declares one record holding each
/// builtin kind: every EXPORTed package record type, the builtin scalars,
/// `String`, `AttributedString`, `json::Json` and the three collections.
fn field_kind_census() -> Vec<(String, FieldKindClass)> {
    let mut src = String::new();
    for package in registry().packages() {
        if !package.is_unqualified_global() {
            src.push_str(&format!("IMPORT {}\n", package.import_name()));
        }
    }
    src.push_str(
        "\nTYPE FieldKinds\n  i AS Integer\n  f AS Float\n  x AS Fixed\n  m AS Money\n  \
             b AS Boolean\n  y AS Byte\n  s AS String\n  t AS AttributedString\n  \
             j AS json::Json\n  l AS List OF Integer\n  p AS Map OF String TO Integer\n  \
             e AS Set OF Integer\nEND TYPE\n\nFUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
    );
    let target = CodeTarget::MacosAarch64;
    let mode = target.app_mode().expect("macOS has an app mode");
    let nir = crate::testutil::nir_for_src(&src, target, mode).expect("the census lowers");
    let model = TypeModel::from_module(&nir).expect("the census model builds");
    let mut kinds = Vec::new();
    for t in &nir.types {
        if t.name == "FieldKinds" {
            for field in &t.fields {
                kinds.push((
                    field.type_.name().into_owned(),
                    field_kind_class(&model, &field.type_),
                ));
            }
        } else if t.kind == "type" && t.visibility == "export" {
            let ty = ParameterType::named(&t.name);
            kinds.push((t.name.clone(), field_kind_class(&model, &ty)));
        }
    }
    kinds
}

#[test]
fn field_kind_census_covers_every_record_field_type() {
    let census = field_kind_census();
    let mut failures = Vec::new();
    for (kind, class) in &census {
        let Some((_, row)) = FIELD_KIND_TABLE.iter().find(|(k, _)| k == kind) else {
            failures.push(format!(
                "{kind} ({class:?}) has no FIELD_KIND_TABLE row — classify it: `Arm(letter)`, \
                     `Rebuild {{ reason, proof }}` or `Deferred(plan)`"
            ));
            continue;
        };
        let consistent = match row {
            FieldKindRow::Arm(letter) => {
                "BCDEFGH".contains(*letter)
                    && matches!(
                        class,
                        FieldKindClass::Scalar
                            | FieldKindClass::Pointer
                            | FieldKindClass::InlinedFixed
                            | FieldKindClass::Collection
                    )
            }
            FieldKindRow::Rebuild { reason, proof } => {
                *class == FieldKindClass::InlinedVariable
                    && !reason.trim().is_empty()
                    && !proof.trim().is_empty()
            }
            FieldKindRow::Deferred(plan) => !plan.trim().is_empty(),
        };
        if !consistent {
            failures.push(format!(
                "{kind}: row {row:?} does not fit its class {class:?}"
            ));
        }
    }
    for (kind, _) in FIELD_KIND_TABLE {
        if !census.iter().any(|(k, _)| k == kind) {
            failures.push(format!("FIELD_KIND_TABLE row {kind} names no field kind"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// plan-142-B Phase 1: every spelling a shrink self-update's call target has
/// after lowering (recorded from a `--nir` build in plan-142-B) resolves to its
/// bare name, and nothing else does.
#[test]
fn self_update_builtin_names_every_spelling() {
    // `Body::abi_inline` / `Body::Intrinsic` members keep the qualified name.
    assert_eq!(self_update_builtin("collections.filter"), Some("filter"));
    assert_eq!(self_update_builtin("collections.mid"), Some("mid"));
    assert_eq!(self_update_builtin("collections.append"), Some("append"));
    // plan-153-B: `collections.repeat` and `strings.repeat` share the bare name;
    // the collection arm declines a `String` site and the string arm a `List`.
    assert_eq!(self_update_builtin("collections.repeat"), Some("repeat"));
    assert_eq!(self_update_builtin("strings.repeat"), Some("repeat"));
    // `Body::Mfb` members arrive as their internalized monomorph.
    assert_eq!(
        self_update_builtin("#collections_take$Integer"),
        Some("take")
    );
    assert_eq!(
        self_update_builtin("#collections_take$String"),
        Some("take")
    );
    assert_eq!(
        self_update_builtin("#collections_drop$Integer"),
        Some("drop")
    );
    assert_eq!(
        self_update_builtin("#collections_distinct$Integer"),
        Some("distinct")
    );
    // … or, before monomorphization, qualified.
    assert_eq!(self_update_builtin("collections.take"), Some("take"));
    // plan-146-B: every spelling of a `String` row plan-146 arms (C, D, E) and
    // of `toString`, from the `--nir` probe (plan-146-B Phase 1).
    for (target, bare) in [
        ("strings.left", "left"),
        ("strings.right", "right"),
        ("strings.mid", "mid"),
        ("strings.stripPrefix", "stripPrefix"),
        ("strings.stripSuffix", "stripSuffix"),
        ("strings.trim", "trim"),
        ("strings.trimStart", "trimStart"),
        ("strings.trimEnd", "trimEnd"),
        ("strings.trimChars", "trimChars"),
        ("strings.graphemeAt", "graphemeAt"),
        ("fs.pathBaseName", "pathBaseName"),
        ("fs.pathDirName", "pathDirName"),
        ("fs.pathExtension", "pathExtension"),
        ("strings.padLeft", "padLeft"),
        ("strings.padRight", "padRight"),
        ("#strings_padLeftToWidth", "padLeftToWidth"),
        ("#strings_padRightToWidth", "padRightToWidth"),
        ("strings.repeat", "repeat"),
        ("os.appResourcePath", "appResourcePath"),
        ("os.appDataPath", "appDataPath"),
        ("os.appCachePath", "appCachePath"),
        ("os.userHomePath", "userHomePath"),
        ("os.userDocumentsPath", "userDocumentsPath"),
        ("strings.upper", "upper"),
        ("strings.lower", "lower"),
        ("strings.caseFold", "caseFold"),
        ("strings.normalizeNfc", "normalizeNfc"),
        ("strings.replace", "replace"),
        ("fs.pathNormalize", "pathNormalize"),
        ("toString", "toString"),
    ] {
        assert_eq!(self_update_builtin(target), Some(bare), "{target}");
    }
    assert_eq!(self_update_builtin("#strings_nope"), None);
    // A Rewrite helper that is not a row stays invisible.
    assert_eq!(self_update_builtin("#strings_padToWidthCopies"), None);
    // Not a collections builtin.
    assert_eq!(self_update_builtin("#collections_nope$Integer"), None);
    assert_eq!(self_update_builtin("#json_parse"), None);
    assert_eq!(self_update_builtin("take"), None);
    assert_eq!(self_update_builtin("userFunction$Integer"), None);
}

#[test]
fn self_update_census_covers_every_registry_overload() {
    let rows: BTreeSet<&str> = SELF_UPDATE_TABLE.iter().map(|r| r.function).collect();
    let missing: Vec<String> = shaped_functions()
        .into_iter()
        .filter(|f| !rows.contains(f.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "self-update-shaped builtin(s) with no SELF_UPDATE_TABLE row — add an `Arm` \
             (an in-place lowering) or an `Exempt` with its proof: {missing:?}"
    );
}

/// plan-146-A: the Tier-B `AttributedString` transforms are a second census
/// source — `mfb man` and the registry cannot see them.
#[test]
fn tier_b_transforms_have_rows() {
    let rows: BTreeSet<&str> = SELF_UPDATE_TABLE.iter().map(|r| r.function).collect();
    let missing: Vec<String> = crate::codegen::builtins::strings::tier_b_transforms()
        .iter()
        .map(|(member, _)| tier_b_row(member))
        .filter(|row| !rows.contains(row.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "Tier-B AttributedString transform(s) with no SELF_UPDATE_TABLE row: {missing:?}"
    );
}

#[test]
fn self_update_table_has_no_stale_rows() {
    let shaped = shaped_functions();
    let tier_b: BTreeSet<String> = crate::codegen::builtins::strings::tier_b_transforms()
        .iter()
        .map(|(member, _)| tier_b_row(member))
        .collect();
    let arms: BTreeSet<ArmId> = SELF_UPDATE_ARMS.iter().map(|(id, _, _)| *id).collect();
    assert_eq!(
        arms.len(),
        SELF_UPDATE_ARMS.len(),
        "an arm id appears twice in SELF_UPDATE_ARMS"
    );
    let mut seen = BTreeSet::new();
    let mut referenced = BTreeSet::new();
    for row in SELF_UPDATE_TABLE {
        assert!(seen.insert(row.function), "duplicate row {}", row.function);
        assert!(
            shaped.contains(row.function)
                || NON_REGISTRY.contains(&row.function)
                || tier_b.contains(row.function),
            "row {} names no registry function with a self-update-shaped overload",
            row.function
        );
        assert!(!row.probes.is_empty(), "row {} has no probe", row.function);
        if let SelfUpdate::Exempt { reason, proof } = row.kind {
            assert!(
                !reason.trim().is_empty() && !proof.trim().is_empty(),
                "row {} is Exempt without a reason and a proof",
                row.function
            );
        }
        if let SelfUpdate::Deferred(plan) = row.kind {
            assert!(
                !plan.trim().is_empty(),
                "row {} is Deferred without naming the plan that owns it",
                row.function
            );
        }
        if let SelfUpdate::Arm(ids) = row.kind {
            assert!(!ids.is_empty(), "row {} lists no arm", row.function);
            for id in ids {
                assert!(
                    arms.contains(id),
                    "row {} names {id:?}, which is not in SELF_UPDATE_ARMS",
                    row.function
                );
                referenced.insert(*id);
            }
        }
    }
    let unreferenced: Vec<&ArmId> = arms.difference(&referenced).collect();
    assert!(
        unreferenced.is_empty(),
        "SELF_UPDATE_ARMS entries no row names: {unreferenced:?}"
    );
}

#[test]
fn every_arm_row_fires_at_every_enabled_site() {
    let arms: BTreeSet<ArmId> = SELF_UPDATE_ARMS.iter().map(|(id, _, _)| *id).collect();
    let never = |id: ArmId, probe: &Probe, site: Site| {
        // plan-147-B: S11's exclusions are their own list — `FIELD_NEVER` is
        // about field sites, and `Return` is not one.
        // S12 is S11 inside an owned variant and S11F is its field form, so both
        // inherit S11's exclusions: a `String` block must be tight to leave its
        // frame however the return is spelled.
        if matches!(site, Site::Return | Site::OwnedParam | Site::S11F)
            && RETURN_NEVER.iter().any(|(arm, _)| *arm == id)
        {
            return true;
        }
        // plan-147-F: S12 additionally has no owned variant to lower a `String`
        // arm in, because a bare `String` parameter is never handed over.
        if site == Site::OwnedParam && OWNED_PARAM_NEVER.iter().any(|(arm, _)| *arm == id) {
            return true;
        }
        FIELD_NEVER.iter().any(|(arm, ty, sites, _)| {
            *arm == id
                && (ty.is_empty() || *ty == probe.ty)
                && sites.contains(&site.code().as_str())
        })
    };
    let mut failures = Vec::new();
    for row in SELF_UPDATE_TABLE {
        let SelfUpdate::Arm(ids) = row.kind else {
            continue;
        };
        for &site in ENABLED_SITES.iter().chain(FIELD_SITES) {
            let field = FIELD_SITES.contains(&site);
            let mut fired = BTreeSet::new();
            // The ids some compiled probe is expected to be able to fire.
            let mut reachable = BTreeSet::new();
            for probe in row.probes {
                let Some(src) = probe.source(site) else {
                    continue;
                };
                let code = code_for_src_cached(&src, CodeTarget::LinuxX86_64, Console);
                let hit: Vec<ArmId> = ids
                    .iter()
                    .copied()
                    .filter(|id| {
                        code.functions
                            .iter()
                            .filter(|function| site.lowers_in(&function.name))
                            .flat_map(|function| &function.stack_slots)
                            .any(|slot| id.markers().contains(&slot.type_.as_str()))
                    })
                    .collect();
                for id in &hit {
                    if never(*id, probe, site) {
                        failures.push(format!(
                            "{} at {site:?}: `x = {}` fired {id:?}, which the never-list \
                                 says it does not do there. If that is now WRONG, re-measure \
                                 the site and move the entry out with the numbers.",
                            row.function, probe.call
                        ));
                    }
                }
                // plan-147-B: only the ids that are supposed to be able to fire
                // here count. When every id of the row is on a never-list for
                // this site, the probe firing nothing is the expectation.
                let expected: Vec<ArmId> = ids
                    .iter()
                    .copied()
                    .filter(|id| !never(*id, probe, site))
                    .collect();
                if !field && hit.is_empty() && !expected.is_empty() {
                    failures.push(format!(
                        "{} at {site:?}: `x = {}` fired none of {expected:?}",
                        row.function, probe.call
                    ));
                }
                reachable.extend(ids.iter().copied().filter(|id| !never(*id, probe, site)));
                fired.extend(hit);
            }
            for id in ids {
                if !reachable.contains(id) {
                    continue;
                }
                // plan-145-I: every pair not in `FIELD_NEVER` must fire — a field
                // self-update needs an in-place lowering, a `Rebuild` row with its
                // proof, or a deferral to a named plan.
                if !fired.contains(id) {
                    failures.push(format!(
                        "{} at {site:?}: no probe fired {id:?}",
                        row.function
                    ));
                }
                if !arms.contains(id) {
                    failures.push(format!(
                        "{} at {site:?}: {id:?} is not dispatched (not in SELF_UPDATE_ARMS)",
                        row.function
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// plan-147-B: every `RETURN_NEVER` entry names a real dispatched arm, carries a
/// reason, and appears once. Keeps the S11 exclusion list from drifting away from
/// `SELF_UPDATE_ARMS`.
#[test]
fn return_never_names_dispatched_arms_once() {
    let arms: BTreeSet<ArmId> = SELF_UPDATE_ARMS.iter().map(|(id, _, _)| *id).collect();
    let mut seen = BTreeSet::new();
    for (arm, reason) in RETURN_NEVER {
        assert!(
            !reason.trim().is_empty(),
            "RETURN_NEVER {arm:?} has no reason"
        );
        assert!(
            arms.contains(arm),
            "RETURN_NEVER {arm:?} is not in SELF_UPDATE_ARMS"
        );
        assert!(seen.insert(*arm), "RETURN_NEVER lists {arm:?} twice");
    }
    // plan-147-F: the same three properties for S12's own list, and it may not
    // repeat a `RETURN_NEVER` row — that would be a second place to maintain the
    // same exclusion.
    let mut owned_seen = BTreeSet::new();
    for (arm, reason) in OWNED_PARAM_NEVER {
        assert!(
            !reason.trim().is_empty(),
            "OWNED_PARAM_NEVER {arm:?} has no reason"
        );
        assert!(
            arms.contains(arm),
            "OWNED_PARAM_NEVER {arm:?} is not in SELF_UPDATE_ARMS"
        );
        assert!(
            owned_seen.insert(*arm),
            "OWNED_PARAM_NEVER lists {arm:?} twice"
        );
        assert!(
            !seen.contains(arm),
            "OWNED_PARAM_NEVER {arm:?} is already excluded by RETURN_NEVER"
        );
    }
}

/// plan-145-A: `FIELD_NEVER` names only field sites, and a pair appears in it
/// at most once.
#[test]
fn field_never_names_field_sites_once() {
    let codes: Vec<String> = FIELD_SITES.iter().map(|s| s.code()).collect();
    let mut seen = BTreeSet::new();
    let mut failures = Vec::new();
    for (arm, ty, sites, reason) in FIELD_NEVER {
        assert!(
            !reason.trim().is_empty(),
            "FIELD_NEVER {arm:?} has no reason"
        );
        for site in *sites {
            if !codes.iter().any(|c| c == site) {
                failures.push(format!("FIELD_NEVER {arm:?} names no field site {site}"));
            }
            if !seen.insert((*arm, site.to_string(), ty.to_string())) {
                failures.push(format!("FIELD_NEVER lists {arm:?} ({ty}) at {site} twice"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// plan-145-I: an arm that serves no field site (`FieldReach::None` — today the
/// `String` concat, plan-145-A Open Decision 1) must be declared so, not left
/// to fail quietly: `FIELD_NEVER` lists it at every field site, and every
/// `field_expect.tsv` line of its rows is a deferral to a named plan (or `na:`,
/// where the program does not compile).
#[test]
fn an_arm_with_no_field_reach_is_deferred_at_every_field_site() {
    const FIELD_EXPECT: &str =
        include_str!("../../../../../../tests/runtime/inplace_self_update/field_expect.tsv");
    let mut failures = Vec::new();
    for (id, _, reach) in SELF_UPDATE_ARMS {
        if *reach != FieldReach::None {
            continue;
        }
        for site in FIELD_SITES {
            let code = site.code();
            if !FIELD_NEVER
                .iter()
                .any(|(arm, _, sites, _)| arm == id && sites.contains(&code.as_str()))
            {
                failures.push(format!(
                    "{id:?} has FieldReach::None but FIELD_NEVER does not list it at {code}"
                ));
            }
        }
        let rows = SELF_UPDATE_TABLE
            .iter()
            .filter(|row| matches!(row.kind, SelfUpdate::Arm(ids) if ids.contains(id)));
        for row in rows {
            let prefixes = [format!("{}(", row.function), format!("{} (", row.function)];
            let lines: Vec<&str> = FIELD_EXPECT
                .lines()
                .filter(|line| !line.starts_with('#'))
                .filter(|line| prefixes.iter().any(|p| line.starts_with(p.as_str())))
                .collect();
            if lines.is_empty() {
                failures.push(format!(
                    "{id:?}: field_expect.tsv has no line for row {}",
                    row.function
                ));
            }
            for line in lines {
                let expect = line.rsplit('\t').next().unwrap_or("");
                // `na:` — the program does not compile there at all.
                if !expect.starts_with("deferred:") && !expect.starts_with("na:") {
                    failures.push(format!(
                        "{id:?} has FieldReach::None, but field_expect.tsv says `{line}` — \
                             a field self-update needs an in-place lowering, a `Rebuild` row \
                             with its proof, or a deferral to a named plan"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
