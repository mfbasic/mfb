//! The shared driver for Opt1's local-rewrite rows (algebraic simplification,
//! non-loop strength reduction): one scope-tracked, bottom-up walk over every
//! value in the module, offering each visited node to each enabled row.
//!
//! The rows need identical machinery — a lexical `Local` → declared-type
//! environment (their numeric rewrites gate on the §4.1 promoted type) and a
//! children-first traversal (so `(x*1)*2` fully simplifies) — so the walk lives
//! here once and the rows are per-node rewrite callbacks. Each row keeps its own
//! catalog level in [`apply`], and its fire count feeds `optimizer::stats` for
//! the `mfb build -v` per-row lines.
//!
//! The traversal is exhaustive (no `_` arm) over `NirOp`/`NirValue` for the same
//! anti-drift reason as `nir::visit` (bug-328); it cannot ride that seam because
//! this walk *mutates* and `NirVisitor` is read-only.

use std::collections::HashMap;

use crate::optimizer::level_enabled;
use crate::target::shared::nir::{NirMatchPattern, NirModule, NirOp, NirValue};
use crate::types::ParameterType;

use super::{algebraic, constant_folding, strength};
#[cfg(test)]
use crate::operators::{BinaryOp, UnaryOp};

/// Run every enabled Opt1 local-rewrite row over the whole module.
pub(crate) fn apply(module: &mut NirModule) {
    // One catalog level per row (`planning/optimizations.md`); a row is offered
    // nodes only when its own level is on the active dial.
    let mut rows = Rows {
        constant: level_enabled(1).then_some(0),
        algebraic: level_enabled(1).then_some(0),
        strength: level_enabled(1).then_some(0),
    };
    if rows.constant.is_none() && rows.algebraic.is_none() && rows.strength.is_none() {
        return;
    }
    let empty = Scopes::new();
    for global in &mut module.globals {
        if let Some(value) = &mut global.value {
            rows.rewrite_value(value, &empty);
        }
    }
    for function in &mut module.functions {
        let mut scopes = Scopes::new();
        for param in &mut function.params {
            if let Some(default) = &mut param.default {
                // A default is evaluated outside this body's scope: a name-free
                // literal lowered at each call site, or a call to its hidden
                // default function (plan-136-A).
                rows.rewrite_value(default, &Scopes::new());
            }
            scopes.insert(param.name.clone(), param.type_.clone());
        }
        rows.rewrite_ops(&mut function.body, &mut scopes);
    }
    if let Some(fired) = rows.constant {
        crate::optimizer::stats::count_constant_folds(fired);
    }
    if let Some(fired) = rows.algebraic {
        crate::optimizer::stats::count_algebraic_simplifications(fired);
    }
    if let Some(fired) = rows.strength {
        crate::optimizer::stats::count_strength_reductions(fired);
    }
}

/// The enabled rows and their fire counts (`None` = the dial disabled the row).
struct Rows {
    constant: Option<u64>,
    algebraic: Option<u64>,
    strength: Option<u64>,
}

impl Rows {
    /// Offer one node (children already rewritten) to each enabled row.
    /// Order matters and makes one application each a fixpoint: folding runs
    /// first so a collapsed constant feeds the identity/strength patterns
    /// (`(1+1) * x` → `2 * x` → `x + x`), algebraic's output is a bare
    /// already-visited operand, and strength's `x+x`/`x*x` output matches no
    /// row (a const-const `*`/`^` would already have folded before strength
    /// could see it).
    fn rewrite_node(&mut self, value: &mut NirValue, scopes: &Scopes) {
        if let Some(fired) = &mut self.constant {
            if constant_folding::rewrite_value(value, scopes) {
                *fired += 1;
            }
        }
        if let Some(fired) = &mut self.algebraic {
            if algebraic::rewrite_value(value, scopes) {
                *fired += 1;
            }
        }
        if let Some(fired) = &mut self.strength {
            if strength::rewrite_value(value, scopes) {
                *fired += 1;
            }
        }
    }

    fn rewrite_ops(&mut self, ops: &mut [NirOp], scopes: &mut Scopes) {
        for op in ops {
            match op {
                NirOp::Bind {
                    name, type_, value, ..
                } => {
                    if let Some(value) = value {
                        self.rewrite_value(value, scopes);
                    }
                    scopes.insert(name.clone(), type_.clone());
                }
                NirOp::StoreGlobal { value, .. } => {
                    if let Some(value) = value {
                        self.rewrite_value(value, scopes);
                    }
                }
                NirOp::Assign { value, .. } | NirOp::StateAssign { value, .. } => {
                    self.rewrite_value(value, scopes)
                }
                NirOp::Return { value } => {
                    if let Some(value) = value {
                        self.rewrite_value(value, scopes);
                    }
                }
                NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
                NirOp::ExitProgram { code } => self.rewrite_value(code, scopes),
                NirOp::Fail { error } => self.rewrite_value(error, scopes),
                NirOp::Eval { value } => self.rewrite_value(value, scopes),
                NirOp::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    self.rewrite_value(condition, scopes);
                    scopes.scoped(|scopes| self.rewrite_ops(then_body, scopes));
                    scopes.scoped(|scopes| self.rewrite_ops(else_body, scopes));
                }
                NirOp::Match { value, cases } => {
                    self.rewrite_value(value, scopes);
                    for case in cases {
                        match &mut case.pattern {
                            NirMatchPattern::Else => {}
                            NirMatchPattern::Value(value) => self.rewrite_value(value, scopes),
                            NirMatchPattern::OneOf(values) => {
                                for value in values {
                                    self.rewrite_value(value, scopes);
                                }
                            }
                        }
                        if let Some(guard) = &mut case.guard {
                            self.rewrite_value(guard, scopes);
                        }
                        scopes.scoped(|scopes| self.rewrite_ops(&mut case.body, scopes));
                    }
                }
                NirOp::While {
                    condition, body, ..
                } => {
                    self.rewrite_value(condition, scopes);
                    scopes.scoped(|scopes| self.rewrite_ops(body, scopes));
                }
                NirOp::For {
                    name,
                    type_,
                    start,
                    end,
                    step,
                    body,
                    ..
                } => {
                    self.rewrite_value(start, scopes);
                    self.rewrite_value(end, scopes);
                    self.rewrite_value(step, scopes);
                    scopes.scoped(|scopes| {
                        scopes.insert(name.clone(), type_.clone());
                        self.rewrite_ops(body, scopes);
                    });
                }
                NirOp::DoUntil { body, condition } => {
                    scopes.scoped(|scopes| self.rewrite_ops(body, scopes));
                    // The condition is lowered against the *outer* locals
                    // (`ir/lower/`, `HirStatement::DoUntil`), so body bindings
                    // are out of scope here — rewrite it after the body frame is
                    // gone.
                    self.rewrite_value(condition, scopes);
                }
                NirOp::ForEach {
                    name,
                    type_,
                    iterable,
                    body,
                    ..
                } => {
                    self.rewrite_value(iterable, scopes);
                    scopes.scoped(|scopes| {
                        scopes.insert(name.clone(), type_.clone());
                        self.rewrite_ops(body, scopes);
                    });
                }
                NirOp::Trap { name, body } => {
                    scopes.scoped(|scopes| {
                        scopes.insert(name.clone(), ParameterType::named("Error"));
                        self.rewrite_ops(body, scopes);
                    });
                }
            }
        }
    }

    fn rewrite_value(&mut self, value: &mut NirValue, scopes: &Scopes) {
        match value {
            NirValue::Const { .. }
            | NirValue::Local(_)
            | NirValue::LocalRef { .. }
            | NirValue::Global { .. }
            | NirValue::FunctionRef { .. }
            | NirValue::Capture { .. } => {}
            NirValue::Closure { captures, .. } => {
                for capture in captures {
                    self.rewrite_value(capture, scopes);
                }
            }
            NirValue::Call { args, .. }
            | NirValue::CallResult { args, .. }
            | NirValue::RuntimeCall { args, .. }
            | NirValue::Constructor { args, .. } => {
                for arg in args {
                    self.rewrite_value(arg, scopes);
                }
            }
            NirValue::UnionWrap { value, .. }
            | NirValue::UnionExtract { value, .. }
            | NirValue::ResultIsOk { value }
            | NirValue::ResultValue { value }
            | NirValue::ResultError { value }
            | NirValue::Checked { value, .. } => self.rewrite_value(value, scopes),
            NirValue::WithUpdate {
                target, updates, ..
            } => {
                self.rewrite_value(target, scopes);
                for update in updates {
                    self.rewrite_value(&mut update.value, scopes);
                }
            }
            NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => {
                for value in values {
                    self.rewrite_value(value, scopes);
                }
            }
            NirValue::MapLiteral { entries, .. } => {
                for (key, entry_value) in entries {
                    self.rewrite_value(key, scopes);
                    self.rewrite_value(entry_value, scopes);
                }
            }
            NirValue::MemberAccess { target, .. } => self.rewrite_value(target, scopes),
            NirValue::Binary { left, right, .. } => {
                self.rewrite_value(left, scopes);
                self.rewrite_value(right, scopes);
                self.rewrite_node(value, scopes);
            }
            NirValue::Unary { operand, .. } => {
                self.rewrite_value(operand, scopes);
                self.rewrite_node(value, scopes);
            }
        }
    }
}

/// Lexical `Local` → declared-type environment. Shadowing is real in NIR
/// (`ir/lower/` keeps source names), so nested bodies push a frame and lookup
/// walks innermost-out; a miss means "type unknown — do not rewrite".
pub(super) struct Scopes {
    stack: Vec<HashMap<String, ParameterType>>,
}

impl Scopes {
    pub(super) fn new() -> Self {
        Scopes {
            stack: vec![HashMap::new()],
        }
    }

    pub(super) fn insert(&mut self, name: String, type_: ParameterType) {
        self.stack
            .last_mut()
            .expect("scope stack is never empty")
            .insert(name, type_);
    }

    fn lookup(&self, name: &str) -> Option<&ParameterType> {
        self.stack.iter().rev().find_map(|frame| frame.get(name))
    }

    fn scoped(&mut self, body: impl FnOnce(&mut Scopes)) {
        self.stack.push(HashMap::new());
        body(self);
        self.stack.pop();
    }
}

/// Move a boxed operand out, leaving a placeholder that the enclosing
/// `*value = …` assignment immediately drops.
pub(super) fn take(operand: &mut NirValue) -> NirValue {
    std::mem::replace(operand, NirValue::Local(String::new()))
}

/// Whether the operand's statically-known type equals `expected`. Only shapes
/// that carry (or scope-resolve to) a declared type answer; anything else is
/// unknown and blocks the rewrite. These four shapes are also exactly the
/// pure, cheaply re-evaluable leaves, so strength reduction reuses this as its
/// duplicability gate.
pub(super) fn scopes_type_is(value: &NirValue, expected: &ParameterType, scopes: &Scopes) -> bool {
    let known = match value {
        NirValue::Const { type_, .. }
        | NirValue::Global { type_, .. }
        | NirValue::Capture { type_, .. } => Some(type_),
        NirValue::Local(name) => scopes.lookup(name),
        NirValue::LocalRef { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Closure { .. }
        | NirValue::Call { .. }
        | NirValue::CallResult { .. }
        | NirValue::RuntimeCall { .. }
        | NirValue::Constructor { .. }
        | NirValue::UnionWrap { .. }
        | NirValue::UnionExtract { .. }
        | NirValue::ResultIsOk { .. }
        | NirValue::ResultValue { .. }
        | NirValue::ResultError { .. }
        | NirValue::Checked { .. }
        | NirValue::WithUpdate { .. }
        | NirValue::ListLiteral { .. }
        | NirValue::SetLiteral { .. }
        | NirValue::MapLiteral { .. }
        | NirValue::MemberAccess { .. }
        | NirValue::Binary { .. }
        | NirValue::Unary { .. } => None,
    };
    known == Some(expected)
}

#[cfg(test)]
pub(super) mod testutil {
    use super::*;
    use crate::target::shared::nir::NirSourceLoc;

    pub(in crate::optimizer::opt1) fn int_const(value: &str) -> NirValue {
        typed_const(ParameterType::Integer, value)
    }

    pub(in crate::optimizer::opt1) fn typed_const(type_: ParameterType, value: &str) -> NirValue {
        NirValue::Const {
            type_,
            value: value.to_string(),
        }
    }

    pub(in crate::optimizer::opt1) fn local(name: &str) -> NirValue {
        NirValue::Local(name.to_string())
    }

    pub(in crate::optimizer::opt1) fn binary(
        op: BinaryOp,
        left: NirValue,
        right: NirValue,
    ) -> NirValue {
        NirValue::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
            loc: NirSourceLoc::default(),
        }
    }

    pub(in crate::optimizer::opt1) fn unary(op: UnaryOp, operand: NirValue) -> NirValue {
        NirValue::Unary {
            op,
            operand: Box::new(operand),
            loc: NirSourceLoc::default(),
        }
    }

    /// Render enough of a value to compare rewrites structurally.
    pub(in crate::optimizer::opt1) fn shape(value: &NirValue) -> String {
        match value {
            NirValue::Const { value, .. } => format!("const({value})"),
            NirValue::Local(name) => format!("local({name})"),
            NirValue::Binary {
                op, left, right, ..
            } => format!("({} {} {})", shape(left), op.name(), shape(right)),
            NirValue::Unary { op, operand, .. } => {
                format!("({} {})", op.name(), shape(operand))
            }
            _ => "other".to_string(),
        }
    }

    /// A one-function `NirModule` for pass-entry tests.
    pub(in crate::optimizer::opt1) fn test_module(
        functions: Vec<crate::target::shared::nir::NirFunction>,
    ) -> NirModule {
        NirModule {
            target: "macos-aarch64".to_string(),
            build_mode: crate::target::NativeBuildMode::Console,
            stdin_log_cap: 0,
            debug: crate::codegen::debug::DebugOptions::OFF,
            project: "test".to_string(),
            entry: None,
            globals: vec![],
            types: vec![],
            imports: vec![],
            runtime_helpers: vec![],
            functions,
            link_functions: vec![],
            link_cstructs: vec![],
            native_resources: vec![],
            native_libraries: crate::binary_repr::NativeLibraryTable { entries: vec![] },
            max_buffer_bytes: 0,
            canvas_auto_paint_ms: crate::manifest::DEFAULT_CANVAS_AUTO_PAINT_MS,
            arena_memory: crate::ir::ArenaMemoryLimits::default(),
        }
    }

    pub(in crate::optimizer::opt1) fn int_scope(name: &str) -> Scopes {
        let mut scopes = Scopes::new();
        scopes.insert(name.to_string(), ParameterType::Integer);
        scopes
    }
}

#[cfg(test)]
mod tests;
