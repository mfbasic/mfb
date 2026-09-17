//! bug-648: whether the stores into an inline-`TRAP` temp hand it ownership.
//!
//! The inline-`TRAP` desugar (`ir/lower.rs`) routes a trapped value through a temp
//! bound with no initializer:
//!
//! ```text
//! bind $trap_resN = callResult f(…)
//! bind MUT $trap_valN : T
//! if resultIsOk($trap_resN) { $trap_valN = resultValue($trap_resN) }
//! else                      { …handler…; $trap_valN = <RECOVER value> }
//! bind y = $trap_valN
//! ```
//!
//! `y` is an alias (`value_aliases_live_resource` answers `Local`), so for a
//! resource `T` the temp is the binding that carries the close obligation. A bind
//! with no value gives the bind-time classification nothing to look at, so the temp
//! used to own every value assigned to it. Two kinds of store are not its to close:
//!
//! * the `resultValue` of a call whose resource result is BORROWED
//!   (`CodeBuilder::target_returns_borrowed_resource`: `tcp`/`udp`/`tls::poll` over
//!   a list, `collections::get`/`getOr`) — the list still owns and closes it;
//! * a `RECOVER` value that only names a live resource (`RECOVER outer`, a field, a
//!   borrowed call) — exactly the shapes a `RES x = <value>` bind treats as an alias.
//!
//! Closing either at the temp's scope exit closed a handle its owner still held, and
//! for the `tcp`/`udp`/`tls` records freed the record too.
//!
//! Ownership is a property of each STORE, not of the temp: a borrowed success can
//! recover an owned handle and an owned success can recover an alias. And the temp
//! is never empty — its bind materializes a closed default record it DOES own, and
//! the first store's old-value drop is what releases it (bug-643). So a temp that
//! receives any borrowed store keeps its cleanup and gains a flag slot meaning "the
//! value in the slot is owned": set when the default is bound, rewritten at every
//! store, and tested by the old-value drop and the scope drop alike. A temp whose
//! every store is owned is not in [`TrapOwnership::lent_temps`] and lowers exactly
//! as before.

use crate::codegen::engine::builder::CodeBuilder;
use crate::target::shared::nir::visit::{walk_op, NirVisitor};
use crate::target::shared::nir::{NirOp, NirValue};
use std::collections::{HashMap, HashSet};

/// The facts one function's lowering consults.
#[derive(Default)]
pub(crate) struct TrapOwnership {
    /// Every value-less `Bind` that receives at least one borrowed store. Type-blind
    /// on purpose — `Local(s)` into a `String` temp lands here too — so the consumer
    /// asks only about resource-typed temps.
    pub(crate) lent_temps: HashSet<String>,
    /// Locals bound to a call whose resource result is borrowed.
    borrowed_results: HashSet<String>,
}

impl TrapOwnership {
    /// Whether storing `value` into a temp hands it an owned resource — the rule
    /// [`Self::lent_temps`] is built from, asked of the one `Assign` codegen is
    /// lowering.
    pub(crate) fn store_is_owned(&self, value: &NirValue) -> bool {
        match value {
            NirValue::ResultValue { value } => !matches!(
                value.as_ref(),
                NirValue::Local(result) if self.borrowed_results.contains(result)
            ),
            other => !CodeBuilder::value_aliases_live_resource(other),
        }
    }
}

pub(crate) fn collect_trap_ownership(ops: &[NirOp]) -> TrapOwnership {
    #[derive(Default)]
    struct Collector {
        value_less: HashSet<String>,
        borrowed_results: HashSet<String>,
        stores: HashMap<String, Vec<NirValue>>,
    }
    impl NirVisitor for Collector {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind {
                    name, value: None, ..
                } => {
                    self.value_less.insert(name.clone());
                }
                NirOp::Bind {
                    name,
                    value:
                        Some(
                            NirValue::Call { target, .. }
                            | NirValue::CallResult { target, .. }
                            | NirValue::RuntimeCall { target, .. },
                        ),
                    ..
                } if CodeBuilder::target_returns_borrowed_resource(target) => {
                    self.borrowed_results.insert(name.clone());
                }
                NirOp::Assign { name, value } => {
                    self.stores
                        .entry(name.clone())
                        .or_default()
                        .push(value.clone());
                }
                _ => {}
            }
            walk_op(self, op);
        }
    }
    let mut collector = Collector::default();
    collector.visit_ops(ops);
    let mut ownership = TrapOwnership {
        lent_temps: HashSet::new(),
        borrowed_results: collector.borrowed_results,
    };
    for (name, values) in collector.stores {
        if collector.value_less.contains(&name)
            && values.iter().any(|value| !ownership.store_is_owned(value))
        {
            ownership.lent_temps.insert(name);
        }
    }
    ownership
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::nir::NirSourceLoc;
    use crate::types::ParameterType;

    fn call_result(target: &str) -> NirValue {
        NirValue::CallResult {
            target: target.to_string(),
            args: Vec::new(),
            loc: NirSourceLoc::default(),
        }
    }

    fn local(name: &str) -> NirValue {
        NirValue::Local(name.to_string())
    }

    fn result_value(name: &str) -> NirValue {
        NirValue::ResultValue {
            value: Box::new(local(name)),
        }
    }

    /// The desugar's shape: a trapped call, a value-less temp, the Ok store, and an
    /// optional RECOVER store in the handler branch.
    fn trap(target: &str, recover: Option<NirValue>) -> Vec<NirOp> {
        let mut else_body = Vec::new();
        if let Some(value) = recover {
            else_body.push(NirOp::Assign {
                name: "$trap_val1".to_string(),
                value,
            });
        }
        vec![
            NirOp::Bind {
                mutable: false,
                name: "$trap_res0".to_string(),
                type_: ParameterType::parse("Result OF udp.Socket"),
                value: Some(call_result(target)),
            },
            NirOp::Bind {
                mutable: true,
                name: "$trap_val1".to_string(),
                type_: ParameterType::parse("udp.Socket"),
                value: None,
            },
            NirOp::If {
                condition: local("ok"),
                then_body: vec![NirOp::Assign {
                    name: "$trap_val1".to_string(),
                    value: result_value("$trap_res0"),
                }],
                else_body,
            },
        ]
    }

    /// A producer's success is owned, and so is a fresh RECOVER: the temp keeps the
    /// pre-bug-648 lowering.
    #[test]
    fn an_owned_success_and_an_owned_recover_are_not_lent() {
        let ops = trap("udp.bind", Some(call_result("udp.bind")));
        assert!(!collect_trap_ownership(&ops).lent_temps.contains("$trap_val1"));
    }

    #[test]
    fn a_borrowed_success_is_lent_whatever_it_recovers() {
        for target in ["udp.poll", "tcp.poll", "tls.poll", "collections.get"] {
            for recover in [None, Some(local("outer")), Some(call_result("udp.bind"))] {
                let ops = trap(target, recover.clone());
                assert!(
                    collect_trap_ownership(&ops).lent_temps.contains("$trap_val1"),
                    "{target} recovering {recover:?}"
                );
            }
        }
    }

    #[test]
    fn an_owned_success_recovering_an_alias_is_lent() {
        let ops = trap("udp.bind", Some(local("outer")));
        assert!(collect_trap_ownership(&ops).lent_temps.contains("$trap_val1"));
    }

    /// Only a value-less bind is a trap temp: a bound local assigned an alias later
    /// keeps its own rules.
    #[test]
    fn a_bind_with_an_initializer_is_never_lent() {
        let mut ops = trap("udp.poll", None);
        ops[1] = NirOp::Bind {
            mutable: true,
            name: "$trap_val1".to_string(),
            type_: ParameterType::parse("udp.Socket"),
            value: Some(call_result("udp.bind")),
        };
        assert!(!collect_trap_ownership(&ops).lent_temps.contains("$trap_val1"));
    }

    #[test]
    fn the_per_store_answer_matches_the_classification() {
        let ownership = collect_trap_ownership(&trap("udp.poll", None));
        assert!(!ownership.store_is_owned(&result_value("$trap_res0")));
        assert!(ownership.store_is_owned(&result_value("$trap_res9")));
        assert!(!ownership.store_is_owned(&local("outer")));
        assert!(ownership.store_is_owned(&call_result("udp.bind")));
        assert!(!ownership.store_is_owned(&call_result("collections.get")));
    }
}
