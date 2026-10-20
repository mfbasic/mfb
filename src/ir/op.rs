use super::*;
use crate::types::ParameterType;

#[derive(Clone)]
pub(crate) enum IrOp {
    Bind {
        mutable: bool,
        name: String,
        type_: ParameterType,
        value: Option<IrValue>,
        // Whether `type_` came from an explicit `AS T` annotation (vs inferred
        // from the initializer or synthesized by lowering). Only explicitly
        // annotated bindings are subject to `TYPE_BINDING_MISMATCH`: an inferred
        // binding's type *is* its initializer's type and cannot disagree
        // (plan-20-Z; mirrors the former source checker's `declared: Option<Type>`).
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
        target: Option<String>,
    },
    ContinueLoop {
        kind: LoopKind,
        loc: IrSourceLoc,
        target: Option<String>,
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
        type_: ParameterType,
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
        type_: ParameterType,
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

impl IrOp {
    /// Call `f` on every value this op holds, and on every value of every op
    /// nested in it, pre-order through each value tree
    /// ([`crate::ir::value::visit_value_mut`]). Exhaustive by construction: a new
    /// `IrOp` variant does not compile until it says where its values are.
    pub(crate) fn visit_values_mut(&mut self, f: &mut impl FnMut(&mut IrValue)) {
        use crate::ir::value::visit_value_mut;
        match self {
            IrOp::Bind { value, .. } | IrOp::Return { value, .. } => {
                if let Some(value) = value {
                    visit_value_mut(value, f);
                }
            }
            IrOp::Assign { value, .. }
            | IrOp::AssignGlobal { value, .. }
            | IrOp::StateAssign { value, .. }
            | IrOp::Eval { value, .. }
            | IrOp::Fail { error: value, .. }
            | IrOp::ExitProgram { code: value, .. } => visit_value_mut(value, f),
            IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
            IrOp::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                visit_value_mut(condition, f);
                for op in then_body.iter_mut().chain(else_body.iter_mut()) {
                    op.visit_values_mut(f);
                }
            }
            IrOp::Match { value, cases, .. } => {
                visit_value_mut(value, f);
                for case in cases {
                    match &mut case.pattern {
                        IrMatchPattern::Else => {}
                        IrMatchPattern::Value(value) => visit_value_mut(value, f),
                        IrMatchPattern::OneOf(values) => {
                            for value in values {
                                visit_value_mut(value, f);
                            }
                        }
                    }
                    if let Some(guard) = &mut case.guard {
                        visit_value_mut(guard, f);
                    }
                    for op in &mut case.body {
                        op.visit_values_mut(f);
                    }
                }
            }
            IrOp::While {
                condition, body, ..
            }
            | IrOp::DoUntil {
                body, condition, ..
            } => {
                visit_value_mut(condition, f);
                for op in body {
                    op.visit_values_mut(f);
                }
            }
            IrOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                visit_value_mut(start, f);
                visit_value_mut(end, f);
                visit_value_mut(step, f);
                for op in body {
                    op.visit_values_mut(f);
                }
            }
            IrOp::ForEach { iterable, body, .. } => {
                visit_value_mut(iterable, f);
                for op in body {
                    op.visit_values_mut(f);
                }
            }
            IrOp::Trap { body, .. } => {
                for op in body {
                    op.visit_values_mut(f);
                }
            }
        }
    }
}
