//! Whether running a piece of NIR can reach a particular store — the one
//! call-graph walk behind three guards that must each fail closed:
//!
//! * `G25` (bug-487, `collection/assign/inplace_dest.rs`): an in-place
//!   `RES … STATE` update declines when an operand can reach a `STATE`
//!   assignment ([`StoreLeaf::StateAssign`]).
//! * bug-665 (`operand_snapshot.rs`): a call argument read out of a module-level
//!   global `g` is copied when the call can reach a `StoreGlobal` of `g`
//!   ([`StoreLeaf::Global`]) — that store frees the block the parameter borrows.
//! * bug-666 (`lower_for_each`): a `FOR EACH` over `g` walks its own copy when
//!   the body can reach a `StoreGlobal` of `g`.
//!
//! A false "cannot reach" is a use-after-free; a false "can reach" only costs a
//! copy (or the in-place fast path). So every call whose body this walk cannot
//! see is assumed to reach:
//!
//! * a module function is followed into its body (memoized DFS);
//! * a builtin runs user code only through a callback: a `FunctionRef` or
//!   `LAMBDA` argument is followed into its lifted body, and any other argument
//!   in a position the registry types as `FUNC` is opaque;
//! * every other target — an indirect call through a `FUNC` binding, a foreign
//!   import, a name nothing here resolves — is opaque.

use crate::codegen::engine::builder::*;
use crate::target::shared::nir::visit::{walk_op, walk_value, NirVisitor};
use crate::target::shared::nir::*;
use crate::types::ParameterType;
use std::collections::HashSet;

/// The store a walk is looking for.
#[derive(Clone, Copy)]
pub(crate) enum StoreLeaf<'n> {
    /// Any `RES … STATE` assignment (`NirOp::StateAssign`).
    StateAssign,
    /// A reassignment of the named module-level global (`NirOp::StoreGlobal`).
    Global(&'n str),
}

impl StoreLeaf<'_> {
    fn matches(self, op: &NirOp) -> bool {
        match (self, op) {
            (StoreLeaf::StateAssign, NirOp::StateAssign { .. }) => true,
            (StoreLeaf::Global(global), NirOp::StoreGlobal { name, .. }) => name == global,
            _ => false,
        }
    }
}

/// The module-level global whose block `value` lowers to a pointer into: the
/// global itself, or a field / variant payload / `Result` payload read out of
/// it. `None` for anything that is not rooted in a global.
pub(crate) fn global_root(value: &NirValue) -> Option<&str> {
    match value {
        NirValue::Global { name, .. } => Some(name),
        NirValue::MemberAccess { target, .. }
        | NirValue::UnionExtract { value: target, .. }
        | NirValue::ResultValue { value: target }
        | NirValue::ResultError { value: target } => global_root(target),
        _ => None,
    }
}

impl CodeBuilder<'_> {
    /// Whether evaluating any of `values` can reach `leaf`. A value is an
    /// expression, so it can hold no store op of its own — only its calls can.
    pub(crate) fn values_reach_store(&self, values: &[NirValue], leaf: StoreLeaf<'_>) -> bool {
        let mut walk = Walk {
            builder: self,
            leaf,
            visited: HashSet::new(),
        };
        values.iter().any(|value| walk.value_reaches(value))
    }

    /// Whether running `ops` can reach `leaf`, directly or through a call.
    pub(crate) fn ops_reach_store(&self, ops: &[NirOp], leaf: StoreLeaf<'_>) -> bool {
        let mut walk = Walk {
            builder: self,
            leaf,
            visited: HashSet::new(),
        };
        walk.ops_reach(ops)
    }

    /// Whether the call `target(args)` itself — not its argument expressions —
    /// can reach `leaf`.
    pub(crate) fn call_reaches_store(
        &self,
        target: &str,
        args: &[NirValue],
        leaf: StoreLeaf<'_>,
    ) -> bool {
        let mut walk = Walk {
            builder: self,
            leaf,
            visited: HashSet::new(),
        };
        walk.call_reaches(target, args)
    }
}

struct Walk<'b, 'a, 'n> {
    builder: &'b CodeBuilder<'a>,
    leaf: StoreLeaf<'n>,
    /// Module functions already on the DFS stack or already explored. The search
    /// is a monotone OR that stops at the first hit, so a function seen once adds
    /// nothing the second time: a sound memo, and what terminates recursion.
    visited: HashSet<String>,
}

impl Walk<'_, '_, '_> {
    fn ops_reach(&mut self, ops: &[NirOp]) -> bool {
        let mut finder = Finder {
            walk: self,
            found: false,
        };
        finder.visit_ops(ops);
        finder.found
    }

    fn value_reaches(&mut self, value: &NirValue) -> bool {
        let mut finder = Finder {
            walk: self,
            found: false,
        };
        finder.visit_value(value);
        finder.found
    }

    fn call_reaches(&mut self, target: &str, args: &[NirValue]) -> bool {
        if self.builder.functions.contains_key(target) {
            return self.function_reaches(target);
        }
        if self.builder.function_symbols.contains_key(target)
            || !crate::codegen::builtins::is_builtin_call(target)
        {
            // A foreign import, an indirect call through a `FUNC` binding, or a
            // name nothing here resolves: its body is invisible.
            return true;
        }
        let callback_positions = builtin_callback_positions(target);
        args.iter().enumerate().any(|(index, arg)| match arg {
            NirValue::FunctionRef { name, .. } | NirValue::Closure { name, .. } => {
                self.callback_reaches(name)
            }
            _ => callback_positions.as_ref().is_none_or(|positions| positions.contains(&index))
                && !self.is_plainly_not_a_function(arg),
        })
    }

    /// An argument that cannot evaluate to a function value, by its shape alone:
    /// a constant, an operator, or a node whose own type is not a function.
    /// Anything else in a callback position is assumed to be a callback.
    fn is_plainly_not_a_function(&self, value: &NirValue) -> bool {
        let type_ = match value {
            NirValue::Const { type_, .. }
            | NirValue::LocalRef { type_, .. }
            | NirValue::Capture { type_, .. } => type_,
            // A global's node may carry an unset type; the declaration has it.
            NirValue::Global { name, .. } => match self.builder.globals.get(name) {
                Some(global) => &global.type_,
                None => return false,
            },
            NirValue::Binary { .. } | NirValue::Unary { .. } => return true,
            _ => return false,
        };
        !matches!(type_, ParameterType::Func(..))
    }

    /// A callback handed to a builtin: a lifted `LAMBDA` or a named function is a
    /// module function; a builtin used as a value (`toString`) runs no user code.
    fn callback_reaches(&mut self, name: &str) -> bool {
        if self.builder.functions.contains_key(name) {
            return self.function_reaches(name);
        }
        !crate::codegen::builtins::is_builtin_call(name)
    }

    fn function_reaches(&mut self, name: &str) -> bool {
        if !self.visited.insert(name.to_string()) {
            return false;
        }
        let function = self.builder.functions[name];
        self.ops_reach(&function.body)
    }
}

/// Stops at the first store matching the leaf, or the first call that reaches one.
struct Finder<'w, 'b, 'a, 'n> {
    walk: &'w mut Walk<'b, 'a, 'n>,
    found: bool,
}

impl NirVisitor for Finder<'_, '_, '_, '_> {
    fn visit_op(&mut self, op: &NirOp) {
        if self.found {
            return;
        }
        if self.walk.leaf.matches(op) {
            self.found = true;
            return;
        }
        walk_op(self, op);
    }

    fn visit_value(&mut self, value: &NirValue) {
        if self.found {
            return;
        }
        let reaches = match value {
            NirValue::Call { target, args, .. }
            | NirValue::CallResult { target, args, .. }
            | NirValue::RuntimeCall { target, args, .. } => self.walk.call_reaches(target, args),
            _ => false,
        };
        if reaches {
            self.found = true;
            return;
        }
        walk_value(self, value);
    }
}

/// The argument positions any overload of builtin `target` types as a function.
/// `None` when the registry does not describe `target` — every position is then
/// treated as a possible callback. The unqualified general builtins (`len`,
/// `toString`, …) take no function.
fn builtin_callback_positions(target: &str) -> Option<Vec<usize>> {
    if crate::codegen::builtins::general::is_general_call(target) {
        return Some(Vec::new());
    }
    let resolved = crate::codegen::registry::registry().resolve_func(target)?;
    let mut positions = Vec::new();
    for implementation in resolved.function.implementations() {
        for (index, param) in implementation.params.iter().enumerate() {
            if matches!(param.ty, ParameterType::Func(..)) && !positions.contains(&index) {
                positions.push(index);
            }
        }
    }
    Some(positions)
}
