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
    /// The *address* of a local binding's slot (a borrow of the slot itself, not
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
        index: usize,
        type_: String,
        /// When set, the env slot at `index` holds a pointer to the parent
        /// binding's slot (a non-escaping `MUT` borrow), so the capture binds a
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
