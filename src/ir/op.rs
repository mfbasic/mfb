use super::*;

#[derive(Clone)]
pub(crate) enum IrOp {
    Bind {
        mutable: bool,
        name: String,
        type_: String,
        value: Option<IrValue>,
        // Whether `type_` came from an explicit `AS T` annotation (vs inferred
        // from the initializer or synthesized by lowering). Only explicitly
        // annotated bindings are subject to `TYPE_BINDING_MISMATCH`: an inferred
        // binding's type *is* its initializer's type and cannot disagree
        // (plan-20-Z; mirrors syntaxcheck's `declared: Option<Type>`).
        explicit_type: bool,
        // Source location of the declaring statement.
        loc: IrSourceLoc,
    },
    Assign {
        name: String,
        value: IrValue,
        loc: IrSourceLoc,
    },
    AssignGlobal {
        name: String,
        value: IrValue,
        loc: IrSourceLoc,
    },
    /// Replace the `STATE` payload of a `RES` binding (`resource.state = value`).
    StateAssign {
        resource: String,
        value: IrValue,
        loc: IrSourceLoc,
    },
    Return {
        value: Option<IrValue>,
        loc: IrSourceLoc,
    },
    ExitLoop {
        kind: LoopKind,
        loc: IrSourceLoc,
    },
    ContinueLoop {
        kind: LoopKind,
        loc: IrSourceLoc,
    },
    ExitProgram {
        code: IrValue,
        loc: IrSourceLoc,
    },
    Fail {
        error: IrValue,
        loc: IrSourceLoc,
    },
    Eval {
        value: IrValue,
        loc: IrSourceLoc,
    },
    If {
        condition: IrValue,
        then_body: Vec<IrOp>,
        else_body: Vec<IrOp>,
        loc: IrSourceLoc,
    },
    Match {
        value: IrValue,
        cases: Vec<IrMatchCase>,
        loc: IrSourceLoc,
    },
    While {
        kind: LoopKind,
        condition: IrValue,
        body: Vec<IrOp>,
        loc: IrSourceLoc,
    },
    For {
        name: String,
        type_: String,
        start: IrValue,
        end: IrValue,
        step: IrValue,
        body: Vec<IrOp>,
        // Source location of the loop header; origin for increment overflow.
        loc: IrSourceLoc,
    },
    DoUntil {
        body: Vec<IrOp>,
        condition: IrValue,
        loc: IrSourceLoc,
    },
    ForEach {
        name: String,
        type_: String,
        iterable: IrValue,
        body: Vec<IrOp>,
        loc: IrSourceLoc,
    },
    Trap {
        name: String,
        body: Vec<IrOp>,
        loc: IrSourceLoc,
    },
}

impl IrOp {
    /// The source location of the op (the statement it was lowered from).
    pub(crate) fn loc(&self) -> IrSourceLoc {
        match self {
            IrOp::Bind { loc, .. }
            | IrOp::Assign { loc, .. }
            | IrOp::AssignGlobal { loc, .. }
            | IrOp::StateAssign { loc, .. }
            | IrOp::Return { loc, .. }
            | IrOp::ExitLoop { loc, .. }
            | IrOp::ContinueLoop { loc, .. }
            | IrOp::ExitProgram { loc, .. }
            | IrOp::Fail { loc, .. }
            | IrOp::Eval { loc, .. }
            | IrOp::If { loc, .. }
            | IrOp::Match { loc, .. }
            | IrOp::While { loc, .. }
            | IrOp::For { loc, .. }
            | IrOp::DoUntil { loc, .. }
            | IrOp::ForEach { loc, .. }
            | IrOp::Trap { loc, .. } => *loc,
        }
    }
}
