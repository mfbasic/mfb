use super::*;

#[derive(Clone)]

pub(crate) struct IrMatchCase {
    pub(crate) pattern: IrMatchPattern,
    pub(crate) guard: Option<IrValue>,
    pub(crate) body: Vec<IrOp>,
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
        // Source location of the call expression (origin for helper-generated errors).
        loc: IrSourceLoc,
    },
    CallResult {
        target: String,
        args: Vec<IrValue>,
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
    },
    Binary {
        op: String,
        left: Box<IrValue>,
        right: Box<IrValue>,
        // Source location of the operator (origin for arithmetic-generated errors).
        loc: IrSourceLoc,
    },
    Unary {
        op: String,
        operand: Box<IrValue>,
        // Source location of the operator (origin for arithmetic-generated errors).
        loc: IrSourceLoc,
    },
}
