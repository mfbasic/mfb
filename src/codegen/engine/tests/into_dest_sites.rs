//! bug-784's destination-variant analysis (`engine/analysis/into_dest.rs`), asked
//! directly over the NIR of real programs: which `(callee, parameter)` pairs get a
//! `<base>$into<j>` variant, and which element-update call sites may call one.
//!
//! The lowering suites (`element_update.rs`, `element_update_forms.rs`) see only the
//! sites that DO get a variant (`into_result_kept`). What this pins is the other
//! half, the "fail closed" contract the module states: a callee whose parameter is
//! not a flat record of its return type, that writes the parameter, or that never
//! `RETURN WITH`s it gets no variant, and a site whose shape is not exactly
//! `WITH p { f := g(…, p.f, …) }` over the element binding is not a site. A variant
//! called for any of them would write a fresh value's bytes over the caller's
//! element.

use crate::codegen::engine::analysis::into_dest::{
    collect_into_sites, demanded_into_variants, into_param_eligible,
};
use crate::codegen::engine::builder::TypeModel;
use crate::target::shared::nir::{NirFunction, NirModule, NirOp, NirValue};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;
use std::collections::{HashMap, HashSet};

const SOURCE: &str = "\
IMPORT collections
IMPORT fs
IMPORT io
IMPORT strings

TYPE Inner
  a AS Integer
  b AS Integer
  name AS String
END TYPE

TYPE Deep
  inner AS Inner
  n AS Integer
END TYPE

TYPE Listy
  xs AS List OF Integer
  n AS Integer
END TYPE

TYPE Boxed
  err AS Error
  n AS Integer
END TYPE

TYPE Logged
  f AS RES fs::File
  n AS Integer
END TYPE

TYPE Holder
  id AS Integer
  label AS String
  inner AS Inner
  deep AS Deep
  listy AS Listy
  boxed AS Boxed
  count AS Integer
END TYPE

FUNC stepIn(w AS Inner, i AS Integer) AS Inner
  RETURN WITH w { a := w.a + i }
END FUNC

FUNC fresh(w AS Inner, i AS Integer) AS Inner
  RETURN Inner[a := i, b := w.b, name := w.name]
END FUNC

FUNC lead(i AS Integer, w AS Inner) AS Inner
  RETURN WITH w { b := i }
END FUNC

FUNC stepDeep(d AS Deep) AS Deep
  RETURN WITH d { n := d.n + 1 }
END FUNC

FUNC stepListy(l AS Listy) AS Listy
  RETURN WITH l { n := l.n + 1 }
END FUNC

FUNC stepBoxed(e AS Boxed) AS Boxed
  RETURN WITH e { n := e.n + 1 }
END FUNC

FUNC bump(l AS Logged) AS Logged
  RETURN WITH l { n := l.n + 1 }
END FUNC

FUNC inc(x AS Integer) AS Integer
  RETURN x + 1
END FUNC

FUNC holders() AS List OF Holder
  LET inner AS Inner = Inner[a := 1, b := 2, name := \"x\"]
  RETURN [Holder[id := 1, label := \"h\", inner := inner, deep := Deep[inner := inner, n := 0], listy := Listy[xs := [1], n := 0], boxed := Boxed[err := error(1, \"e\"), n := 0], count := 0]]
END FUNC

FUNC main() AS Integer
  MUT hs AS List OF Holder = holders()
  FOR k = 0 TO 0
    LET p = collections::get(hs, k)
    hs = collections::set(hs, k, WITH p { inner := stepIn(p.inner, k), deep := stepDeep(p.deep), listy := stepListy(p.listy), boxed := stepBoxed(p.boxed), count := inc(p.count), label := strings::upper(p.label), id := p.id + 1 })
  NEXT
  FOR k = 0 TO 0
    LET q = collections::get(hs, k)
    hs = collections::set(hs, k, WITH q { inner := fresh(q.inner, k) })
  NEXT
  FOR k = 0 TO 0
    LET r = collections::get(hs, k)
    hs = collections::set(hs, k, WITH r { inner := lead(k, r.inner) })
  NEXT
  FOR k = 0 TO 0
    LET s = collections::get(hs, k)
    hs = collections::set(hs, k, WITH s { inner := stepIn(fresh(s.inner, k), k) })
  NEXT
  LET logged AS Logged = Logged[f := fs::createTempFile(), n := 0]
  LET bumped AS Logged = bump(logged)
  io::print(toString(len(hs) + bumped.n))
  RETURN 0
END FUNC
";

fn module() -> NirModule {
    nir_for_src(SOURCE, CodeTarget::LinuxX86_64, Console)
        .unwrap_or_else(|error| panic!("the probe lowers to NIR: {error}"))
}

fn function<'m>(module: &'m NirModule, name: &str) -> &'m NirFunction {
    module
        .functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no function `{name}`"))
}

fn by_name(module: &NirModule) -> HashMap<String, &NirFunction> {
    module
        .functions
        .iter()
        .map(|f| (f.name.clone(), f))
        .collect()
}

/// The `(callee, parameter)` pairs `function`'s sites name, sorted.
fn sites_of(function: &NirFunction, model: &TypeModel, module: &NirModule) -> Vec<(String, usize)> {
    let mut sites: Vec<(String, usize)> = collect_into_sites(function, model, &by_name(module))
        .into_values()
        .map(|site| (site.target, site.param))
        .collect();
    sites.sort();
    sites
}

/// Only a parameter that is a flat record of the function's own return type,
/// returned through `RETURN WITH <param>`, gets a destination variant. Every other
/// row is a reason the variant would be wrong: it would write a value with
/// pointer, collection, `Error` or resource fields as raw bytes over the caller's
/// element, or serve a function with no `RETURN WITH` to write.
#[test]
fn only_a_flat_record_parameter_returned_through_with_is_eligible() {
    let module = module();
    let model = TypeModel::from_module(&module).expect("the probe's type model builds");
    let rows: &[(&str, usize, bool, &str)] = &[
        ("stepIn", 0, true, "flat Inner, RETURN WITH w"),
        (
            "stepIn",
            1,
            false,
            "an Integer parameter is not the Inner return type",
        ),
        ("stepIn", 7, false, "there is no parameter 7"),
        (
            "lead",
            1,
            true,
            "the record need not be the first parameter",
        ),
        (
            "fresh",
            0,
            false,
            "no RETURN WITH w: every return is a fresh block",
        ),
        (
            "stepDeep",
            0,
            false,
            "a nested record field is a pointer slot",
        ),
        ("stepListy", 0, false, "a collection field"),
        ("stepBoxed", 0, false, "an Error field"),
        ("bump", 0, false, "a resource field"),
        ("inc", 0, false, "an Integer is not a record at all"),
    ];
    for (name, param, expected, why) in rows {
        assert_eq!(
            into_param_eligible(function(&module, name), *param, &model),
            *expected,
            "{name} parameter {param}: {why}"
        );
    }
}

/// A body that writes the parameter, or takes its address, no longer holds the
/// caller's destination in that slot for the whole call, so the variant is
/// refused even though `RETURN WITH w` is still there. No program spells this
/// (a parameter is immutable), so the write is put into `stepIn`'s NIR directly.
#[test]
fn a_parameter_the_body_writes_or_borrows_is_not_eligible() {
    let module = module();
    let model = TypeModel::from_module(&module).expect("the probe's type model builds");
    let original = function(&module, "stepIn");
    assert!(into_param_eligible(original, 0, &model));
    let inner = ParameterType::declared("Inner");
    let writes: Vec<(&str, NirOp)> = vec![
        (
            "an assignment",
            NirOp::Assign {
                name: "w".to_string(),
                value: NirValue::Local("w".to_string()),
            },
        ),
        (
            "a re-binding",
            NirOp::Bind {
                mutable: true,
                name: "w".to_string(),
                type_: inner.clone(),
                value: None,
            },
        ),
        (
            "a by-reference read",
            NirOp::Eval {
                value: NirValue::LocalRef {
                    name: "w".to_string(),
                    type_: inner.clone(),
                },
            },
        ),
    ];
    let mut module = module;
    for (what, op) in writes {
        let index = module
            .functions
            .iter()
            .position(|f| f.name == "stepIn")
            .expect("stepIn");
        let saved = module.functions[index].body.clone();
        module.functions[index].body.insert(0, op);
        assert!(
            !into_param_eligible(&module.functions[index], 0, &model),
            "{what} of the parameter left stepIn eligible"
        );
        module.functions[index].body = saved;
    }
}

/// Of `main`'s update calls only those of an eligible callee that read the
/// element binding's own field at the eligible position are sites: not
/// `fresh(q.inner, k)` (ineligible callee), not `strings::upper(p.label)` (no
/// user function), not `p.id + 1` (no call), not `stepIn(fresh(s.inner, k), k)`
/// (the destination argument is not `s.inner`), not the ineligible record fields.
/// `lead(k, r.inner)` is one, at parameter 1.
#[test]
fn only_an_eligible_call_reading_the_bindings_own_field_is_a_site() {
    let module = module();
    let model = TypeModel::from_module(&module).expect("the probe's type model builds");
    assert_eq!(
        sites_of(function(&module, "main"), &model, &module),
        vec![("lead".to_string(), 1), ("stepIn".to_string(), 0)]
    );
    let demanded = demanded_into_variants(&module.functions, &model, &by_name(&module));
    assert_eq!(
        demanded,
        HashSet::from([("lead".to_string(), 1), ("stepIn".to_string(), 0)])
    );
}

/// The finder fails closed on what it cannot place: with a type model that does
/// not know the element's record type, no update of it is a site; and an update
/// whose field the record does not have is skipped while the other one still is.
/// (Neither comes from a well-formed module — the lowering hands the finder the
/// module's own model — so the second is built by renaming one field in `main`.)
#[test]
fn an_update_the_model_cannot_place_is_not_a_site() {
    let mut module = module();
    let model = TypeModel::from_module(&module).expect("the probe's type model builds");
    assert!(sites_of(function(&module, "main"), &TypeModel::empty(), &module).is_empty());
    let index = module
        .functions
        .iter()
        .position(|f| f.name == "main")
        .expect("main");
    let mut renamed = 0;
    rename_inner_updates(&mut module.functions[index].body, &mut renamed);
    assert_eq!(renamed, 2, "the probe has two `inner := stepIn(…)` updates");
    assert_eq!(
        sites_of(function(&module, "main"), &model, &module),
        vec![("lead".to_string(), 1)]
    );
}

/// Rename the field of the `inner := stepIn(…)` update in every
/// `x = collections.set(x, i, WITH …)` of `ops` to one the record lacks.
fn rename_inner_updates(ops: &mut [NirOp], renamed: &mut usize) {
    for op in ops {
        match op {
            NirOp::Assign {
                value: NirValue::Call { target, args, .. },
                ..
            } if target == "collections.set" && args.len() == 3 => {
                if let NirValue::WithUpdate { updates, .. } = &mut args[2] {
                    for update in updates {
                        if update.field == "inner"
                            && matches!(&update.value, NirValue::Call { target, .. } if target == "stepIn")
                        {
                            update.field = "noSuchField".to_string();
                            *renamed += 1;
                        }
                    }
                }
            }
            NirOp::For { body, .. } => rename_inner_updates(body, renamed),
            _ => {}
        }
    }
}
