//! bug-784: destination-passing for an element field updated by a helper call —
//! `xs = set(xs, i, WITH p { inner := step(p.inner, …), … })` with `p` an
//! element-bound `get` binding.
//!
//! `step(p.inner, …)` builds a fresh `Inner` that the update then copies over
//! `p.inner`'s bytes: one allocation per update, even when the new value has the
//! old one's size. The callee already receives `p.inner` as a pointer to those very
//! bytes (an argument is lent, never copied), so a second lowering of the callee —
//! the **destination variant** `<base>$into<j>` — may write its result there
//! instead: at `RETURN WITH w { … }` over its parameter `j` it stores the new field
//! values into `w`'s bytes when the result has exactly `w`'s size (the field store
//! of `CodeBuilder::try_inplace_scalar_fields` over a span owner, which evaluates
//! every value before the first store) and returns `w`'s own address; otherwise it
//! leaves `w` untouched and returns a fresh block as the base does. The caller
//! tells the two apart by comparing the result with the destination address.
//!
//! This module answers which call sites may use a destination variant and which
//! `(callee, parameter)` pairs need one. Both questions are answered from the NIR
//! alone, by the same function, so the caller (which calls the variant symbol) and
//! the variant demand (which emits it) cannot disagree about a site — the same
//! discipline as `handover`. The builder adds the gates only it can answer (the
//! update really is lowered as an element field store; nothing after the call
//! reads the destination; a later failure cannot be observed) and simply calls the
//! base symbol when one fails, so a demanded variant may go unused but a called one
//! always exists.
//!
//! **Fail closed.** Anything unrecognised is not a site.

use std::collections::{HashMap, HashSet};

use crate::codegen::collection::layout::{record_field_is_pointer, type_contains_resource};
use crate::codegen::engine::analysis::borrow_get::{collect_borrow_gets, element_update_with};
use crate::codegen::engine::analysis::handover::call_key;
use crate::codegen::engine::analysis::last_use::op_key;
use crate::codegen::engine::builder::TypeModel;
use crate::target::shared::nir::visit::{walk_op, walk_value, NirVisitor};
use crate::target::shared::nir::{NirFunction, NirOp, NirValue};
use crate::types::ParameterType;

/// One destination-passing call site: the callee's NIR name and the index of the
/// argument that is the destination (`p.inner`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IntoSite {
    pub(crate) target: String,
    pub(crate) param: usize,
}

/// The destination-passing call sites of one function, by [`call_key`].
pub(crate) type IntoSites = HashMap<usize, IntoSite>;

/// The call sites of `function` that may call a destination variant: the
/// `inner := g(…, p.inner, …)` update of an element-bound binding `p`'s `WITH`
/// (`borrow_get::ElementBinding::update`), where `g` is a user function whose
/// parameter `j` — the argument that reads `p.inner` — qualifies
/// ([`into_param_eligible`]).
///
/// The element bindings come from `collect_borrow_gets` with "no call writes a
/// global", a superset of the lowering's set (the same answer `last_use` uses), so
/// every site the lowering can use is here; the extra ones only cost an unused
/// variant.
pub(crate) fn collect_into_sites(
    function: &NirFunction,
    model: &TypeModel,
    functions: &HashMap<String, &NirFunction>,
) -> IntoSites {
    let mut address_taken = HashSet::new();
    crate::codegen::engine::function::function_lowering::collect_address_taken_locals(
        &function.body,
        &mut address_taken,
    );
    let borrow = collect_borrow_gets(&function.body, &address_taken, &|_, _| false);
    // The update statements, by `op_key`, with the binding each one updates.
    let mut updates: HashMap<usize, &str> = HashMap::new();
    for (name, binding) in borrow.element_bindings() {
        for (update, _) in binding.sites() {
            if let Some(update) = update {
                updates.insert(update, name);
            }
        }
    }
    if updates.is_empty() {
        return IntoSites::new();
    }
    struct Finder<'a> {
        updates: &'a HashMap<usize, &'a str>,
        model: &'a TypeModel,
        functions: &'a HashMap<String, &'a NirFunction>,
        sites: IntoSites,
    }
    impl NirVisitor for Finder<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            // `borrow_get` registers an update only at `p = WITH p { … }`, at
            // `LET u = WITH p { … }`, and at the fused `c = set(c, i, WITH p { … })`
            // (an `Assign`, or a `StoreGlobal` with its value): each carries a value.
            match op {
                NirOp::Assign { value, .. }
                | NirOp::Bind {
                    value: Some(value), ..
                }
                | NirOp::StoreGlobal {
                    value: Some(value), ..
                } => {
                    if let Some(binding) = self.updates.get(&op_key(op)) {
                        self.sites_in(binding, element_update_with(value));
                    }
                }
                _ => {}
            }
            walk_op(self, op);
        }
    }
    impl Finder<'_> {
        fn sites_in(&mut self, binding: &str, with: &NirValue) {
            let (type_, updates) = match with {
                NirValue::WithUpdate {
                    type_,
                    target,
                    updates,
                } if matches!(target.as_ref(), NirValue::Local(name) if name == binding) => {
                    (type_, updates)
                }
                _ => return,
            };
            let Some(fields) = self.model.record_fields.get(type_) else {
                return;
            };
            for update in updates {
                let NirValue::Call {
                    target: callee,
                    args,
                    ..
                } = &update.value
                else {
                    continue;
                };
                let Some(field_type) = fields
                    .iter()
                    .find(|(name, _)| *name == update.field)
                    .map(|(_, field_type)| field_type)
                else {
                    continue;
                };
                let Some(function) = self.functions.get(callee.as_str()) else {
                    continue;
                };
                let Some(param) = args.iter().position(|arg| {
                    matches!(arg, NirValue::MemberAccess { target, member }
                        if *member == update.field
                            && matches!(target.as_ref(), NirValue::Local(name) if name == binding))
                }) else {
                    continue;
                };
                if function.params.get(param).map(|p| &p.type_) == Some(field_type)
                    && into_param_eligible(function, param, self.model)
                {
                    self.sites.insert(
                        call_key(&update.value),
                        IntoSite {
                            target: callee.clone(),
                            param,
                        },
                    );
                }
            }
        }
    }
    let mut finder = Finder {
        updates: &updates,
        model,
        functions,
        sites: IntoSites::new(),
    };
    finder.visit_ops(&function.body);
    finder.sites
}

/// Whether `function` gets a destination variant for parameter `param`: the
/// parameter's type is the function's return type, a flat record
/// ([`flat_record`]); the body never assigns the parameter or takes its address
/// (so the parameter's slot holds the caller's destination for the whole call);
/// and some `RETURN WITH <param> { … }` exists for the variant to serve — every
/// other return builds a fresh block exactly as the base does.
pub(crate) fn into_param_eligible(function: &NirFunction, param: usize, model: &TypeModel) -> bool {
    let Some(param) = function.params.get(param) else {
        return false;
    };
    if param.type_ != function.returns || !flat_record(model, &param.type_) {
        return false;
    }
    struct Scan<'a> {
        name: &'a str,
        written: bool,
        returned: bool,
    }
    impl NirVisitor for Scan<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Assign { name, .. } if name == self.name => self.written = true,
                NirOp::Bind { name, .. } if name == self.name => self.written = true,
                NirOp::Return {
                    value: Some(NirValue::WithUpdate { target, .. }),
                } if matches!(target.as_ref(), NirValue::Local(n) if n == self.name) => {
                    self.returned = true;
                }
                _ => {}
            }
            walk_op(self, op);
        }

        fn visit_value(&mut self, value: &NirValue) {
            if matches!(value, NirValue::LocalRef { name, .. } if name == self.name) {
                self.written = true;
            }
            walk_value(self, value);
        }
    }
    let mut scan = Scan {
        name: &param.name,
        written: false,
        returned: false,
    };
    scan.visit_ops(&function.body);
    !scan.written && scan.returned
}

/// A record whose bytes are its whole value: every field a scalar slot or a
/// `String` — no pointer, collection or resource anywhere, and not `Error` (whose
/// `source` sub-block may be absent, bug-371).
///
/// A nested record is never part of such a record's bytes: a record, union,
/// collection, `Result`, `Error` or function field is a pointer slot
/// (`record_field_is_pointer`), and the one composite a record holds inline
/// without a pointer, a `RES` resource-union box (`field_is_inlined_union_box`),
/// contains a resource. So "flat" needs no recursion.
fn flat_record(model: &TypeModel, type_: &ParameterType) -> bool {
    if type_.is_named("Error") || type_contains_resource(model, type_) {
        return false;
    }
    let Some(fields) = model.record_fields.get(type_) else {
        return false;
    };
    fields
        .iter()
        .all(|(_, field)| !record_field_is_pointer(model, field))
}

/// The `(callee, parameter)` pairs the module's sites need a destination variant
/// for.
pub(crate) fn demanded_into_variants(
    module_functions: &[NirFunction],
    model: &TypeModel,
    functions: &HashMap<String, &NirFunction>,
) -> HashSet<(String, usize)> {
    module_functions
        .iter()
        .flat_map(|function| collect_into_sites(function, model, functions).into_values())
        .map(|site| (site.target, site.param))
        .collect()
}
