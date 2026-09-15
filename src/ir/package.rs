use super::*;

/// Namespace a decoded package's own functions and globals by its deterministic
/// identity prefix `<id>.<package>` (see `binary_repr::package_identity_id`),
/// rewriting every internal reference to match. Its own TYPES take the
/// package-qualified identity `<package>.<Name>` ([`qualify_package_types`]).
///
/// The `<id>` segment makes the prefix content-addressed: identical packages
/// reached via two dependency paths collapse to one copy at merge time, while
/// two distinct packages that share a name stay separate instead of colliding.
pub fn prefix_package_symbols(
    pir: &mut IrProject,
    id: &str,
    owners: &crate::manifest::package::PackageTypeOwners,
) {
    qualify_package_types(pir, owners);
    let prefix = format!("{id}.{}", pir.name);
    let mut own_fns: HashSet<String> = pir.functions.iter().map(|f| f.name.clone()).collect();
    let own_globals: HashSet<String> = pir.bindings.iter().map(|b| b.name.clone()).collect();
    // A native `LINK` function's routing name is its package-internal `alias.func`
    // (wrapper bodies reference it unprefixed), yet after merge every package
    // shares one project-global `link_functions` namespace. Fold those routing
    // names into the identity prefix exactly like regular functions, so two
    // packages that independently pick the same alias *and* function name stay
    // distinct and each wrapper routes to its own thunk (bug-251). The alias is
    // load-bearing in three places at once — the merge dedup key, these body
    // references, and the emitted thunk symbol — so qualify it in this one pass
    // and let all three inherit the package-distinct identity together.
    for link in &pir.link_functions {
        own_fns.insert(format!("{}.{}", link.alias, link.name));
    }

    visit_project_targets_mut(pir, &mut |target| {
        qualify_owned_target(target, &own_fns, &own_globals, &prefix)
    });
    for function in &mut pir.functions {
        function.name = format!("{prefix}.{}", function.name);
    }
    for binding in &mut pir.bindings {
        binding.name = format!("{prefix}.{}", binding.name);
    }
    if let Some(entry) = &mut pir.entry {
        entry.name = format!("{prefix}.{}", entry.name);
    }
    // Qualify each LINK function's alias with the identity prefix so its routing
    // name (`alias.func`), its `link_thunk_symbol`, and the merge dedup key all
    // become package-distinct in lockstep with the wrapper-body references
    // rewritten above. The CSTRUCT table joins to its functions by `alias`, so
    // prefix it identically to keep that join intact; a re-export alias routes by
    // its target's `alias.func`, so prefix the target the same way.
    for link in &mut pir.link_functions {
        link.alias = format!("{prefix}.{}", link.alias);
    }
    for cstruct in &mut pir.link_cstructs {
        cstruct.alias = format!("{prefix}.{}", cstruct.alias);
    }
    for (_, target) in &mut pir.link_aliases {
        *target = format!("{prefix}.{target}");
    }
}

/// bug-632: give a decoded package's OWN types — every declared record, union and
/// enum, each union variant, and each native resource type — their
/// package-qualified identity `<package>.<Name>`, in the declarations and at
/// every reference inside the package's IR.
///
/// A package compiles its own types as local names, so its `.mfp` IR spells them
/// bare. Left bare, `merge_package`'s type dedup made a consumer's `A`, this
/// package's `A` and another package's `A` one type: the later ones were
/// discarded and their code verified (and laid out) against the first. The
/// importer already names this package's types `<package>.<Name>` — the parser
/// canonicalizes `binding::Name` to it and `manifest::package` qualifies the
/// package's signatures and layouts the same way — so after this pass both sides
/// name one type, and a diamond's single package still collapses to one copy.
///
/// The package NAME, not the `<id>` prefix functions carry: it is the spelling
/// every importer-side table already keys, and the import rules forbid two
/// different packages of one name in one program. A name already containing a
/// `.` is someone else's (a built-in value type, `net.Url`) and is left alone.
pub(crate) fn qualify_package_types(
    pir: &mut IrProject,
    owners: &crate::manifest::package::PackageTypeOwners,
) {
    let package = pir.name.clone();
    // `owners` comes from the `.mfp` (its type-export and RESOURCE tables), which
    // is the only place a native `RESOURCE`'s name survives — a decoded package
    // IR carries no `native_resources` (`ir/binary.rs` drops them by contract),
    // so `sqlite3.Db` would otherwise stay bare here while every consumer-side
    // table qualified it. A type this package declares but the `.mfp` did not
    // surface (a PRIVATE one) is owned by this package.
    let mut owned: HashMap<String, String> = owners.clone();
    for type_decl in &pir.types {
        owned
            .entry(type_decl.name.clone())
            .or_insert_with(|| package.clone());
        for variant in &type_decl.variants {
            owned
                .entry(variant.name.clone())
                .or_insert_with(|| package.clone());
        }
    }
    for resource in &pir.native_resources {
        owned
            .entry(resource.name.clone())
            .or_insert_with(|| package.clone());
    }
    owned.retain(|name, _| !name.contains('.'));
    if owned.is_empty() {
        return;
    }
    // Two IR positions name a TYPE with a plain string rather than a typed field:
    // a `CASE Variant(x)` pattern (`IrMatchPattern::Value(Local("Variant"))`) and
    // an enum member read's target (`MemberAccess { target: Local("Kind") }`).
    // They are renamed only when the string is one of this package's own type
    // names, which a local binding can never shadow in those positions.
    let enums: HashSet<String> = pir
        .types
        .iter()
        .filter(|type_decl| type_decl.kind == "enum")
        .map(|type_decl| type_decl.name.clone())
        .filter(|name| owned.contains_key(name))
        .collect();
    let rename_name =
        |name: &str| owned.get(name).map(|owner| format!("{owner}.{name}"));
    let rename: &dyn Fn(&str) -> Option<String> = &rename_name;
    let rename_enum_name = |name: &str| {
        enums
            .contains(name)
            .then(|| owned.get(name).map(|owner| format!("{owner}.{name}")))
            .flatten()
    };
    let rename_enum: &dyn Fn(&str) -> Option<String> = &rename_enum_name;
    let names = TypeRenames {
        types: rename,
        enums: rename_enum,
    };
    let rename = &names;

    for type_decl in &mut pir.types {
        rename_in_place(&mut type_decl.name, rename.types);
        for include in &mut type_decl.includes {
            rename_in_place(include, rename.types);
        }
        for field in &mut type_decl.fields {
            qualify_type(&mut field.type_, rename);
        }
        for variant in &mut type_decl.variants {
            rename_in_place(&mut variant.name, rename.types);
            for field in &mut variant.fields {
                qualify_type(&mut field.type_, rename);
            }
        }
    }
    for resource in &mut pir.native_resources {
        rename_in_place(&mut resource.name, rename.types);
    }
    for binding in &mut pir.bindings {
        qualify_type(&mut binding.type_, rename);
        if let Some(value) = &mut binding.value {
            qualify_value_types(value, rename);
        }
    }
    for function in &mut pir.functions {
        for param in &mut function.params {
            qualify_type(&mut param.type_, rename);
            if let Some(default) = &mut param.default {
                qualify_value_types(default, rename);
            }
        }
        qualify_type(&mut function.returns, rename);
        for op in &mut function.body {
            qualify_op_types(op, rename);
        }
    }
    if let Some(entry) = &mut pir.entry {
        qualify_type(&mut entry.returns, rename);
    }
    for link in &mut pir.link_functions {
        for (_, type_) in &mut link.params {
            qualify_type(type_, rename);
        }
        qualify_type(&mut link.return_type, rename);
        if let Some(state) = &mut link.return_state_type {
            qualify_type(state, rename);
        }
    }
}

/// The two renames `qualify_package_types` applies: every owned type name, and
/// the owned ENUM names alone (for an enum-member read's string target).
struct TypeRenames<'a> {
    types: &'a dyn Fn(&str) -> Option<String>,
    enums: &'a dyn Fn(&str) -> Option<String>,
}

fn rename_in_place(name: &mut String, rename: &dyn Fn(&str) -> Option<String>) {
    if let Some(renamed) = rename(name) {
        *name = renamed;
    }
}

fn qualify_type(type_: &mut crate::types::ParameterType, rename: &TypeRenames<'_>) {
    *type_ = type_.map_nominals(&rename.types);
}

/// A `CASE` pattern value that names a union variant (or type) by string.
fn qualify_pattern(pattern: &mut IrValue, rename: &TypeRenames<'_>) {
    if let IrValue::Local(name) = pattern {
        rename_in_place(name, rename.types);
    }
    qualify_value_types(pattern, rename);
}

/// Every type annotation in `value` and its descendants.
fn qualify_value_types(value: &mut IrValue, rename: &TypeRenames<'_>) {
    crate::ir::value::visit_value_mut(value, &mut |node| {
        if let IrValue::MemberAccess { target, .. } = node {
            if let IrValue::Local(name) = target.as_mut() {
                rename_in_place(name, rename.enums);
            }
        }
        qualify_node_type(node, rename);
    });
}

fn qualify_node_type(node: &mut IrValue, rename: &TypeRenames<'_>) {
    match node {
        IrValue::Const { type_, .. }
        | IrValue::LocalRef { type_, .. }
        | IrValue::FunctionRef { type_, .. }
        | IrValue::Closure { type_, .. }
        | IrValue::Capture { type_, .. }
        | IrValue::Call { type_, .. }
        | IrValue::CallResult { type_, .. }
        | IrValue::Checked { type_, .. }
        | IrValue::Constructor { type_, .. }
        | IrValue::UnionExtract { type_, .. }
        | IrValue::ResultValue { type_, .. }
        | IrValue::WithUpdate { type_, .. }
        | IrValue::ListLiteral { type_, .. }
        | IrValue::SetLiteral { type_, .. }
        | IrValue::MapLiteral { type_, .. }
        | IrValue::MemberAccess { type_, .. }
        | IrValue::Binary { type_, .. }
        | IrValue::Unary { type_, .. } => qualify_type(type_, rename),
        IrValue::UnionWrap {
            union_type,
            member_type,
            ..
        } => {
            qualify_type(union_type, rename);
            qualify_type(member_type, rename);
        }
        IrValue::ResultIsOk { .. }
        | IrValue::ResultError { .. }
        | IrValue::Local(_)
        | IrValue::Global(_) => {}
    }
}

/// Every type annotation in `op`, its values and its nested bodies.
fn qualify_op_types(op: &mut IrOp, rename: &TypeRenames<'_>) {
    let body = |ops: &mut Vec<IrOp>| {
        for op in ops {
            qualify_op_types(op, rename);
        }
    };
    match op {
        IrOp::Bind { type_, value, .. } => {
            qualify_type(type_, rename);
            if let Some(value) = value {
                qualify_value_types(value, rename);
            }
        }
        IrOp::Assign { value, .. }
        | IrOp::AssignGlobal { value, .. }
        | IrOp::StateAssign { value, .. }
        | IrOp::Eval { value, .. }
        | IrOp::Fail { error: value, .. }
        | IrOp::ExitProgram { code: value, .. } => qualify_value_types(value, rename),
        IrOp::Return { value, .. } => {
            if let Some(value) = value {
                qualify_value_types(value, rename);
            }
        }
        IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
        IrOp::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            qualify_value_types(condition, rename);
            body(then_body);
            body(else_body);
        }
        IrOp::Match { value, cases, .. } => {
            qualify_value_types(value, rename);
            for case in cases {
                match &mut case.pattern {
                    IrMatchPattern::Else => {}
                    IrMatchPattern::Value(pattern) => qualify_pattern(pattern, rename),
                    IrMatchPattern::OneOf(patterns) => {
                        for pattern in patterns {
                            qualify_pattern(pattern, rename);
                        }
                    }
                }
                if let Some(guard) = &mut case.guard {
                    qualify_value_types(guard, rename);
                }
                body(&mut case.body);
            }
        }
        IrOp::While {
            condition,
            body: loop_body,
            ..
        }
        | IrOp::DoUntil {
            condition,
            body: loop_body,
            ..
        } => {
            qualify_value_types(condition, rename);
            body(loop_body);
        }
        IrOp::For {
            type_,
            start,
            end,
            step,
            body: loop_body,
            ..
        } => {
            qualify_type(type_, rename);
            qualify_value_types(start, rename);
            qualify_value_types(end, rename);
            qualify_value_types(step, rename);
            body(loop_body);
        }
        IrOp::ForEach {
            type_,
            iterable,
            body: loop_body,
            ..
        } => {
            qualify_type(type_, rename);
            qualify_value_types(iterable, rename);
            body(loop_body);
        }
        IrOp::Trap {
            body: trap_body, ..
        } => body(trap_body),
    }
}

/// The package-qualified names (`package.symbol`) by which a *consumer* and
/// other packages reference this package's functions and globals. Computed
/// *before* `prefix_package_symbols` rewrites the definitions into their
/// identity-prefixed `<id>.package.symbol` form, so `apply_package_identity`
/// can rewrite those external references to match.
pub fn package_qualified_reference_names(pir: &IrProject) -> (HashSet<String>, HashSet<String>) {
    let pkg = &pir.name;
    let fns = pir
        .functions
        .iter()
        .map(|f| format!("{pkg}.{}", f.name))
        .collect();
    let globals = pir
        .bindings
        .iter()
        .map(|b| format!("{pkg}.{}", b.name))
        .collect();
    (fns, globals)
}

/// Rewrite every *external* reference to a package's symbols — from the
/// consumer and from other packages — from `package.symbol` to the
/// identity-prefixed `<id>.package.symbol` produced by `prefix_package_symbols`.
/// The package's own internal references are already identity-prefixed and so
/// are not in `fns`/`globals`; they are left untouched.
pub fn apply_package_identity(
    project: &mut IrProject,
    fns: &HashSet<String>,
    globals: &HashSet<String>,
    id: &str,
) {
    visit_project_targets_mut(project, &mut |target| {
        qualify_owned_target(target, fns, globals, id)
    });
}

/// A function or global name an IR reference points at, as the package rewrite
/// and the initialization-order analysis both see it.
enum Target<'a> {
    /// A call target, function reference or closure body name.
    Function(&'a mut String),
    /// A global read or an assignment's global target.
    Global(&'a mut String),
}

fn qualify_owned_target(
    target: Target<'_>,
    fns: &HashSet<String>,
    globals: &HashSet<String>,
    pkg: &str,
) {
    match target {
        Target::Function(name) if fns.contains(name) => qualify_target(name, pkg),
        Target::Global(name) if globals.contains(name) => qualify_target(name, pkg),
        _ => {}
    }
}

/// Every reference in `project`'s function bodies, parameter defaults and
/// binding initializers.
fn visit_project_targets_mut(project: &mut IrProject, f: &mut impl FnMut(Target<'_>)) {
    for function in &mut project.functions {
        for op in &mut function.body {
            visit_op_targets_mut(op, f);
        }
        for param in &mut function.params {
            if let Some(default) = &mut param.default {
                visit_value_targets_mut(default, f);
            }
        }
    }
    for binding in &mut project.bindings {
        if let Some(value) = &mut binding.value {
            visit_value_targets_mut(value, f);
        }
    }
}

/// bug-613: the `package.symbol` names `package` references, collected BEFORE
/// `prefix_package_symbols` so they are spelled the way another package's
/// [`package_qualified_reference_names`] spells its definitions. A package
/// whose references meet another's names depends on it, and its initializer has
/// to run after that one's ([`order_bindings_dependencies_first`]). Takes `&mut`
/// only to share the rewrite's walk; nothing is changed.
pub fn package_referenced_names(package: &mut IrProject) -> HashSet<String> {
    let mut names = HashSet::new();
    visit_project_targets_mut(package, &mut |target| match target {
        Target::Function(name) | Target::Global(name) => {
            names.insert(name.clone());
        }
    });
    names
}

/// One merged package, as [`order_bindings_dependencies_first`] needs it.
pub struct PackageInitialization {
    /// The package's binding names after `prefix_package_symbols`.
    pub bindings: HashSet<String>,
    /// The `package.symbol` names other projects reach it by
    /// ([`package_qualified_reference_names`], functions and globals together).
    pub exports: HashSet<String>,
    /// The `package.symbol` names it reaches ([`package_referenced_names`]).
    pub references: HashSet<String>,
}

/// bug-613: reorder the merged `bindings` so every package initializes before
/// every project that imports it, and the consumer's own bindings initialize
/// last. The global initializer stores bindings in vector order, and the merge
/// appends each package's after the consumer's, in manifest order — so a
/// consumer initializer, or a package initializer calling into another package,
/// read a slot that still held zero.
///
/// `packages` is in merge order. A package depends on another when its
/// references meet that package's exports; packages are emitted in depth-first
/// post-order over that relation, starting from each package in merge order, so
/// unrelated packages keep their merge order. The manifest resolver already
/// refuses a dependency cycle; one that reaches here anyway (two packages
/// sharing a name look mutually dependent) is broken at the back edge rather
/// than looping. Within one project the declaration order is untouched, and a
/// binding no package owns — the consumer's — keeps its relative order at the
/// end.
pub fn order_bindings_dependencies_first(
    bindings: &mut Vec<IrBinding>,
    packages: &[PackageInitialization],
) {
    if packages.is_empty() {
        return;
    }
    let depends_on = |from: usize, to: usize| {
        from != to
            && packages[from]
                .references
                .iter()
                .any(|name| packages[to].exports.contains(name))
    };
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Open,
        Done,
    }
    fn visit(
        index: usize,
        marks: &mut [Mark],
        order: &mut Vec<usize>,
        depends_on: &impl Fn(usize, usize) -> bool,
    ) {
        if marks[index] != Mark::New {
            return;
        }
        marks[index] = Mark::Open;
        for dependency in 0..marks.len() {
            if depends_on(index, dependency) {
                visit(dependency, marks, order, depends_on);
            }
        }
        marks[index] = Mark::Done;
        order.push(index);
    }
    let mut marks = vec![Mark::New; packages.len()];
    let mut order = Vec::with_capacity(packages.len());
    for index in 0..packages.len() {
        visit(index, &mut marks, &mut order, &depends_on);
    }

    let mut remaining: Vec<Option<IrBinding>> =
        std::mem::take(bindings).into_iter().map(Some).collect();
    for index in order {
        for slot in &mut remaining {
            if slot
                .as_ref()
                .is_some_and(|binding| packages[index].bindings.contains(&binding.name))
            {
                bindings.push(slot.take().expect("slot checked above"));
            }
        }
    }
    bindings.extend(remaining.into_iter().flatten());
}

/// Merge a namespaced package `IrProject` into `project`. Functions and globals
/// are de-duplicated by their (already namespaced) name; types by their
/// package-qualified name (bug-632), so two packages' same-named types — or a
/// package's and the consumer's — stay distinct while a diamond collapses.
/// Call `prefix_package_symbols` on `package` first.
pub fn merge_package(project: &mut IrProject, package: IrProject) {
    // bug-342 A8: the five "push if absent" merges below shared the same O(n²)
    // shape, differing only in the identity predicate. `push_unique` folds them
    // into one, preserving the exact order (iterate incoming in order, append
    // each not already present) the hand-rolled loops produced.
    push_unique(&mut project.types, package.types, |a, b| a.name == b.name);
    push_unique(&mut project.bindings, package.bindings, |a, b| {
        a.name == b.name
    });
    push_unique(&mut project.functions, package.functions, |a, b| {
        a.name == b.name
    });
    // Native `LINK` functions are de-duplicated by their `(alias, name)` routing
    // identity. `prefix_package_symbols` has already qualified each imported
    // package's alias with its content-addressed identity prefix, so this key is
    // package-distinct: two packages that independently chose the same alias +
    // function name stay separate (bug-251), while a diamond import — the same
    // package reached twice, hence the same prefix — still collapses to one entry
    // (plan-linker.md §12).
    push_unique(
        &mut project.link_functions,
        package.link_functions,
        |a, b| a.alias == b.alias && a.name == b.name,
    );
    // The CSTRUCT table travels with its LINK functions (plan-50-E): a struct
    // slot's ctype names a declaration in the same alias, so without this an
    // imported binding's struct slots resolve to nothing and are rejected as an
    // unknown ctype. De-duplicated by (alias, name) exactly like the functions.
    push_unique(&mut project.link_cstructs, package.link_cstructs, |a, b| {
        a.alias == b.alias && a.name == b.name
    });
    // A re-export alias is reached by importers as `<package>.<alias>` (the IR
    // normalizes any `IMPORT … AS` binding to the package name), so qualify the
    // bare alias name with the package for routing (plan-link-update.md §5a).
    for (alias_name, target) in package.link_aliases {
        let qualified = format!("{}.{}", package.name, alias_name);
        if !project
            .link_aliases
            .iter()
            .any(|(existing, _)| existing == &qualified)
        {
            project.link_aliases.push((qualified, target));
        }
    }
}

/// bug-342 A8: append each of `items` to `dest` only if `dest` has no element
/// the `same` predicate deems equal — the "push if absent" merge that
/// `merge_package` applied inline six times. Preserves input order (a stable
/// first-wins merge); O(n·m), exactly as the hand-rolled loops were.
fn push_unique<T>(dest: &mut Vec<T>, items: Vec<T>, same: impl Fn(&T, &T) -> bool) {
    for item in items {
        if !dest.iter().any(|existing| same(existing, &item)) {
            dest.push(item);
        }
    }
}

fn qualify_target(name: &mut String, pkg: &str) {
    *name = format!("{pkg}.{name}");
}

fn visit_op_targets_mut(op: &mut IrOp, f: &mut impl FnMut(Target<'_>)) {
    match op {
        IrOp::Bind { value, .. } => {
            if let Some(v) = value {
                visit_value_targets_mut(v, f);
            }
        }
        IrOp::Assign { value, .. }
        | IrOp::StateAssign { value, .. }
        | IrOp::Eval { value, .. }
        | IrOp::Fail { error: value, .. } => visit_value_targets_mut(value, f),
        IrOp::AssignGlobal { name, value, .. } => {
            f(Target::Global(name));
            visit_value_targets_mut(value, f);
        }
        IrOp::Return { value, .. } => {
            if let Some(v) = value {
                visit_value_targets_mut(v, f);
            }
        }
        IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
        IrOp::ExitProgram { code, .. } => visit_value_targets_mut(code, f),
        IrOp::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            visit_value_targets_mut(condition, f);
            for op in then_body.iter_mut().chain(else_body.iter_mut()) {
                visit_op_targets_mut(op, f);
            }
        }
        IrOp::Match { value, cases, .. } => {
            visit_value_targets_mut(value, f);
            for case in cases {
                match &mut case.pattern {
                    IrMatchPattern::Else => {}
                    IrMatchPattern::Value(v) => visit_value_targets_mut(v, f),
                    IrMatchPattern::OneOf(vs) => {
                        for v in vs {
                            visit_value_targets_mut(v, f);
                        }
                    }
                }
                if let Some(guard) = &mut case.guard {
                    visit_value_targets_mut(guard, f);
                }
                for op in &mut case.body {
                    visit_op_targets_mut(op, f);
                }
            }
        }
        IrOp::While {
            condition, body, ..
        } => {
            visit_value_targets_mut(condition, f);
            for op in body {
                visit_op_targets_mut(op, f);
            }
        }
        IrOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            visit_value_targets_mut(start, f);
            visit_value_targets_mut(end, f);
            visit_value_targets_mut(step, f);
            for op in body {
                visit_op_targets_mut(op, f);
            }
        }
        IrOp::DoUntil {
            body, condition, ..
        } => {
            for op in body {
                visit_op_targets_mut(op, f);
            }
            visit_value_targets_mut(condition, f);
        }
        IrOp::ForEach { iterable, body, .. } => {
            visit_value_targets_mut(iterable, f);
            for op in body {
                visit_op_targets_mut(op, f);
            }
        }
        IrOp::Trap { body, .. } => {
            for op in body {
                visit_op_targets_mut(op, f);
            }
        }
    }
}

fn visit_value_targets_mut(value: &mut IrValue, f: &mut impl FnMut(Target<'_>)) {
    // Descend through the shared in-place value seam (bug-328); the per-node
    // work below reports each reference. The seam has no depth cap, matching
    // this walk's original unbounded recursion — a capped walk would leave a
    // deep reference unqualified, or a deep dependency unseen.
    crate::ir::value::visit_value_mut(value, &mut |value| match value {
        IrValue::Call { target, .. } | IrValue::CallResult { target, .. } => {
            f(Target::Function(target))
        }
        IrValue::FunctionRef { name, .. } | IrValue::Closure { name, .. } => {
            f(Target::Function(name))
        }
        IrValue::Global(name) => f(Target::Global(name)),
        _ => {}
    });
}
