//! Name-usage census over a NIR function body — the fact base for tree-level
//! DCE. Rides the sanctioned read-only traversal seam ([`nir::visit`],
//! bug-328) so it cannot drift from the one authoritative recursion.
//!
//! The census is deliberately scope-blind: it counts every occurrence of a
//! name anywhere in the body (reads via `Local`/`LocalRef`, writes via
//! `Assign`/`StateAssign`, and every re-binding position — nested `Bind`s,
//! loop variables, `TRAP` bindings). NIR keeps source names under shadowing,
//! so a per-scope census could confuse two bindings of the same name; the
//! global count is conservative instead: a binding is provably unused only
//! when its name occurs nowhere else at all.

use std::collections::HashMap;

use crate::target::shared::nir::visit::{walk_op, walk_value, NirVisitor};
use crate::target::shared::nir::{NirOp, NirValue};

/// How often each name occurs in reading, writing, or re-binding positions
/// across a whole body — everything except the introducing `Bind` itself.
#[derive(Default)]
pub(crate) struct NameUses {
    counts: HashMap<String, u64>,
}

impl NameUses {
    /// Census `ops`. Every `Bind` occurrence counts too (a *second* `Bind` of
    /// the same name means the name is structurally shared and nothing about
    /// it is provably unused), so the caller subtracts the one occurrence of
    /// the specific `Bind` it is judging via [`NameUses::used_besides_bind`].
    pub(crate) fn census(ops: &[NirOp]) -> NameUses {
        let mut uses = NameUses::default();
        uses.visit_ops(ops);
        uses
    }

    /// Whether `name` occurs anywhere besides the single introducing `Bind`
    /// the caller holds (which contributed exactly one count).
    pub(crate) fn used_besides_bind(&self, name: &str) -> bool {
        self.counts.get(name).copied().unwrap_or(0) > 1
    }

    /// Raw occurrence count. LICM compares a whole-function census against a
    /// loop-body census: equal counts prove every occurrence of the name lives
    /// inside the loop, so hoisting its bind just outside is scope-safe.
    pub(crate) fn count(&self, name: &str) -> u64 {
        self.counts.get(name).copied().unwrap_or(0)
    }

    fn bump(&mut self, name: &str) {
        *self.counts.entry(name.to_string()).or_insert(0) += 1;
    }
}

impl NirVisitor for NameUses {
    fn visit_op(&mut self, op: &NirOp) {
        match op {
            NirOp::Bind { name, .. } => self.bump(name),
            NirOp::Assign { name, .. } => self.bump(name),
            NirOp::StateAssign { resource, .. } => self.bump(resource),
            NirOp::For { name, .. } => self.bump(name),
            NirOp::ForEach { name, .. } => self.bump(name),
            NirOp::Trap { name, .. } => self.bump(name),
            _ => {}
        }
        walk_op(self, op);
    }

    fn visit_value(&mut self, value: &NirValue) {
        match value {
            NirValue::Local(name) => self.bump(name),
            NirValue::LocalRef { name, .. } => self.bump(name),
            // A call through a function-typed local names it in the `target`
            // string, not as a `Local` — count it, or a bind read only by
            // calls looks unused. (Global function targets bump too, which
            // is harmless: those names are never judged by this census.)
            NirValue::Call { target, .. }
            | NirValue::CallResult { target, .. }
            | NirValue::RuntimeCall { target, .. } => self.bump(target),
            _ => {}
        }
        walk_value(self, value);
    }
}
