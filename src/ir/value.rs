use super::*;

#[derive(Clone)]
pub(crate) struct IrMatchCase {
    pub(crate) pattern: IrMatchPattern,
    pub(crate) guard: Option<IrValue>,
    pub(crate) body: Vec<IrOp>,
    // Source location of the case arm.
    pub(crate) loc: IrSourceLoc,
}
#[derive(Clone)]
pub(crate) enum IrMatchPattern {
    Else,
    Value(IrValue),
    OneOf(Vec<IrValue>),
}

#[derive(Clone)]
pub(crate) enum IrValue {
    Const {
        type_: String,
        value: String,
    },
    Local(String),
    Global(String),
    /// The *address* of a local binding's slot (a reference to the slot itself, not
    /// a read of its value). Used to capture a `MUT` binding into a non-escaping
    /// callback's environment so the callback observes and updates the live
    /// binding through the slot.
    LocalRef {
        name: String,
        type_: String,
    },
    FunctionRef {
        name: String,
        type_: String,
    },
    Closure {
        name: String,
        type_: String,
        captures: Vec<IrValue>,
    },
    Capture {
        /// The closure environment slot this capture reads. `u32` is the width
        /// the package format encodes, so the in-memory value cannot silently
        /// disagree with its serialization.
        index: u32,
        type_: String,
        /// When set, the env slot at `index` holds a pointer to the parent
        /// binding's slot (a non-escaping `MUT` by-ref capture), so the capture binds a
        /// *reference* local: reads and writes deref through the slot pointer.
        /// Otherwise it is an ordinary by-value capture.
        by_ref: bool,
    },
    Call {
        target: String,
        args: Vec<IrValue>,
        // Result type of the call (the callee's return type; plan-20-B).
        type_: String,
        // Source location of the call expression (origin for helper-generated errors).
        loc: IrSourceLoc,
    },
    CallResult {
        target: String,
        args: Vec<IrValue>,
        // Success type of the fallible call (the `T` of `Result OF T`; plan-20-B).
        type_: String,
        // Source location of the call expression (origin for helper-generated errors).
        loc: IrSourceLoc,
    },
    Constructor {
        type_: String,
        args: Vec<IrValue>,
    },
    UnionWrap {
        union_type: String,
        member_type: String,
        value: Box<IrValue>,
    },
    UnionExtract {
        type_: String,
        value: Box<IrValue>,
    },
    ResultIsOk {
        value: Box<IrValue>,
    },
    ResultValue {
        // Success type extracted from the `Result` (plan-20-B).
        type_: String,
        value: Box<IrValue>,
    },
    ResultError {
        value: Box<IrValue>,
    },
    WithUpdate {
        type_: String,
        target: Box<IrValue>,
        updates: Vec<IrRecordUpdate>,
    },
    ListLiteral {
        type_: String,
        values: Vec<IrValue>,
    },
    MapLiteral {
        type_: String,
        entries: Vec<(IrValue, IrValue)>,
    },
    MemberAccess {
        target: Box<IrValue>,
        member: String,
        // Type of the accessed field/member (plan-20-B).
        type_: String,
    },
    Binary {
        op: String,
        left: Box<IrValue>,
        right: Box<IrValue>,
        // Result type of the operation (plan-20-B).
        type_: String,
        // Source location of the operator (origin for arithmetic-generated errors).
        loc: IrSourceLoc,
    },
    Unary {
        op: String,
        operand: Box<IrValue>,
        // Result type of the operation (plan-20-B).
        type_: String,
        // Source location of the operator (origin for arithmetic-generated errors).
        loc: IrSourceLoc,
    },
}

impl IrValue {
    /// The node's result type, when it is annotated on the node itself.
    /// `ResultIsOk` is always `Boolean` and `ResultError` always `Error`;
    /// `Local`/`Global` resolve through the enclosing binding environment
    /// (master plan §4.1) and yield `None` here.
    pub(crate) fn annotated_type(&self) -> Option<&str> {
        match self {
            IrValue::Const { type_, .. }
            | IrValue::LocalRef { type_, .. }
            | IrValue::FunctionRef { type_, .. }
            | IrValue::Closure { type_, .. }
            | IrValue::Capture { type_, .. }
            | IrValue::Call { type_, .. }
            | IrValue::CallResult { type_, .. }
            | IrValue::Constructor { type_, .. }
            | IrValue::UnionExtract { type_, .. }
            | IrValue::ResultValue { type_, .. }
            | IrValue::WithUpdate { type_, .. }
            | IrValue::ListLiteral { type_, .. }
            | IrValue::MapLiteral { type_, .. }
            | IrValue::MemberAccess { type_, .. }
            | IrValue::Binary { type_, .. }
            | IrValue::Unary { type_, .. } => Some(type_),
            IrValue::UnionWrap { union_type, .. } => Some(union_type),
            IrValue::ResultIsOk { .. } => Some("Boolean"),
            IrValue::ResultError { .. } => Some("Error"),
            IrValue::Local(_) | IrValue::Global(_) => None,
        }
    }
}

/// Maximum expression-nesting depth [`visit_value`] descends before it stops.
///
/// This mirrors the IR verifier's `MAX_DEPTH` (`ir/verify/mod.rs`): the read-only
/// value walkers there were bounded so a pathologically deep value expression
/// cannot overflow the stack, and the shared seam must keep that cutoff rather
/// than silently unbound it (bug-328). Kept numerically equal to that constant.
pub(crate) const VALUE_VISIT_MAX_DEPTH: usize = 256;

/// The single read-only traversal over an [`IrValue`] tree (bug-328).
///
/// Calls `f` on `value` and then, pre-order, on every descendant value, bounded
/// to [`VALUE_VISIT_MAX_DEPTH`] levels — past that a subtree is left unvisited,
/// exactly as the hand-written `collect_*_depth` walkers in `ir/verify` did. An
/// analysis matches inside `f` on the variants it cares about and inherits the
/// complete, depth-safe recursion for free.
pub(crate) fn visit_value(value: &IrValue, f: &mut impl FnMut(&IrValue)) {
    visit_value_depth(value, 0, f);
}

fn visit_value_depth<F: FnMut(&IrValue)>(value: &IrValue, depth: usize, f: &mut F) {
    if depth > VALUE_VISIT_MAX_DEPTH {
        return;
    }
    f(value);
    let next = depth + 1;
    match value {
        IrValue::Const { .. }
        | IrValue::Local(_)
        | IrValue::Global(_)
        | IrValue::LocalRef { .. }
        | IrValue::FunctionRef { .. }
        | IrValue::Capture { .. } => {}
        IrValue::Closure { captures, .. } => {
            for capture in captures {
                visit_value_depth(capture, next, f);
            }
        }
        IrValue::Call { args, .. }
        | IrValue::CallResult { args, .. }
        | IrValue::Constructor { args, .. } => {
            for arg in args {
                visit_value_depth(arg, next, f);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value }
        | IrValue::Unary { operand: value, .. }
        | IrValue::MemberAccess { target: value, .. } => visit_value_depth(value, next, f),
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            visit_value_depth(target, next, f);
            for update in updates {
                visit_value_depth(&update.value, next, f);
            }
        }
        IrValue::ListLiteral { values, .. } => {
            for value in values {
                visit_value_depth(value, next, f);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                visit_value_depth(key, next, f);
                visit_value_depth(value, next, f);
            }
        }
        IrValue::Binary { left, right, .. } => {
            visit_value_depth(left, next, f);
            visit_value_depth(right, next, f);
        }
    }
}

/// The single in-place mutating traversal over an [`IrValue`] tree (bug-328).
///
/// Calls `f` on `value` and then, pre-order, on every descendant. Unlike
/// [`visit_value`] this carries **no** depth cap: its sole caller,
/// `ir/package.rs`'s `rewrite_value_targets`, never bounded its recursion, so
/// bounding it here would change behavior on a deep tree. The traversal is
/// otherwise identical to the read-only seam.
pub(crate) fn visit_value_mut(value: &mut IrValue, f: &mut impl FnMut(&mut IrValue)) {
    visit_value_mut_inner(value, f);
}

fn visit_value_mut_inner<F: FnMut(&mut IrValue)>(value: &mut IrValue, f: &mut F) {
    f(value);
    match value {
        IrValue::Const { .. }
        | IrValue::Local(_)
        | IrValue::Global(_)
        | IrValue::LocalRef { .. }
        | IrValue::FunctionRef { .. }
        | IrValue::Capture { .. } => {}
        IrValue::Closure { captures, .. } => {
            for capture in captures {
                visit_value_mut_inner(capture, f);
            }
        }
        IrValue::Call { args, .. }
        | IrValue::CallResult { args, .. }
        | IrValue::Constructor { args, .. } => {
            for arg in args {
                visit_value_mut_inner(arg, f);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value }
        | IrValue::Unary { operand: value, .. }
        | IrValue::MemberAccess { target: value, .. } => visit_value_mut_inner(value, f),
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            visit_value_mut_inner(target, f);
            for update in updates {
                visit_value_mut_inner(&mut update.value, f);
            }
        }
        IrValue::ListLiteral { values, .. } => {
            for value in values {
                visit_value_mut_inner(value, f);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                visit_value_mut_inner(key, f);
                visit_value_mut_inner(value, f);
            }
        }
        IrValue::Binary { left, right, .. } => {
            visit_value_mut_inner(left, f);
            visit_value_mut_inner(right, f);
        }
    }
}

#[cfg(test)]
mod visit_tests {
    use super::*;

    fn loc() -> IrSourceLoc {
        IrSourceLoc::default()
    }

    fn unary(operand: IrValue) -> IrValue {
        IrValue::Unary {
            op: "-".to_string(),
            operand: Box::new(operand),
            type_: "Integer".to_string(),
            loc: loc(),
        }
    }

    #[test]
    fn visit_value_reaches_nested_children_pre_order() {
        // Binary(left=Local"a", right=Unary(Local"b")) — every node visited.
        let value = IrValue::Binary {
            op: "+".to_string(),
            left: Box::new(IrValue::Local("a".to_string())),
            right: Box::new(unary(IrValue::Local("b".to_string()))),
            type_: "Integer".to_string(),
            loc: loc(),
        };
        let mut locals = Vec::new();
        visit_value(&value, &mut |v| {
            if let IrValue::Local(name) = v {
                locals.push(name.clone());
            }
        });
        assert_eq!(locals, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn visit_value_stops_past_max_depth() {
        // A Unary chain deeper than the cap, with a sentinel Local at the very
        // bottom that must NOT be reached.
        let mut value = IrValue::Local("sentinel".to_string());
        for _ in 0..(VALUE_VISIT_MAX_DEPTH + 5) {
            value = unary(value);
        }
        let mut saw_sentinel = false;
        let mut visited = 0usize;
        visit_value(&value, &mut |v| {
            visited += 1;
            if matches!(v, IrValue::Local(name) if name == "sentinel") {
                saw_sentinel = true;
            }
        });
        assert!(
            !saw_sentinel,
            "visit_value descended past VALUE_VISIT_MAX_DEPTH to the sentinel"
        );
        // Nodes at depth 0..=MAX_DEPTH are visited: that is MAX_DEPTH + 1 nodes.
        assert_eq!(visited, VALUE_VISIT_MAX_DEPTH + 1);
    }

    #[test]
    fn visit_value_mut_rewrites_every_node_uncapped() {
        // The mutable seam has no depth cap: a chain deeper than the read cap is
        // still fully rewritten (matches rewrite_value_targets).
        let mut value = IrValue::Global("g".to_string());
        for _ in 0..(VALUE_VISIT_MAX_DEPTH + 5) {
            value = unary(value);
        }
        visit_value_mut(&mut value, &mut |v| {
            if let IrValue::Global(name) = v {
                *name = format!("pkg.{name}");
            }
        });
        // Peel back to the bottom and confirm the deep Global was rewritten.
        let mut cursor = &value;
        while let IrValue::Unary { operand, .. } = cursor {
            cursor = operand;
        }
        assert!(matches!(cursor, IrValue::Global(name) if name == "pkg.g"));
    }
}
