//! Whole-module global-usage census — the fact base the three Opt1 global
//! rows share: dead global elimination, global localization/constification,
//! and read-only memory inference.
//!
//! All three ask variations of one question: *who touches this global?* The
//! census answers it once per module, over the sanctioned read-only traversal
//! seam (`nir::visit`), so the three rows cannot disagree about a global's
//! usage and the recursion cannot drift from the one authoritative walk.
//!
//! Per global it records: how many functions read it, how many write it (via
//! `StoreGlobal`), and which single function holds all of its uses when
//! exactly one does. Everything is deliberately whole-module and
//! conservative — a global whose name is also a local's name simply looks
//! more used, never less, because `Global`/`StoreGlobal` are matched by
//! variant rather than by name lookup.
//!
//! One structural fact the rows depend on: an entry point or an exported
//! (`public`) global is reachable from outside this module's function set, so
//! the census marks it as escaping and no row may act on it.

use std::collections::{HashMap, HashSet};

use crate::target::shared::nir::visit::{walk_op, walk_value, NirVisitor};
use crate::target::shared::nir::{NirModule, NirOp, NirValue};

/// What one global's usage looks like across the whole module.
#[derive(Default, Clone)]
pub(crate) struct GlobalUse {
    /// Total read occurrences (`NirValue::Global`).
    pub(crate) reads: usize,
    /// Total write occurrences (`NirOp::StoreGlobal`).
    pub(crate) writes: usize,
    /// Functions that mention it at all, by index into `module.functions`.
    /// Recorded because the *localization* half of the constification row —
    /// sinking a global used by exactly one function into that function —
    /// needs it; that half additionally needs a proof that the value never
    /// carries across calls, so today only the constification half ships.
    pub(crate) functions: HashSet<usize>,
}

impl GlobalUse {
    /// Nothing in the module names it.
    pub(crate) fn untouched(&self) -> bool {
        self.reads == 0 && self.writes == 0
    }

    /// Never written after its initializer — the read-only question.
    pub(crate) fn never_written(&self) -> bool {
        self.writes == 0
    }
}

/// The census, keyed by global name.
pub(crate) type Census = HashMap<String, GlobalUse>;

/// Census every global mention in the module's functions.
pub(crate) fn census(module: &NirModule) -> Census {
    struct Walk<'a> {
        census: &'a mut Census,
        function: usize,
    }
    impl NirVisitor for Walk<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::StoreGlobal { name, .. } = op {
                let entry = self.census.entry(name.clone()).or_default();
                entry.writes += 1;
                entry.functions.insert(self.function);
            }
            walk_op(self, op);
        }
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::Global { name, .. } = value {
                let entry = self.census.entry(name.clone()).or_default();
                entry.reads += 1;
                entry.functions.insert(self.function);
            }
            walk_value(self, value);
        }
    }

    let mut census = Census::new();
    for (index, function) in module.functions.iter().enumerate() {
        let mut walk = Walk {
            census: &mut census,
            function: index,
        };
        walk.visit_ops(&function.body);
    }
    // A global's own initializer may read another global; count those too, so
    // a global kept alive only by another live global's initializer is not
    // mistaken for dead. They belong to no function, so they never make a
    // global look function-local.
    for global in &module.globals {
        if let Some(value) = &global.value {
            struct Init<'a> {
                census: &'a mut Census,
            }
            impl NirVisitor for Init<'_> {
                fn visit_value(&mut self, value: &NirValue) {
                    if let NirValue::Global { name, .. } = value {
                        self.census.entry(name.clone()).or_default().reads += 1;
                    }
                    walk_value(self, value);
                }
            }
            let mut init = Init {
                census: &mut census,
            };
            init.visit_value(value);
        }
    }
    census
}

/// Whether a global is visible outside this module's own function set — an
/// exported one may be read or written by an importer, so no row may reason
/// about its usage from this module alone.
pub(crate) fn escapes(global: &crate::target::shared::nir::NirGlobal) -> bool {
    global.visibility != "private"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimizer::opt1::local_rewrites::testutil::*;
    use crate::target::shared::nir::{NirFunction, NirGlobal};
    use crate::types::ParameterType;
    use std::collections::HashMap as StdHashMap;

    fn global(name: &str, mutable: bool) -> NirGlobal {
        NirGlobal {
            name: name.to_string(),
            symbol: format!("_g_{name}"),
            visibility: "private".to_string(),
            mutable,
            type_: ParameterType::Integer,
            value: Some(int_const("0")),
        }
    }

    fn function(name: &str, body: Vec<NirOp>) -> NirFunction {
        NirFunction {
            name: name.to_string(),
            visibility: "private".to_string(),
            kind: "function".to_string(),
            isolated: false,
            params: vec![],
            returns: ParameterType::Integer,
            body,
            file: "main.mfb".to_string(),
            resource_owners: StdHashMap::new(),
        }
    }

    fn read(name: &str) -> NirValue {
        NirValue::Global {
            name: name.to_string(),
            type_: ParameterType::Integer,
        }
    }

    #[test]
    fn census_separates_reads_writes_and_owning_functions() {
        let mut module = test_module(vec![
            function(
                "a",
                vec![NirOp::Eval {
                    value: read("shared"),
                }],
            ),
            function(
                "b",
                vec![
                    NirOp::Eval {
                        value: read("shared"),
                    },
                    NirOp::StoreGlobal {
                        name: "written".to_string(),
                        type_: ParameterType::Integer,
                        value: Some(int_const("1")),
                    },
                    NirOp::Eval {
                        value: read("local_only"),
                    },
                ],
            ),
        ]);
        module.globals = vec![global("shared", false), global("written", true)];
        let census = census(&module);

        let shared = &census["shared"];
        assert_eq!(shared.reads, 2);
        assert!(shared.never_written());
        assert_eq!(shared.functions.len(), 2, "two functions read it");

        let written = &census["written"];
        assert_eq!(written.writes, 1);
        assert!(!written.never_written());

        let local = &census["local_only"];
        assert_eq!(
            local.functions.iter().copied().collect::<Vec<_>>(),
            vec![1],
            "only function b names it"
        );
        assert!(!local.untouched());
    }

    /// A global nothing mentions is absent from the census entirely — the
    /// dead-global row's signal.
    #[test]
    fn unmentioned_globals_are_untouched() {
        let mut module = test_module(vec![function("a", vec![])]);
        module.globals = vec![global("unused", false)];
        let census = census(&module);
        assert!(census
            .get("unused")
            .cloned()
            .unwrap_or_default()
            .untouched());
    }

    /// A read from another global's initializer counts, so a global kept
    /// alive only that way is not mistaken for dead.
    #[test]
    fn initializer_reads_count() {
        let mut module = test_module(vec![function("a", vec![])]);
        let mut holder = global("holder", false);
        holder.value = Some(read("seed"));
        module.globals = vec![global("seed", false), holder];
        let census = census(&module);
        assert_eq!(census["seed"].reads, 1);
        assert!(!census["seed"].untouched());
    }
}
