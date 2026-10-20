//! plan-176-Q: global string capacities — the companion capacity global of a `MUT` string global.
//! The arm table and `try_inplace_self_update` live in `collection/assign/self_update/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// A global `String`'s capacity shadow (plan-142-H Open Decision 1).
// ---------------------------------------------------------------------------

/// The hidden global holding the spare capacity of the global `String` `name`'s
/// self-append buffer. `$` keeps it out of every user namespace.
fn global_string_capacity_name(name: &str) -> String {
    format!("$strcap${name}")
}

/// Declare a hidden `Integer` global beside every global `String` that is the
/// target of a self-append (`gs = gs & t`), or of another `String` self-update
/// whose arm needs a shadow (`is_string_self_update`, plan-146-B), anywhere in
/// `module`: the concat arm
/// keeps the buffer's spare capacity there, as it keeps a local's in a frame
/// slot. Every other store to the global frees with it and resets it to 0
/// (`StoreGlobal`), and the global's own initializer is such a store, so it starts
/// at 0. Runs after the optimizer, which would otherwise drop storage no NIR op
/// names.
pub(crate) fn add_global_string_capacities(module: &mut NirModule) {
    struct Finder<'m> {
        strings: &'m std::collections::HashSet<String>,
        found: std::collections::BTreeSet<String>,
    }
    impl NirVisitor for Finder<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::StoreGlobal {
                name,
                value: Some(value),
                ..
            } = op
            {
                if self.strings.contains(name)
                    && crate::codegen::collection::assign::string_self_update::is_string_self_update(
                        value,
                        &|root| matches!(root, NirValue::Global { name: g, .. } if g == name),
                    )
                {
                    self.found.insert(name.clone());
                }
            }
            crate::target::shared::nir::visit::walk_op(self, op);
        }
    }
    let strings: std::collections::HashSet<String> = module
        .globals
        .iter()
        .filter(|global| global.type_ == ParameterType::String)
        .map(|global| global.name.clone())
        .collect();
    let mut finder = Finder {
        strings: &strings,
        found: std::collections::BTreeSet::new(),
    };
    for function in &module.functions {
        finder.visit_ops(&function.body);
    }
    for name in finder.found {
        let hidden = global_string_capacity_name(&name);
        if module.globals.iter().any(|global| global.name == hidden) {
            continue;
        }
        module.globals.push(NirGlobal {
            symbol: crate::target::shared::nir::global_symbol(&module.project, &hidden),
            name: hidden,
            visibility: "private".to_string(),
            mutable: true,
            type_: ParameterType::Integer,
            value: None,
        });
    }
}

impl CodeBuilder<'_> {
    /// The hidden global holding the global `String` `name`'s self-append
    /// capacity, when `add_global_string_capacities` declared one.
    pub(crate) fn global_string_capacity(&self, name: &str) -> Option<String> {
        let hidden = global_string_capacity_name(name);
        self.globals.contains_key(&hidden).then_some(hidden)
    }
}
