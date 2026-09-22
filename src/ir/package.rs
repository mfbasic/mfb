use super::*;
use crate::intern::Symbol;
use crate::internal_name;
use crate::types::ParameterType;

/// Namespace a decoded package's own functions and globals by its deterministic
/// identity prefix `<id>.<package>` (see `binary_repr::package_identity_id`),
/// rewriting every internal reference to match. The package's PRIVATE types are
/// scoped by the same identity ([`scope_private_types`]); every type it declares
/// then takes its package-qualified identity `<package>.<Name>`
/// ([`qualify_package_types`], bug-632).
///
/// The `<id>` segment makes the prefix content-addressed: identical packages
/// reached via two dependency paths collapse to one copy at merge time, while
/// two distinct packages that share a name stay separate instead of colliding.
pub fn prefix_package_symbols(
    pir: &mut IrProject,
    id: &str,
    owners: &crate::manifest::package::PackageTypeOwners,
) {
    // Private types are scoped FIRST: that pass gives them an identity-bearing
    // name, and `qualify_package_types` leaves an already-qualified spelling
    // alone, so the two compose instead of qualifying a private type twice.
    scope_private_types(pir, id);
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
    // bug-624 gave the shared walk a `Target::Type` for every type annotation and
    // a `Target::TypeName` for the two VALUE positions that name a type by plain
    // string — a `CASE Variant(x)` pattern (`Local("Variant")`) and an enum member
    // read's target (`MemberAccess { target: Local("Kind") }`). Riding it rather
    // than walking separately is what keeps a position added there qualified here
    // too, the same way `scope_private_types` rides it.
    let qualify = |type_: &mut ParameterType| {
        for_each_nominal(type_, &mut |name| {
            if let Some(owner) = owned.get(name.resolve()) {
                *name = Symbol::intern(&format!("{owner}.{}", name.resolve()));
            }
        });
    };
    let rename_name = |name: &mut String| {
        if let Some(owner) = owned.get(name.as_str()) {
            *name = format!("{owner}.{name}");
        }
    };
    visit_project_targets_mut(pir, &mut |target| match target {
        Target::Type(type_) => qualify(type_),
        Target::TypeName(name) => rename_name(name),
        Target::Function(_) | Target::Global(_) => {}
    });

    // The declaration side, and the three tables the walk above does not reach.
    for type_decl in &mut pir.types {
        rename_name(&mut type_decl.name);
        for include in &mut type_decl.includes {
            rename_name(include);
        }
        for field in &mut type_decl.fields {
            qualify(&mut field.type_);
        }
        for variant in &mut type_decl.variants {
            rename_name(&mut variant.name);
            for field in &mut variant.fields {
                qualify(&mut field.type_);
            }
        }
    }
    for resource in &mut pir.native_resources {
        rename_name(&mut resource.name);
    }
    for link in &mut pir.link_functions {
        for (_, type_) in &mut link.params {
            qualify(type_);
        }
        qualify(&mut link.return_type);
        if let Some(state) = &mut link.return_state_type {
            qualify(state);
        }
    }
    // A `CSTRUCT`'s `maps_to` names one of this package's RECORDS: the layout the
    // marshaller copies the C struct into. Renaming the record without it left
    // the mapping pointing at a name no type has —
    // `CSTRUCT 'SfFormatInfo' maps to 'AudioFormat', which is not a record type`.
    for cstruct in &mut pir.link_cstructs {
        qualify(&mut cstruct.maps_to);
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

/// A name an IR reference points at, as the package rewrite, the
/// initialization-order analysis and the private-type scoping all see it.
enum Target<'a> {
    /// A call target, function reference or closure body name.
    Function(&'a mut String),
    /// A global read or an assignment's global target.
    Global(&'a mut String),
    /// A type annotation: a parameter, return, binding, loop variable or value
    /// node's type (bug-624).
    Type(&'a mut ParameterType),
    /// A value-position string that names a TYPE: a union `MATCH` case's
    /// variant pattern (`Local("Frame")`), or the enum an `Enum.Member`
    /// selection reads (`MemberAccess { target: Local("Color"), type_: Color }`)
    /// (bug-624).
    TypeName(&'a mut String),
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
            f(Target::Type(&mut param.type_));
            if let Some(default) = &mut param.default {
                visit_value_targets_mut(default, f);
            }
        }
        f(Target::Type(&mut function.returns));
    }
    for binding in &mut project.bindings {
        f(Target::Type(&mut binding.type_));
        if let Some(value) = &mut binding.value {
            visit_value_targets_mut(value, f);
        }
    }
    if let Some(entry) = &mut project.entry {
        f(Target::Type(&mut entry.returns));
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
        Target::Type(_) | Target::TypeName(_) => {}
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

/// bug-673: a builtin package's own top-level bindings — the tables and limits its
/// injected source declares, such as json's depth limit or compress's CRC table —
/// initialize before every other binding. The injected files (`builtins/json.mfb`,
/// …) are appended after the program's own files, so their bindings came last, and
/// a program's top-level initializer that called into the package read them as
/// zero (`MUT doc AS json::Json = json::parse(…)` failed every document as "nested
/// too deeply"). A builtin binding is named `__…` in its source, so it reaches
/// here internalized (`#JSON_DEPTH_LIMIT`, `crate::internal_name`) — a name no
/// program can spell. No builtin binding reads a program's or a manifest
/// package's, so moving them to the front, in their own order, is always sound;
/// run it after [`order_bindings_dependencies_first`] so they precede the
/// packages' too.
pub fn order_builtin_bindings_first(bindings: &mut Vec<IrBinding>) {
    let (builtin, rest): (Vec<IrBinding>, Vec<IrBinding>) = std::mem::take(bindings)
        .into_iter()
        .partition(|binding| crate::internal_name::is_builtin_internal(&binding.name));
    bindings.extend(builtin);
    bindings.extend(rest);
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
        IrOp::Bind { type_, value, .. } => {
            f(Target::Type(type_));
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
                    IrMatchPattern::Value(v) => {
                        // A union case's variant pattern lowers to a `Local`
                        // spelled as the variant's type name (`ir::lower`'s
                        // `HirMatchPattern::Union` arm).
                        if let IrValue::Local(name) = v {
                            f(Target::TypeName(name));
                        }
                        visit_value_targets_mut(v, f)
                    }
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
            type_,
            start,
            end,
            step,
            body,
            ..
        } => {
            f(Target::Type(type_));
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
        IrOp::ForEach {
            type_,
            iterable,
            body,
            ..
        } => {
            f(Target::Type(type_));
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
        IrValue::Call { target, type_, .. } | IrValue::CallResult { target, type_, .. } => {
            f(Target::Function(target));
            f(Target::Type(type_));
        }
        IrValue::FunctionRef { name, type_ } | IrValue::Closure { name, type_, .. } => {
            f(Target::Function(name));
            f(Target::Type(type_));
        }
        IrValue::Global(name) => f(Target::Global(name)),
        IrValue::MemberAccess { target, type_, .. } => {
            // `Color.Green` reads the enum through a `Local` spelled as its type
            // name, and yields that same enum type. A field read on a variable
            // yields the field's type, so it never names itself here.
            if let IrValue::Local(name) = target.as_mut() {
                if type_.is_named(name) {
                    f(Target::TypeName(name));
                }
            }
            f(Target::Type(type_));
        }
        IrValue::UnionWrap {
            union_type,
            member_type,
            ..
        } => {
            f(Target::Type(union_type));
            f(Target::Type(member_type));
        }
        IrValue::Const { type_, .. }
        | IrValue::LocalRef { type_, .. }
        | IrValue::Capture { type_, .. }
        | IrValue::Checked { type_, .. }
        | IrValue::Constructor { type_, .. }
        | IrValue::UnionExtract { type_, .. }
        | IrValue::ResultValue { type_, .. }
        | IrValue::WithUpdate { type_, .. }
        | IrValue::ListLiteral { type_, .. }
        | IrValue::SetLiteral { type_, .. }
        | IrValue::MapLiteral { type_, .. }
        | IrValue::Binary { type_, .. }
        | IrValue::Unary { type_, .. } => f(Target::Type(type_)),
        IrValue::Local(_) | IrValue::ResultIsOk { .. } | IrValue::ResultError { .. } => {}
    });
}

/// bug-624: give every PRIVATE type of a decoded package an identity-scoped
/// name, and rewrite every reference to it, so neither the importing program
/// nor another package can see or shadow it.
///
/// `merge_package` de-duplicates types by name, first wins. Left bare, a
/// package's private `TYPE Frame` lost to an importer's own `Frame` and the
/// package's code was verified and lowered against the importer's fields. The
/// new name is `#<id>$<name>` — the file-PRIVATE mangle shape (`mangle_private`)
/// keyed by the package identity instead of a file hash. It carries no `.`, so
/// no qualified-to-bare fallback in `ir::verify` or codegen equates it with a
/// bare `Frame`. It is content-addressed like the function prefix, so a diamond
/// import still collapses to one copy, and two packages that each keep a
/// private `Frame` stay distinct. A diagnostic renders it as `Frame`
/// (`internal_name::display_name`).
///
/// Three kinds of type keep their spelling:
///
/// * The exported surface ([`package_type_surface`]). An importer names those
///   types bare, and native codegen re-registers them from the `.mfp` type
///   exports under that bare name, so renaming the merged definition would
///   split one type in two.
/// * Builtin package types ([`is_builtin_type_name`]).
/// * A declaration nothing references as a nominal. That covers a dead type,
///   which merging drops or keeps harmlessly, and a declaration that shadows a
///   builtin spelling (`TYPE Integer`). The decoder parses that type's references
///   as the scalar, so renaming its definition would orphan them.
fn scope_private_types(pir: &mut IrProject, id: &str) {
    let surface = package_type_surface(pir);
    let mut referenced: HashSet<Symbol> = HashSet::new();
    visit_project_targets_mut(pir, &mut |target| {
        if let Target::Type(type_) = target {
            for_each_nominal(type_, &mut |name| {
                referenced.insert(*name);
            });
        }
    });
    for ty in &mut pir.types {
        for field in ty
            .fields
            .iter_mut()
            .chain(ty.variants.iter_mut().flat_map(|v| v.fields.iter_mut()))
        {
            for_each_nominal(&mut field.type_, &mut |name| {
                referenced.insert(*name);
            });
        }
        referenced.extend(ty.variants.iter().map(|v| Symbol::intern(&v.name)));
        referenced.extend(ty.includes.iter().map(|include| Symbol::intern(include)));
    }

    let renames: HashMap<Symbol, Symbol> = pir
        .types
        .iter()
        .filter_map(|ty| {
            let name = Symbol::intern(&ty.name);
            (!surface.contains(&name)
                && referenced.contains(&name)
                && !is_builtin_type_name(&ty.name))
            .then(|| {
                let scoped = package_private_type_name(id, &ty.name);
                (name, Symbol::intern(&scoped))
            })
        })
        .collect();
    if renames.is_empty() {
        return;
    }

    let rename_name = |name: &mut String| {
        if let Some(scoped) = renames.get(&Symbol::intern(name)) {
            *name = scoped.resolve().to_string();
        }
    };
    let rename_type = |type_: &mut ParameterType| {
        for_each_nominal(type_, &mut |name| {
            if let Some(scoped) = renames.get(name) {
                *name = *scoped;
            }
        });
    };
    visit_project_targets_mut(pir, &mut |target| match target {
        Target::Type(type_) => rename_type(type_),
        Target::TypeName(name) => rename_name(name),
        Target::Function(_) | Target::Global(_) => {}
    });
    for ty in &mut pir.types {
        rename_name(&mut ty.name);
        for include in &mut ty.includes {
            rename_name(include);
        }
        for variant in &mut ty.variants {
            rename_name(&mut variant.name);
        }
        for field in ty
            .fields
            .iter_mut()
            .chain(ty.variants.iter_mut().flat_map(|v| v.fields.iter_mut()))
        {
            rename_type(&mut field.type_);
        }
    }
}

/// The types an importer can reach without naming a private one: every
/// `EXPORT` type, every nominal in an exported function's or binding's
/// signature, and — transitively — every type named by a field, a union
/// variant or an include of a type already in the set. A non-exported record
/// held in an exported record's field is on the surface: the importer receives
/// values of it, and codegen lays the exported record out through it. A package
/// built by this compiler never has one — the build rejects an `EXPORT` naming a
/// non-exported type (`shape::export_names_non_exported_type_diagnostics`,
/// bug-624 B) — so only a `.mfp` from an older compiler reaches that case here.
///
/// Every type a native `LINK` wrapper or `CSTRUCT` names is on it too. Those
/// wrappers route to thunks and resource tables that the importer keys by the
/// names in the `.mfp`'s `RESOURCE_TABLE` and type exports.
fn package_type_surface(pir: &mut IrProject) -> HashSet<Symbol> {
    const EXPORT: &str = "export";
    let mut pending: Vec<Symbol> = Vec::new();
    {
        let mut seed = |type_: &mut ParameterType| {
            for_each_nominal(type_, &mut |name| pending.push(*name));
        };
        for function in pir.functions.iter_mut().filter(|f| f.visibility == EXPORT) {
            for param in &mut function.params {
                seed(&mut param.type_);
            }
            seed(&mut function.returns);
        }
        for binding in pir.bindings.iter_mut().filter(|b| b.visibility == EXPORT) {
            seed(&mut binding.type_);
        }
        for link in &mut pir.link_functions {
            for (_, type_) in &mut link.params {
                seed(type_);
            }
            seed(&mut link.return_type);
            if let Some(state) = &mut link.return_state_type {
                seed(state);
            }
        }
        for cstruct in &mut pir.link_cstructs {
            seed(&mut cstruct.maps_to);
        }
    }
    pending.extend(
        pir.types
            .iter()
            .filter(|ty| ty.visibility == EXPORT)
            .map(|ty| Symbol::intern(&ty.name)),
    );

    let index: HashMap<Symbol, usize> = pir
        .types
        .iter()
        .enumerate()
        .map(|(i, ty)| (Symbol::intern(&ty.name), i))
        .collect();
    let mut surface = HashSet::new();
    while let Some(name) = pending.pop() {
        if !surface.insert(name) {
            continue;
        }
        let Some(&i) = index.get(&name) else {
            continue;
        };
        let ty = &mut pir.types[i];
        for field in ty
            .fields
            .iter_mut()
            .chain(ty.variants.iter_mut().flat_map(|v| v.fields.iter_mut()))
        {
            for_each_nominal(&mut field.type_, &mut |name| pending.push(*name));
        }
        pending.extend(ty.variants.iter().map(|v| Symbol::intern(&v.name)));
        pending.extend(ty.includes.iter().map(|include| Symbol::intern(include)));
    }
    surface
}

/// Whether a type declaration belongs to a builtin package's injected source.
/// Such a type rides in every importer's IR under one compiler-owned spelling.
/// That spelling is either package-qualified (`json.Json`, bug-480) or a sigil
/// internal name (`#json_Node`), so the copies de-duplicate by it and must keep
/// it. A user declaration can carry neither. `.` is field access, and monomorph
/// sanitizes it to `$` in an instance name. `#` cannot be lexed, so it appears
/// only in a file-PRIVATE mangle `#<hash>$<name>`.
pub(super) fn is_builtin_type_name(name: &str) -> bool {
    name.contains('.')
        || (internal_name::strip_sigil(name).is_some()
            && internal_name::private_name_parts(name).is_none())
}

/// The identity-scoped spelling of a package-private type (see
/// [`scope_private_types`]). A type that is already file-PRIVATE
/// (`#<file hash>$Frame`) is re-hashed with the identity. That keeps it distinct
/// from the same file's `Frame` in another package, and from the same package's
/// other files, while keeping the one-hash shape a diagnostic demangles.
fn package_private_type_name(id: &str, name: &str) -> String {
    match internal_name::private_name_parts(name) {
        Some((file_hash, plain)) => internal_name::mangle_private(
            &internal_name::file_scope_hash(&format!("{id}${file_hash}")),
            plain,
        ),
        None => internal_name::mangle_private(id, name),
    }
}

/// Every nominal a type names: a `Named` type, or the template head of a user
/// generic, at any depth. Exhaustive on purpose. A new `ParameterType` variant
/// that can hold a nominal must be walked here, or the private type it names is
/// left unscoped — or, for `shape::export_names_non_exported_type_diagnostics`,
/// unchecked.
pub(super) fn for_each_nominal(type_: &mut ParameterType, f: &mut impl FnMut(&mut Symbol)) {
    match type_ {
        ParameterType::Named(name) => f(name),
        ParameterType::UserOf(name, args) => {
            f(name);
            for arg in args {
                for_each_nominal(arg, f);
            }
        }
        ParameterType::ListOf(inner)
        | ParameterType::SetOf(inner)
        | ParameterType::ResultOf(inner)
        | ParameterType::Res(inner) => for_each_nominal(inner, f),
        ParameterType::MapOf(key, value)
        | ParameterType::MapEntryOf(key, value)
        | ParameterType::Stateful {
            base: key,
            state: value,
        } => {
            for_each_nominal(key, f);
            for_each_nominal(value, f);
        }
        ParameterType::Func(params, returns, _) => {
            for param in params {
                for_each_nominal(param, f);
            }
            for_each_nominal(returns, f);
        }
        ParameterType::ThreadHandle { msg, res, out, .. } => {
            for_each_nominal(msg, f);
            for_each_nominal(res, f);
            for_each_nominal(out, f);
        }
        ParameterType::AttributeString
        | ParameterType::Boolean
        | ParameterType::Byte
        | ParameterType::Integer
        | ParameterType::Fixed
        | ParameterType::Float
        | ParameterType::Money
        | ParameterType::Nothing
        | ParameterType::String
        | ParameterType::C(_)
        | ParameterType::Var(_)
        | ParameterType::Arg(_)
        | ParameterType::Unknown => {}
    }
}
