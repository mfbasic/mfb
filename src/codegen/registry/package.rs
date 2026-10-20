//! plan-176-M: `RegistryPackage`, the per-package table the registry is built from, and its
//! builders. `Registry` and `build()` live in `codegen/registry/mod.rs`.

use super::*;

/// One builtin package: its import name, documentation, the packages it imports, its
/// records, unions, and functions. Fields are private — construct via
/// [`Registry::add_package`] and fill with [`RegistryPackage::add_imports`] /
/// [`RegistryPackage::add_record`] / [`RegistryPackage::add_union`] /
/// [`RegistryPackage::add_function`].
#[derive(Debug)]
pub(crate) struct RegistryPackage {
    pub(super) import_name: &'static str,
    intro: &'static str,
    desc: &'static str,
    imports: Vec<&'static str>,
    pub(super) records: Vec<RegistryRecord>,
    pub(super) unions: Vec<RegistryUnion>,
    pub(super) enums: Vec<RegistryEnum>,
    /// Opaque resource handle types. Semantic-only (not injectable source), so they are
    /// not rendered by [`get_mfb`](Self::get_mfb).
    resources: Vec<RegistryResource>,
    /// Shared source chunks (or ordering edges) the package contributes to an injected
    /// program, each gated by a [`HelperGate`] (plan-99). An [`Always`](HelperGate::Always)
    /// body renders inline in [`get_mfb`](Self::get_mfb) between the unions and the member
    /// bodies (the old `helper_functions` position); gated bodies are injected as their
    /// own synthetic files by [`Registry::augment_project`].
    helpers: Vec<RegistryHelper>,
    pub(super) functions: Vec<RegistryFunction>,
    /// Parameters that **consume the resources reachable from their argument**
    /// (plan-116-J). `canvas::setGroup`'s `items` is the first: the group takes over
    /// closing every `canvas::Image`/`canvas::Font` named by the `DrawItem`s in the list,
    /// so the caller's bindings must be marked moved or scope-drop closes them out from
    /// under the group.
    ///
    /// **Data, not a field on [`Parameter`], deliberately.** A required field would have
    /// to be spelled at all 726 `Parameter { … }` literals in the tree to say "no" 725
    /// times, which buries the one site that says yes. The `add_*` builder shape is how
    /// this registry already carries per-package facts that only a handful of members
    /// have (`source_types`, `constants`, `overrides`).
    ///
    /// **Keyed by (function, parameter NAME), not by index**, because a parameter's
    /// position is not stable under an overload split and a name-keyed verdict over an
    /// overloaded builtin is wrong for exactly one overload if it is keyed any other way.
    consuming_params: Vec<ConsumingParameter>,
    /// Value-type names (`EXPORT TYPE`/`ENUM`) a package declares **only** in its
    /// injected companion source (`package.mfb`) rather than as a modeled
    /// [`RegistryRecord`]/[`RegistryEnum`] — `datetime`'s `Instant`/`Date`/…/`ZoneKind`,
    /// whose `DOC`-block-carrying declarations and byte-exact formatting cannot be
    /// reproduced by [`get_mfb`](Self::get_mfb)'s renderers. Recorded as semantic-only
    /// facts so [`is_builtin_type`] / [`qualified_builtin_type`] recognize them without
    /// a per-package predicate; they are NOT rendered (the companion already declares
    /// them).
    pub(super) source_types: Vec<&'static str>,
    /// Compile-time package constants (scalar folds + record-constructor inlines) the
    /// package owns — the registry home of the `math`/`errorcode`/`vector` constant
    /// hand tables. Queried by the [`is_package_constant`] / [`constant_type`] /
    /// [`constant_value`] / [`constant_components`] boundary fns.
    constants: Vec<RegistryConstant>,
    /// General-builtin overrides (`toString`, …) this package provides for its value
    /// types — the registry home of the `builtins::general_override_target` rows.
    /// Queried by [`general_override_target`].
    overrides: Vec<RegistryOverride>,
    /// Whether this package's members are **unqualified global** builtins — bare
    /// names (`expectEqual`, never `testing::expectEqual`) that carry no writable
    /// `IMPORT <pkg>` spelling. The package is registered under a real name only so it
    /// has a home in the registry; the member calls stay bare end-to-end. Set for the
    /// `testing` (and, later, `general`) migrations. Its one behavioral effect is
    /// documentation: `mfb man2 --all` skips such a package, because rendering a
    /// `# testing` / `testing::expect` page would advertise a spelling users cannot
    /// write. Defaults `false`; set via [`mark_unqualified_global`](Self::mark_unqualified_global).
    ///
    /// **`testing` and `general` are the globals holders, by design.** `testing` holds the
    /// globals for testing; `general` holds the globals for the language. A program never
    /// needs to write `testing::` or `general::` — the members *are* the globals and are
    /// called bare (`expectEqual(a, b)`, `len(s)`). The package name is only their home in
    /// the registry and their grouping in the documentation, so a `testing::expectEqual` /
    /// `general::len` spelling on these packages' own man pages is expected, not a defect.
    /// bug-607 reported it as one and was closed as NOT A BUG
    /// (`bugs/skipped/bug-607-man-renders-unqualified-globals-with-package-prefix.md`);
    /// do not re-file it.
    unqualified_global: bool,
}

impl RegistryPackage {
    /// An empty package with the given import name and documentation. Fill it with
    /// [`add_imports`](Self::add_imports) / [`add_record`](Self::add_record) /
    /// [`add_union`](Self::add_union) / [`add_helper_functions`](Self::add_helper_functions)
    /// / [`add_function`](Self::add_function), then hand it to [`Registry::add_package`].
    pub(crate) fn new(import_name: &'static str, intro: &'static str, desc: &'static str) -> Self {
        Self {
            import_name,
            intro,
            desc,
            imports: Vec::new(),
            records: Vec::new(),
            unions: Vec::new(),
            enums: Vec::new(),
            resources: Vec::new(),
            helpers: Vec::new(),
            functions: Vec::new(),
            consuming_params: Vec::new(),
            source_types: Vec::new(),
            constants: Vec::new(),
            overrides: Vec::new(),
            unqualified_global: false,
        }
    }

    /// The package's import name, e.g. `"encoding"`.
    pub(crate) fn import_name(&self) -> &'static str {
        self.import_name
    }
    /// Whether this package is an **unqualified global** builtin package (see the
    /// [`unqualified_global`](Self::unqualified_global) field). `mfb man2 --all` skips
    /// packages for which this is true.
    pub(crate) fn is_unqualified_global(&self) -> bool {
        self.unqualified_global
    }
    /// Mark this package as an **unqualified global** builtin package — its members are
    /// bare names with no writable `IMPORT <pkg>` spelling, so `mfb man2 --all` skips
    /// its documentation page (see the field docs). Used by the `testing`/`general`
    /// migrations.
    pub(crate) fn mark_unqualified_global(&mut self) -> &mut Self {
        self.unqualified_global = true;
        self
    }
    /// One-line documentation intro.
    pub(crate) fn intro(&self) -> &'static str {
        self.intro
    }
    /// Full documentation description.
    pub(crate) fn desc(&self) -> &'static str {
        self.desc
    }
    /// The package's functions, in registration order.
    pub(crate) fn functions(&self) -> &[RegistryFunction] {
        &self.functions
    }
    /// The function with this public name, or `None`.
    pub(crate) fn function(&self, name: &str) -> Option<&RegistryFunction> {
        self.functions.iter().find(|f| f.name == name)
    }

    /// The packages this one imports, in the order they were added.
    pub(crate) fn imports(&self) -> &[&'static str] {
        &self.imports
    }

    /// The package's records, in declaration order.
    pub(crate) fn records(&self) -> &[RegistryRecord] {
        &self.records
    }

    /// The package's unions, in declaration order.
    pub(crate) fn unions(&self) -> &[RegistryUnion] {
        &self.unions
    }

    /// The package's enums, in declaration order.
    pub(crate) fn enums(&self) -> &[RegistryEnum] {
        &self.enums
    }

    /// The package's resource types, in declaration order.
    pub(crate) fn resources(&self) -> &[RegistryResource] {
        &self.resources
    }

    /// The package's gated helper chunks/edges, in the order added.
    pub(crate) fn helpers(&self) -> &[RegistryHelper] {
        &self.helpers
    }

    /// The bodies of this package's [`Always`](HelperGate::Always) helpers, in add
    /// order — the chunks that render inline in [`get_mfb`](Self::get_mfb) (the old
    /// `helper_functions`). Gated helpers (`WhenUsed`/`WhenImported`) are injected as
    /// separate files by [`Registry::augment_project`] and are excluded here.
    ///
    /// A [`debug_split`](RegistryHelper::debug_split) pair contributes the half that
    /// matches `debug`.
    pub(super) fn always_helper_bodies(&self, debug: bool) -> Vec<&'static str> {
        self.helpers
            .iter()
            .filter(|h| match h.gate {
                HelperGate::Always => true,
                HelperGate::NormalBuildOnly => !debug,
                HelperGate::DebugBuildOnly => debug,
                HelperGate::WhenUsed(_)
                | HelperGate::WhenImported(_)
                | HelperGate::WhenBothImported(..)
                | HelperGate::WhenUsedOrConstructed(..)
                | HelperGate::WhenNotConstructed(_)
                | HelperGate::WhenCalledOrNamed(..)
                | HelperGate::WhenNeitherCalledNorNamed(..) => false,
            })
            .filter_map(|h| h.body)
            .collect()
    }

    /// The package's source-declared value-type names (see [`source_types`](Self::source_types)).
    pub(crate) fn source_types(&self) -> &[&'static str] {
        &self.source_types
    }

    /// Whether `ast` imports this package — the generic replacement for the old
    /// per-package `uses_package`. A property of the *program being compiled*, so it
    /// takes the AST rather than being a stored flag.
    pub(crate) fn is_imported_by(&self, view: &ProjectView) -> bool {
        view.imports(self.import_name)
    }

    /// Assemble this package's complete injectable MFBASIC source — the modern
    /// equivalent of a hand-written `package.mfb` (e.g.
    /// `codegen/builtins/csv/package.mfb`), reconstructed from the package's own
    /// imports, records, unions, helper functions, and members' [`Body::Mfb`] bodies.
    ///
    /// The output is, in order:
    ///
    /// 1. the package's [`imports`](Self::imports), as a leading block of
    ///    `IMPORT <name>` lines (imports must precede all declarations in MFBASIC);
    /// 2. every [`RegistryRecord`] as an `[EXPORT] TYPE … END TYPE` block (in
    ///    `add_record` order);
    /// 3. every [`RegistryUnion`] as an `[EXPORT] UNION … END UNION` block (in
    ///    `add_union` order);
    /// 4. every [`RegistryEnum`] as an `[EXPORT] ENUM … END ENUM` block (in `add_enum`
    ///    order);
    /// 5. the [`helper_functions`](Self::helper_functions) — the shared `__pkg_*`
    ///    helpers the members call, in the order added;
    /// 6. every [`Body::Mfb`] body across all functions (each overload counts — a
    ///    same-named native-overload set like `encoding::utf8Encode` emits one `FUNC`
    ///    per overload), in registration order.
    ///
    /// Bodies keep their raw `__pkg_name` spelling; internalization to `#pkg_name`
    /// happens later when the assembled text is parsed.
    ///
    /// Returns the **empty string** only when the package has *nothing* to inject —
    /// no records, no unions, no enums, no `Mfb` member, and no helper functions.
    /// Records, unions, and enums are injectable source in their own right (a package
    /// whose functions are all `Native`/`Rewrite` still emits its `TYPE`/`UNION`/`ENUM`
    /// declarations here), and a
    /// helper-function chunk is likewise standalone injectable source — the native
    /// `process` package carries only its `Stream`/`Signal` `EXPORT ENUM` companion as
    /// a helper chunk, with no records, unions, or `Mfb` bodies. Pieces are
    /// separated by a blank line and the result ends with a newline, so the output is
    /// directly parseable.
    pub(crate) fn get_mfb(&self) -> String {
        self.get_mfb_for(false)
    }

    /// [`get_mfb`](Self::get_mfb) for a build with (`debug`) or without `--debug`: the
    /// same assembly, with each [`debug_split`](RegistryHelper::debug_split) pair
    /// contributing the matching half (plan-130-E). `get_mfb` is the normal build's.
    pub(crate) fn get_mfb_for(&self, debug: bool) -> String {
        let helper_bodies = self.always_helper_bodies(debug);
        let bodies: Vec<&str> = self
            .functions
            .iter()
            .flat_map(|f| f.implementations.iter())
            .filter_map(|imp| match &imp.body {
                Body::Mfb { body, .. } => Some(body.trim_end()),
                _ => None,
            })
            .collect();
        if self.records.is_empty()
            && self.unions.is_empty()
            && self.enums.is_empty()
            && bodies.is_empty()
            && helper_bodies.is_empty()
        {
            return String::new();
        }

        let mut pieces: Vec<String> = Vec::with_capacity(
            1 + self.records.len()
                + self.unions.len()
                + self.enums.len()
                + helper_bodies.len()
                + bodies.len(),
        );
        if !self.imports.is_empty() {
            let imports = self
                .imports
                .iter()
                .map(|name| format!("IMPORT {name}"))
                .collect::<Vec<_>>()
                .join("\n");
            pieces.push(imports);
        }
        pieces.extend(self.records.iter().map(RegistryRecord::render));
        pieces.extend(self.unions.iter().map(RegistryUnion::render));
        pieces.extend(self.enums.iter().map(RegistryEnum::render));
        pieces.extend(
            helper_bodies
                .iter()
                .map(|helper| helper.trim_end().to_string()),
        );
        pieces.extend(bodies.iter().map(|body| body.to_string()));

        let mut out = pieces.join("\n\n");
        out.push('\n');
        out
    }

    /// Add packages to this package's import list. Additive — later calls append —
    /// so a package may accumulate its imports across several calls. They render into
    /// [`get_mfb`](Self::get_mfb) as leading `IMPORT <name>` lines, before everything
    /// else, in the order added.
    pub(crate) fn add_imports(&mut self, imports: Vec<&'static str>) -> &mut Self {
        self.imports.extend(imports);
        self
    }

    /// Add a gated helper chunk/edge (plan-99). Additive — later calls append. An
    /// [`Always`](HelperGate::Always) body renders into [`get_mfb`](Self::get_mfb)
    /// between the unions and the member bodies (the old `add_helper_functions`
    /// position); a gated body (`WhenUsed`/`WhenImported`) and any `import_name`
    /// ordering edge are consumed by [`Registry::augment_project`].
    pub(crate) fn add_helper(&mut self, helper: RegistryHelper) -> &mut Self {
        self.helpers.push(helper);
        self
    }

    /// Record the package's **source-declared value-type** names (see the
    /// [`source_types`](Self::source_types) field). Additive; the names are semantic-only
    /// (recognized by [`is_builtin_type`] / [`qualified_builtin_type`]) and are NOT
    /// rendered by [`get_mfb`](Self::get_mfb) — the companion source already declares them.
    pub(crate) fn add_source_types(&mut self, names: &[&'static str]) -> &mut Self {
        self.source_types.extend_from_slice(names);
        self
    }

    /// Add a value record (a `RegistryRecord { … }`). Records render into
    /// [`get_mfb`](Self::get_mfb) in the order they are added, before the functions.
    pub(crate) fn add_record(&mut self, record: RegistryRecord) -> &mut Self {
        debug_assert!(
            !record.props.is_empty(),
            "record `{}` needs at least one field",
            record.name,
        );
        self.records.push(record);
        self
    }

    /// Add a tagged union (a `RegistryUnion { … }`). Unions render into
    /// [`get_mfb`](Self::get_mfb) in the order they are added, between the records and
    /// the functions.
    pub(crate) fn add_union(&mut self, union: RegistryUnion) -> &mut Self {
        debug_assert!(
            !union.variants.is_empty(),
            "union `{}` needs at least one variant",
            union.name,
        );
        self.unions.push(union);
        self
    }

    /// Add a value enum (a `RegistryEnum { … }`). Enums render into
    /// [`get_mfb`](Self::get_mfb) in the order they are added, between the unions and
    /// the helper functions.
    pub(crate) fn add_enum(&mut self, r#enum: RegistryEnum) -> &mut Self {
        debug_assert!(
            !r#enum.variants.is_empty(),
            "enum `{}` needs at least one variant",
            r#enum.name,
        );
        self.enums.push(r#enum);
        self
    }

    /// Add an opaque resource type (a `RegistryResource { … }`). Resources carry no
    /// injectable source, so they are recorded as semantic facts only and are not
    /// rendered by [`get_mfb`](Self::get_mfb).
    pub(crate) fn add_resource(&mut self, resource: RegistryResource) -> &mut Self {
        self.resources.push(resource);
        self
    }

    /// Declare that `function`'s `parameter` **consumes the resources reachable from its
    /// argument** — see [`consuming_params`](Self::consuming_params).
    ///
    /// The call must name a real member and a real parameter of it; nothing else in the
    /// compiler would notice a typo, because a lookup that finds nothing reads exactly
    /// like "this parameter does not consume". `the_consuming_parameters_name_real_members`
    /// is what makes a typo fail, and it is a real test rather than a `debug_assert!`:
    /// CI builds `--release` on all five platforms, so a `debug_assert!` here would run
    /// nowhere in it.
    pub(crate) fn add_consuming_parameter(
        &mut self,
        function: &'static str,
        parameter: &'static str,
    ) -> &mut Self {
        self.consuming_params.push(ConsumingParameter {
            function,
            parameter,
        });
        self
    }

    /// The package's consuming parameters, in registration order.
    pub(crate) fn consuming_params(&self) -> &[ConsumingParameter] {
        &self.consuming_params
    }

    /// Add a function (a `RegistryFunction { … }`).
    pub(crate) fn add_function(&mut self, function: RegistryFunction) -> &mut Self {
        debug_assert!(
            !function.implementations.is_empty(),
            "function `{}` needs at least one implementation",
            function.name,
        );
        self.functions.push(function);
        self
    }

    /// The package's compile-time constants, in registration order.
    pub(crate) fn constants(&self) -> &[RegistryConstant] {
        &self.constants
    }

    /// Add a compile-time package constant (a `RegistryConstant { … }`). Scalar
    /// constants set `value`; record constants set `components`.
    pub(crate) fn add_constant(&mut self, constant: RegistryConstant) -> &mut Self {
        debug_assert!(
            constant.value.is_some() != constant.components.is_some(),
            "constant `{}` must set exactly one of `value` (scalar) / `components` (record)",
            constant.name,
        );
        self.constants.push(constant);
        self
    }

    /// The package's general-builtin overrides, in registration order.
    pub(crate) fn overrides(&self) -> &[RegistryOverride] {
        &self.overrides
    }

    /// Add a general-builtin override (a `RegistryOverride { … }`).
    pub(crate) fn add_override(&mut self, r#override: RegistryOverride) -> &mut Self {
        self.overrides.push(r#override);
        self
    }
}
