//! The single NIR traversal seam (bug-328).
//!
//! Every analysis over `NirOp`/`NirValue` used to hand-write its own recursive
//! `match`, so a fix applied to one walk (e.g. bug-118's `WHEN … WHERE` guard
//! traversal) had to be replicated by hand to every sibling — and drifted when
//! it was not. [`NirVisitor`] replaces that: the `walk_*` free functions own the
//! one authoritative recursion into every child node, and an implementer
//! overrides only the arms it cares about, inheriting complete traversal for
//! everything else.
//!
//! The `walk_*` functions are exhaustive (no `_ =>` arm), so adding a `NirOp` or
//! `NirValue` variant is a compile error *here* — in one place — rather than a
//! silent gap in a dozen scattered walkers.
//!
//! Scope-sensitive analyses (those that clone a constants map per branch, clear
//! it in loop bodies, etc.) keep that state on themselves and override
//! [`NirVisitor::visit_op`] for the arms where the scoping matters, delegating
//! the remaining arms to [`walk_op`]. The trait deliberately carries no such
//! map: encoding one analysis's scoping policy into the shared seam would leak
//! it into every other analysis.

use super::{NirMatchPattern, NirOp, NirValue};

/// A recursive traversal over the NIR tree with complete default recursion.
///
/// The default method bodies descend into every child node via the `walk_*`
/// free functions. Override a method to do work at that node; call the matching
/// `walk_*` from the override to continue the default recursion into children.
pub(crate) trait NirVisitor {
    fn visit_ops(&mut self, ops: &[NirOp]) {
        walk_ops(self, ops)
    }

    fn visit_op(&mut self, op: &NirOp) {
        walk_op(self, op)
    }

    fn visit_value(&mut self, value: &NirValue) {
        walk_value(self, value)
    }
}

pub(crate) fn walk_ops<V: NirVisitor + ?Sized>(visitor: &mut V, ops: &[NirOp]) {
    for op in ops {
        visitor.visit_op(op);
    }
}

pub(crate) fn walk_op<V: NirVisitor + ?Sized>(visitor: &mut V, op: &NirOp) {
    match op {
        NirOp::Bind { value, .. } | NirOp::StoreGlobal { value, .. } => {
            if let Some(value) = value {
                visitor.visit_value(value);
            }
        }
        NirOp::Assign { value, .. } => visitor.visit_value(value),
        NirOp::StateAssign { value, .. } => visitor.visit_value(value),
        NirOp::Return { value } => {
            if let Some(value) = value {
                visitor.visit_value(value);
            }
        }
        NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
        NirOp::ExitProgram { code } => visitor.visit_value(code),
        NirOp::Fail { error } => visitor.visit_value(error),
        NirOp::Eval { value } => visitor.visit_value(value),
        NirOp::If {
            condition,
            then_body,
            else_body,
        } => {
            visitor.visit_value(condition);
            visitor.visit_ops(then_body);
            visitor.visit_ops(else_body);
        }
        NirOp::Match { value, cases } => {
            visitor.visit_value(value);
            for case in cases {
                // The scrutinee, the pattern's own values, the `WHEN … WHERE`
                // guard, and the body are all executable and all part of the
                // one complete traversal. Walking the guard is the divergence
                // bug-118 fixed on some siblings and left broken on others
                // (bug-328); centralising it here makes it uniform.
                walk_match_pattern(visitor, &case.pattern);
                if let Some(guard) = &case.guard {
                    visitor.visit_value(guard);
                }
                visitor.visit_ops(&case.body);
            }
        }
        NirOp::While {
            condition, body, ..
        } => {
            visitor.visit_value(condition);
            visitor.visit_ops(body);
        }
        NirOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            visitor.visit_value(start);
            visitor.visit_value(end);
            visitor.visit_value(step);
            visitor.visit_ops(body);
        }
        NirOp::DoUntil { body, condition } => {
            visitor.visit_ops(body);
            visitor.visit_value(condition);
        }
        NirOp::ForEach { iterable, body, .. } => {
            visitor.visit_value(iterable);
            visitor.visit_ops(body);
        }
        NirOp::Trap { body, .. } => visitor.visit_ops(body),
    }
}

fn walk_match_pattern<V: NirVisitor + ?Sized>(visitor: &mut V, pattern: &NirMatchPattern) {
    match pattern {
        NirMatchPattern::Else => {}
        NirMatchPattern::Value(value) => visitor.visit_value(value),
        NirMatchPattern::OneOf(values) => {
            for value in values {
                visitor.visit_value(value);
            }
        }
    }
}

pub(crate) fn walk_value<V: NirVisitor + ?Sized>(visitor: &mut V, value: &NirValue) {
    match value {
        NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Capture { .. } => {}
        NirValue::Closure { captures, .. } => {
            for capture in captures {
                visitor.visit_value(capture);
            }
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                visitor.visit_value(arg);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => visitor.visit_value(value),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            visitor.visit_value(target);
            for update in updates {
                visitor.visit_value(&update.value);
            }
        }
        NirValue::ListLiteral { values, .. } => {
            for value in values {
                visitor.visit_value(value);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                visitor.visit_value(key);
                visitor.visit_value(value);
            }
        }
        NirValue::MemberAccess { target, .. } => visitor.visit_value(target),
        NirValue::Binary { left, right, .. } => {
            visitor.visit_value(left);
            visitor.visit_value(right);
        }
        NirValue::Unary { operand, .. } => visitor.visit_value(operand),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::nir::{NirMatchCase, NirRecordUpdate, NirSourceLoc};
    use crate::target::shared::runtime::RuntimeHelper;

    /// Records the discriminant tag of every `NirValue` the walk reaches, so a
    /// test can assert the default recursion visits every variant — including
    /// the ones reachable only through a `NirMatchCase::guard`.
    #[derive(Default)]
    struct TagCollector {
        value_tags: Vec<&'static str>,
    }

    impl NirVisitor for TagCollector {
        fn visit_value(&mut self, value: &NirValue) {
            self.value_tags.push(value_tag(value));
            walk_value(self, value);
        }
    }

    fn value_tag(value: &NirValue) -> &'static str {
        match value {
            NirValue::Const { .. } => "Const",
            NirValue::Local(_) => "Local",
            NirValue::LocalRef { .. } => "LocalRef",
            NirValue::Global { .. } => "Global",
            NirValue::FunctionRef { .. } => "FunctionRef",
            NirValue::Closure { .. } => "Closure",
            NirValue::Capture { .. } => "Capture",
            NirValue::Call { .. } => "Call",
            NirValue::CallResult { .. } => "CallResult",
            NirValue::RuntimeCall { .. } => "RuntimeCall",
            NirValue::Constructor { .. } => "Constructor",
            NirValue::UnionWrap { .. } => "UnionWrap",
            NirValue::UnionExtract { .. } => "UnionExtract",
            NirValue::ResultIsOk { .. } => "ResultIsOk",
            NirValue::ResultValue { .. } => "ResultValue",
            NirValue::ResultError { .. } => "ResultError",
            NirValue::WithUpdate { .. } => "WithUpdate",
            NirValue::ListLiteral { .. } => "ListLiteral",
            NirValue::MapLiteral { .. } => "MapLiteral",
            NirValue::MemberAccess { .. } => "MemberAccess",
            NirValue::Binary { .. } => "Binary",
            NirValue::Unary { .. } => "Unary",
        }
    }

    fn local(name: &str) -> NirValue {
        NirValue::Local(name.to_string())
    }

    fn boxed(value: NirValue) -> Box<NirValue> {
        Box::new(value)
    }

    /// Every `NirValue` variant, each tagged with a distinct `Local` so the
    /// walk's reach can be verified positionally. Composite variants carry a
    /// single child; leaf variants stand alone.
    fn one_of_every_value_variant() -> Vec<NirValue> {
        vec![
            NirValue::Const {
                type_: "Integer".to_string(),
                value: "1".to_string(),
            },
            local("plain"),
            NirValue::LocalRef {
                name: "r".to_string(),
                type_: "Integer".to_string(),
            },
            NirValue::Global {
                name: "g".to_string(),
                type_: "Integer".to_string(),
            },
            NirValue::FunctionRef {
                name: "f".to_string(),
                type_: "Function".to_string(),
            },
            NirValue::Closure {
                name: "c".to_string(),
                type_: "Function".to_string(),
                captures: vec![local("closure_child")],
            },
            NirValue::Capture {
                index: 0,
                type_: "Integer".to_string(),
                by_ref: false,
            },
            NirValue::Call {
                target: "call".to_string(),
                args: vec![local("call_child")],
                loc: NirSourceLoc::default(),
            },
            NirValue::CallResult {
                target: "callres".to_string(),
                args: vec![local("callresult_child")],
                loc: NirSourceLoc::default(),
            },
            NirValue::RuntimeCall {
                helper: RuntimeHelper::Thread,
                target: "rt".to_string(),
                args: vec![local("runtimecall_child")],
                loc: NirSourceLoc::default(),
            },
            NirValue::Constructor {
                type_: "T".to_string(),
                args: vec![local("constructor_child")],
            },
            NirValue::UnionWrap {
                union_type: "U".to_string(),
                member_type: "M".to_string(),
                value: boxed(local("unionwrap_child")),
            },
            NirValue::UnionExtract {
                type_: "M".to_string(),
                value: boxed(local("unionextract_child")),
            },
            NirValue::ResultIsOk {
                value: boxed(local("resultisok_child")),
            },
            NirValue::ResultValue {
                value: boxed(local("resultvalue_child")),
            },
            NirValue::ResultError {
                value: boxed(local("resulterror_child")),
            },
            NirValue::WithUpdate {
                type_: "T".to_string(),
                target: boxed(local("withupdate_target")),
                updates: vec![NirRecordUpdate {
                    field: "x".to_string(),
                    value: local("withupdate_child"),
                }],
            },
            NirValue::ListLiteral {
                type_: "List OF Integer".to_string(),
                values: vec![local("list_child")],
            },
            NirValue::MapLiteral {
                type_: "Map OF Integer TO Integer".to_string(),
                entries: vec![(local("map_key"), local("map_value"))],
            },
            NirValue::MemberAccess {
                target: boxed(local("member_child")),
                member: "m".to_string(),
            },
            NirValue::Binary {
                op: "+".to_string(),
                left: boxed(local("binary_left")),
                right: boxed(local("binary_right")),
                loc: NirSourceLoc::default(),
            },
            NirValue::Unary {
                op: "-".to_string(),
                operand: boxed(local("unary_child")),
                loc: NirSourceLoc::default(),
            },
        ]
    }

    #[test]
    fn walk_value_reaches_every_variant_and_its_children() {
        let mut collector = TagCollector::default();
        for value in one_of_every_value_variant() {
            collector.visit_value(&value);
        }

        // Every top-level variant tag was produced exactly once.
        for tag in [
            "Const",
            "Local",
            "LocalRef",
            "Global",
            "FunctionRef",
            "Closure",
            "Capture",
            "Call",
            "CallResult",
            "RuntimeCall",
            "Constructor",
            "UnionWrap",
            "UnionExtract",
            "ResultIsOk",
            "ResultValue",
            "ResultError",
            "WithUpdate",
            "ListLiteral",
            "MapLiteral",
            "MemberAccess",
            "Binary",
            "Unary",
        ] {
            assert!(
                collector.value_tags.contains(&tag),
                "walk_value never reached the {tag} variant"
            );
        }
        // The composite variants' children were descended into: 19 child
        // `Local`s (each composite carries one, plus the extra WithUpdate,
        // MapLiteral, and Binary children) plus the one standalone `plain`
        // Local at top level = 20.
        let local_count = collector
            .value_tags
            .iter()
            .filter(|t| **t == "Local")
            .count();
        assert_eq!(
            local_count, 20,
            "walk_value did not descend into every composite variant's children"
        );
    }

    #[test]
    fn walk_op_reaches_the_match_guard() {
        // A runtime call reachable *only* through a match-case guard must be
        // visited — this is the exact traversal bug-118 required and bug-328
        // makes uniform.
        let guard_only_call = NirValue::RuntimeCall {
            helper: RuntimeHelper::Thread,
            target: "guard.only.call".to_string(),
            args: vec![],
            loc: NirSourceLoc::default(),
        };
        let op = NirOp::Match {
            value: local("scrutinee"),
            cases: vec![NirMatchCase {
                pattern: NirMatchPattern::Value(local("pattern_value")),
                guard: Some(guard_only_call),
                body: vec![NirOp::Eval {
                    value: local("body_value"),
                }],
            }],
        };

        let mut collector = TagCollector::default();
        collector.visit_op(&op);

        assert!(
            collector.value_tags.contains(&"RuntimeCall"),
            "walk_op did not reach the RuntimeCall hidden in the match-case guard"
        );
        // Scrutinee, pattern value, and body value are all reached too.
        assert_eq!(
            collector
                .value_tags
                .iter()
                .filter(|t| **t == "Local")
                .count(),
            3,
            "walk_op did not reach the scrutinee, pattern, and body values"
        );
    }

    /// A visitor that overrides `visit_op` for one arm and delegates the rest to
    /// `walk_op` still inherits full recursion for the non-overridden arms.
    #[test]
    fn overriding_one_arm_preserves_default_recursion_elsewhere() {
        struct BindNameCollector {
            binds: Vec<String>,
            values: Vec<&'static str>,
        }
        impl NirVisitor for BindNameCollector {
            fn visit_op(&mut self, op: &NirOp) {
                if let NirOp::Bind { name, .. } = op {
                    self.binds.push(name.clone());
                }
                walk_op(self, op);
            }
            fn visit_value(&mut self, value: &NirValue) {
                self.values.push(value_tag(value));
                walk_value(self, value);
            }
        }

        let ops = vec![
            NirOp::Bind {
                mutable: false,
                name: "x".to_string(),
                type_: "Integer".to_string(),
                value: Some(local("x_init")),
            },
            NirOp::If {
                condition: local("cond"),
                then_body: vec![NirOp::Eval {
                    value: local("then_value"),
                }],
                else_body: vec![],
            },
        ];

        let mut collector = BindNameCollector {
            binds: vec![],
            values: vec![],
        };
        collector.visit_ops(&ops);

        assert_eq!(collector.binds, vec!["x".to_string()]);
        // The Bind's init value, the If condition, and the then-body value were
        // all still reached through the default `walk_op` recursion.
        assert_eq!(
            collector.values.iter().filter(|t| **t == "Local").count(),
            3
        );
    }
}
