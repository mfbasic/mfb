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
    /// Write occurrences (`NirOp::StoreGlobal`) that are NOT the global's own
    /// initializer store. bug-552: the synthetic `__mfb_init_globals_*` SUB
    /// stores every module-level binding exactly once, so counting its stores
    /// here made `writes >= 1` for every global in the language and both proofs
    /// below permanently false.
    pub(crate) writes: usize,
    /// Write occurrences inside the synthetic global-initializer SUB. Kept
    /// separate rather than discarded: the dead-global row must REMOVE these
    /// ops when it removes the global, or it leaves the initializer storing to
    /// a name that no longer exists and the NIR validator refuses the module
    /// ("NIR global store targets unknown global").
    pub(crate) initializer_writes: usize,
    /// Functions that mention it at all, by index into `module.functions`.
    /// Recorded because the *localization* half of the constification row —
    /// sinking a global used by exactly one function into that function —
    /// needs it; that half additionally needs a proof that the value never
    /// carries across calls, so today only the constification half ships.
    pub(crate) functions: HashSet<usize>,
}

impl GlobalUse {
    /// Nothing in the module names it — its own initializer store aside.
    ///
    /// bug-552: the initializer store is deliberately NOT disqualifying. Every
    /// global has one, so counting it here meant no global was ever untouched
    /// and the dead-global row could not fire for any program. A global that
    /// only its own initializer writes, and nothing reads, is exactly the dead
    /// storage this row exists to remove.
    pub(crate) fn untouched(&self) -> bool {
        self.reads == 0 && self.writes == 0
    }

    /// Never written after its initializer — the read-only question.
    ///
    /// `writes` excludes the initializer store, so this now answers the
    /// question the row's name asks. Before bug-552 it asked "is this global
    /// never written at all, including the store that gives it its value?",
    /// which is false for every global in the language.
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
        /// Whether the function being walked is the synthetic
        /// `__mfb_init_globals_*` SUB (bug-552).
        in_initializer: bool,
    }
    impl NirVisitor for Walk<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::StoreGlobal { name, .. } = op {
                let entry = self.census.entry(name.clone()).or_default();
                if self.in_initializer {
                    entry.initializer_writes += 1;
                } else {
                    entry.writes += 1;
                }
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
    let initializer = crate::target::shared::nir::global_initializer_name(&module.project);
    for (index, function) in module.functions.iter().enumerate() {
        let mut walk = Walk {
            census: &mut census,
            function: index,
            in_initializer: function.name == initializer,
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

/// Every function that can run while the global initializer is still running,
/// by name — the initializer SUB itself plus everything it can reach.
///
/// Bindings initialize one at a time, in order (`mfb spec language
/// modules-and-packages`), and a slot not yet initialized reads zero. So a read
/// of a global inside one of these functions may run *before* that global's own
/// store and observe `0`, not its initializer — a program that calls a function
/// from an early initializer which reads a later binding sees exactly that.
/// Replacing such a read with the initializer's literal would change the value
/// the program computes, so neither global-folding row may rewrite a read in a
/// function this returns (bug-744).
///
/// Reachability is by name and conservative: a direct call (`Call` /
/// `CallResult`) whose target is a module function follows the edge, and so
/// does any mention of a function as a value (`FunctionRef`, `Closure`) — a
/// function handed to a builtin such as `collections::forEach` or
/// `thread::start` may be invoked, and nothing here tries to prove it is not.
/// A call to a name that is not a module function (a builtin or runtime helper)
/// runs no user code by itself; any user function it could invoke reaches it
/// as a value argument, which the mention rule already follows.
pub(crate) fn initializer_reachable(module: &NirModule) -> HashSet<String> {
    struct Mentions<'a> {
        functions: &'a HashSet<&'a str>,
        found: Vec<String>,
    }
    impl NirVisitor for Mentions<'_> {
        fn visit_value(&mut self, value: &NirValue) {
            let named = match value {
                NirValue::Call { target, .. } | NirValue::CallResult { target, .. } => Some(target),
                NirValue::FunctionRef { name, .. } | NirValue::Closure { name, .. } => Some(name),
                _ => None,
            };
            if let Some(name) = named {
                if self.functions.contains(name.as_str()) {
                    self.found.push(name.clone());
                }
            }
            walk_value(self, value);
        }
    }

    let functions: HashSet<&str> = module
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect();
    let by_name: HashMap<&str, &crate::target::shared::nir::NirFunction> = module
        .functions
        .iter()
        .map(|function| (function.name.as_str(), function))
        .collect();
    let initializer = crate::target::shared::nir::global_initializer_name(&module.project);
    let mut reached: HashSet<String> = HashSet::new();
    let mut pending = vec![initializer];
    while let Some(name) = pending.pop() {
        if !reached.insert(name.clone()) {
            continue;
        }
        let Some(function) = by_name.get(name.as_str()) else {
            continue;
        };
        let mut mentions = Mentions {
            functions: &functions,
            found: Vec::new(),
        };
        mentions.visit_ops(&function.body);
        pending.extend(
            mentions
                .found
                .into_iter()
                .filter(|found| !reached.contains(found)),
        );
    }
    reached
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

    fn call(target: &str) -> NirValue {
        NirValue::Call {
            target: target.to_string(),
            args: vec![],
            loc: Default::default(),
        }
    }

    /// bug-744: the initializer SUB reaches what it calls, what those call,
    /// and every function it mentions as a value; nothing else. A call to a
    /// name that is no module function (a builtin) is not an edge.
    #[test]
    fn initializer_reachability_follows_calls_and_function_values() {
        let initializer = crate::target::shared::nir::global_initializer_name("test");
        let module = test_module(vec![
            function(
                &initializer,
                vec![
                    NirOp::Eval {
                        value: call("direct"),
                    },
                    NirOp::Eval {
                        value: NirValue::Call {
                            target: "collections.forEach".to_string(),
                            args: vec![NirValue::FunctionRef {
                                name: "callback".to_string(),
                                type_: ParameterType::Unknown,
                            }],
                            loc: Default::default(),
                        },
                    },
                ],
            ),
            function(
                "direct",
                vec![NirOp::Eval {
                    value: call("transitive"),
                }],
            ),
            function("transitive", vec![]),
            function(
                "callback",
                vec![NirOp::Eval {
                    value: NirValue::Closure {
                        name: "lambda".to_string(),
                        type_: ParameterType::Unknown,
                        captures: vec![],
                    },
                }],
            ),
            function("lambda", vec![]),
            function(
                "main",
                vec![NirOp::Eval {
                    value: call("direct"),
                }],
            ),
            function("unrelated", vec![]),
        ]);
        let reached = initializer_reachable(&module);
        let mut names: Vec<&str> = reached.iter().map(String::as_str).collect();
        names.sort_unstable();
        let mut expected = vec![
            initializer.as_str(),
            "callback",
            "direct",
            "lambda",
            "transitive",
        ];
        expected.sort_unstable();
        assert_eq!(names, expected);
    }
}
