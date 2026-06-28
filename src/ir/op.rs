use super::*;

#[derive(Clone)]

pub(crate) enum IrOp {
    Bind {
        mutable: bool,
        name: String,
        type_: String,
        value: Option<IrValue>,
    },
    Assign {
        name: String,
        value: IrValue,
    },
    AssignGlobal {
        name: String,
        value: IrValue,
    },
    /// Replace the `STATE` payload of a `RES` binding (`resource.state = value`).
    StateAssign {
        resource: String,
        value: IrValue,
    },
    Return {
        value: Option<IrValue>,
    },
    ExitLoop {
        kind: LoopKind,
    },
    ContinueLoop {
        kind: LoopKind,
    },
    ExitProgram {
        code: IrValue,
    },
    Fail {
        error: IrValue,
    },
    Eval {
        value: IrValue,
    },
    If {
        condition: IrValue,
        then_body: Vec<IrOp>,
        else_body: Vec<IrOp>,
    },
    Match {
        value: IrValue,
        cases: Vec<IrMatchCase>,
    },
    While {
        kind: LoopKind,
        condition: IrValue,
        body: Vec<IrOp>,
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
    },
    ForEach {
        name: String,
        type_: String,
        iterable: IrValue,
        body: Vec<IrOp>,
    },
    Trap {
        name: String,
        body: Vec<IrOp>,
    },
}
