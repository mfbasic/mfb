use super::helpers::*;
use super::*;
use crate::types::ParameterType;

impl<'a> SyntaxChecker<'a> {
    /// The type a source/imported type SPELLING denotes.
    ///
    /// plan-106-C: the grammar is [`ParameterType::parse`] — syntaxcheck no
    /// longer carries a private copy of it. What remains here is only the work
    /// the canonical parser deliberately does not do, established by reading the
    /// old parser's five non-grammar steps (recorded in the plan):
    ///
    /// 1. the top-level ` STATE T` peel — `Type` has no STATE concept, it carries
    ///    the clause *beside* the type in `LocalInfo`/`ParamSig`, so a
    ///    `fs::File STATE Cursor` IS a `File` for every purpose `Type` serves
    ///    (plan-52-D §4);
    /// 2. a qualified builtin **resource** keeps its qualified identity (plan-97);
    /// 3. a qualified builtin **value** type collapses to its bare internal id
    ///    (plan-03-http §A.1/§B.2);
    /// 4. the thread plane's `STATE` splits into its own slot (plan-54);
    /// 5. nothing — the old tail consulted `user_types` and discarded the answer.
    ///
    /// Steps 1–4 are applied by [`type_from_parameter`](Self::type_from_parameter)
    /// at every level, exactly as the recursive private parser applied them.
    pub(super) fn parse_type(&self, name: &str) -> Type {
        self.normalize_type(&ParameterType::parse(name))
    }

    /// The same normalization applied to a type HIR already carries.
    ///
    /// plan-106-D: every type syntaxcheck reads off an HIR node arrives as a
    /// `ParameterType`, so it needs syntaxcheck's three non-grammar steps and
    /// nothing else. `parse_type` above is now just this, preceded by the parse
    /// — and it survives only for the handful of spellings HIR still stores as
    /// strings (`UNION` variants, `LINK` signatures).
    pub(super) fn normalize_type(&self, type_: &Type) -> Type {
        self.normalize(type_, false)
    }

    /// syntaxcheck's three normalizations, applied at every level — the only
    /// work left over once [`ParameterType::parse`] owns the grammar
    /// (plan-106-C; rung 2e turned the old `type_from_parameter` conversion into
    /// this, because `Type` IS `ParameterType` now and there is nothing to
    /// convert):
    ///
    /// 1. **Peel a top-level ` STATE T`.** syntaxcheck carries a resource's
    ///    state BESIDE the type (`LocalInfo::state_type`), so a
    ///    `fs::File STATE Cursor` IS a `File` for every purpose it serves — the
    ///    fix plan-52-D §4 made, without which `fs::close(h)` on an imported
    ///    stateful handle reported "expected File, got fs::File STATE Cursor".
    ///    The ONE exception is a thread handle's resource plane (`in_thread_res`),
    ///    where plan-54 wants the clause kept so `transfer`/`accept` can check the
    ///    state that crosses the boundary.
    /// 2. **A qualified builtin RESOURCE keeps its qualified identity** —
    ///    resources are package-scoped, so a user `TYPE File` no longer collides
    ///    (plan-97).
    /// 3. **A qualified builtin VALUE type collapses to its bare internal id**
    ///    (plan-03-http §A.1/§B.2).
    fn normalize(&self, type_: &Type, in_thread_res: bool) -> Type {
        match type_ {
            Type::ListOf(element) => Type::list_of(self.normalize(element, false)),
            Type::SetOf(element) => Type::set_of(self.normalize(element, false)),
            Type::ResultOf(success) => Type::result_of(self.normalize(success, false)),
            Type::Res(inner) => Type::Res(Box::new(self.normalize(inner, in_thread_res))),
            Type::MapOf(key, value) => {
                Type::map_of(self.normalize(key, false), self.normalize(value, false))
            }
            Type::MapEntryOf(key, value) => {
                Type::map_entry_of(self.normalize(key, false), self.normalize(value, false))
            }
            Type::Func(params, return_type, isolated) => Type::Func(
                params.iter().map(|p| self.normalize(p, false)).collect(),
                Box::new(self.normalize(return_type, false)),
                *isolated,
            ),
            Type::ThreadHandle {
                worker,
                msg,
                res,
                out,
            } => Type::ThreadHandle {
                worker: *worker,
                msg: Box::new(self.normalize(msg, false)),
                // The plane keeps its ` STATE T`: this is the exception in step 1.
                res: Box::new(self.normalize(res, true)),
                out: Box::new(self.normalize(out, false)),
            },
            Type::Named(name) => self.normalize_leaf(name.resolve(), in_thread_res),
            // plan-106-D: a type variable collapses back to the bare nominal.
            //
            // `hir::elaborate` classifies a name appearing in the enclosing
            // declaration's `template_params` as a [`Type::Var`] (`with_vars`).
            // syntaxcheck's own parser never produced that variant — a generic
            // parameter `T` was a `Named("T")` here — and every rule below is
            // written against the nominal. Without this collapse, the injected
            // `collections` package source stops type-checking against itself: its
            // generic members' parameters become `Var` while their call sites carry
            // nominals, and every candidate is rejected as
            // `TYPE_CALL_ARGUMENT_MISMATCH`.
            //
            // The classification is not lost — it lives in the HIR that monomorph
            // reads. syntaxcheck simply predates it.
            Type::Var(name) => self.normalize_leaf(name.resolve(), in_thread_res),
            // Scalars, `Unknown`, and the variants syntaxcheck's own parser never
            // produces (`Arg`, `UserOf`, `AttributeString`) normalize to
            // themselves.
            other => other.clone(),
        }
    }

    /// Steps 1-3 of [`normalize`](Self::normalize) at a nominal leaf.
    fn normalize_leaf(&self, name: &str, keep_state: bool) -> Type {
        // Step 1 (or its exception): split the clause off, normalize the base,
        // and re-attach only inside a thread plane.
        let (base, state) = match crate::codegen::resource::state_type_name(name) {
            Some(state) => (
                crate::codegen::resource::base_resource_name(name),
                Some(state),
            ),
            None => (name, None),
        };
        let normalized = self.normalize_bare(base);
        match state {
            Some(state) if keep_state => normalized.with_state(&Type::named(state)),
            _ => normalized,
        }
    }

    /// Steps 2-3 plus the two spellings that are not nominals at all.
    fn normalize_bare(&self, name: &str) -> Type {
        // Step 2: a package-qualified built-in RESOURCE keeps its qualified identity.
        if builtins::is_qualified_builtin_resource(name) {
            return Type::named(name);
        }
        // Step 3: a package-qualified built-in VALUE type resolves to its bare id.
        if let Some(bare) = builtins::qualified_builtin_type(name) {
            return Type::named(&bare);
        }
        // A spelling that LOOKS like a function type but did not parse as one is
        // malformed (`FUNC(Integer` — no `) AS ` return clause). The old private
        // parser answered `Type::Unknown` for it, and that is the right answer:
        // `Unknown` is syntaxcheck's permissive skip, whereas treating it as a
        // nominal would make it match nothing and reject the program on a
        // *parse* failure. Pinned by `parse_function_type_malformed_yields_unknown`.
        if name.starts_with("FUNC(") || name.starts_with("ISOLATED FUNC(") {
            return Type::Unknown;
        }
        // `Result` with no ` OF ` is the bare marker; the canonical parser leaves
        // it a nominal, and it means `Result OF Unknown` here.
        if name == "Result" {
            return Type::result_of(Type::Unknown);
        }
        Type::named(name)
    }

    pub(super) fn compatible(&self, expected: &Type, actual: &Type) -> bool {
        if matches!(expected, Type::Unknown) || matches!(actual, Type::Unknown) {
            return true;
        }
        // The `RES` element marker is an ownership-axis annotation (§15.6), not a
        // distinct value type: a `File` value fits a `RES fs::File` slot and vice
        // versa. Strip it before comparing.
        let (expected, actual) = (strip_res(expected), strip_res(actual));
        match (expected, actual) {
            (Type::ListOf(expected), Type::ListOf(actual)) => self.compatible(expected, actual),
            (Type::SetOf(expected), Type::SetOf(actual)) => self.compatible(expected, actual),
            (Type::MapOf(expected_key, expected_value), Type::MapOf(actual_key, actual_value)) => {
                self.compatible(expected_key, actual_key)
                    && self.compatible(expected_value, actual_value)
            }
            (Type::ResultOf(expected), Type::ResultOf(actual)) => self.compatible(expected, actual),
            // A parent `Thread` handle and a `ThreadWorker` handle never unify
            // with each other, which the `worker` flags equality preserves (they
            // were two separate variant pairs before plan-106-C rung 2c).
            (
                Type::ThreadHandle {
                    worker: expected_worker,
                    msg: expected_message,
                    res: expected_resource,
                    out: expected_output,
                },
                Type::ThreadHandle {
                    worker: actual_worker,
                    msg: actual_message,
                    res: actual_resource,
                    out: actual_output,
                },
            ) => {
                // plan-106-C rung 2e: the resource plane and its ` STATE T` share
                // one slot now, so ONE comparison decides both axes — an absent
                // plane is `Nothing`, so the old `compatible_optional` pair
                // (`None`/`Some` plus state) falls out of ordinary compatibility.
                expected_worker == actual_worker
                    && self.compatible(expected_message, actual_message)
                    && self.compatible(expected_resource, actual_resource)
                    && self.compatible(expected_output, actual_output)
            }
            (
                Type::Func(expected_params, expected_return, expected_isolated),
                Type::Func(actual_params, actual_return, actual_isolated),
            ) => {
                (!expected_isolated || *actual_isolated)
                    && expected_params.len() == actual_params.len()
                    && expected_params
                        .iter()
                        .zip(actual_params.iter())
                        // Function parameters are contravariant: a value promised
                        // to accept any `expected` param must be an actual that
                        // accepts at least as wide a type, so compare the actual's
                        // declared param against the expected one (bug-173 A).
                        .all(|(expected, actual)| self.compatible(actual, expected))
                    && self.compatible(expected_return, actual_return)
            }
            (Type::Named(expected_name), Type::Named(actual_name)) => {
                let (expected_name, actual_name) = (expected_name.resolve(), actual_name.resolve());
                if expected_name == actual_name {
                    return true;
                }
                let expected_bare = expected_name.rsplit('.').next().unwrap_or(expected_name);
                let actual_bare = actual_name.rsplit('.').next().unwrap_or(actual_name);
                let expected_info = self
                    .type_infos
                    .get(expected_name)
                    .or_else(|| self.type_infos.get(expected_bare));
                // A union accepts any of its variant values (a variant value fits
                // its union slot).
                if expected_info.is_some_and(|info| {
                    matches!(info.kind, TypeDeclKind::Union)
                        && info.variants.iter().any(|variant| {
                            // A variant may be spelled qualified (a package-scoped
                            // resource, `fs.File`) or bare — match either form.
                            variant.name == *actual_name || variant.name == *actual_bare
                        })
                }) {
                    return true;
                }
                if expected_bare != actual_bare {
                    return false;
                }
                // The bare names coincide. An imported package's types are
                // registered under their bare name (`Db`), while a qualified
                // reference written by the importer resolves to `binding.Db`
                // (plan-link-update.md §5a) — so a qualified name must equate to
                // its bare form. But two genuinely distinct declarations that
                // merely share a final path segment (an imported `geo.Point` and a
                // local `Point` with different fields) must NOT unify (bug-41):
                // only unify when both names resolve to the *same* registered
                // `TypeInfo`. When either side is unregistered — a built-in `User`
                // type such as `net.Url`, or a template parameter — the shared bare
                // name is authoritative.
                let actual_info = self
                    .type_infos
                    .get(actual_name)
                    .or_else(|| self.type_infos.get(actual_bare));
                match (expected_info, actual_info) {
                    (Some(expected_info), Some(actual_info)) => {
                        std::ptr::eq(expected_info, actual_info)
                    }
                    _ => true,
                }
            }
            _ => expected == actual,
        }
    }

    pub(super) fn is_numeric(&self, type_: &Type) -> bool {
        matches!(
            type_,
            Type::Byte | Type::Fixed | Type::Float | Type::Integer | Type::Money | Type::Unknown
        )
    }

    pub(super) fn is_comparable(&self, type_: &Type) -> bool {
        self.is_comparable_with_seen(type_, &mut HashSet::new())
    }

    /// An operand acceptable on either side of a `String` ordering comparison
    /// (`<`, `>`, `<=`, `>=`). `Unknown` is permitted so a prior error does not
    /// cascade. Numeric operands are handled separately by `is_numeric`.
    pub(super) fn is_orderable_string(&self, type_: &Type) -> bool {
        matches!(type_, Type::String | Type::Unknown)
    }

    /// An operand acceptable on either side of a `Scalar` ordering comparison
    /// (`<`, `>`, `<=`, `>=`). `Scalar` orders by codepoint value and is
    /// non-numeric — it does not order against `String` or any numeric type.
    /// `Unknown` is permitted so a prior error does not cascade.
    pub(super) fn is_orderable_scalar(&self, type_: &Type) -> bool {
        matches!(type_, Type::Unknown) || type_.is_named("Scalar")
    }

    pub(super) fn is_comparable_with_seen(&self, type_: &Type, seen: &mut HashSet<String>) -> bool {
        match type_ {
            Type::Boolean
            | Type::Byte
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Money
            | Type::Nothing
            | Type::String
            | Type::Unknown => true,
            // `Error`/`ErrorLoc`/`Scalar` are comparable.
            Type::Named(name) if is_comparable_builtin_nominal(name.resolve()) => true,
            // `AttributedString` is NOT: it wraps a list overlay (like `List`),
            // so it is never a `Map` key or `Set` element (plan-89-A). It needs
            // its own arm — the general `User` arm below answers `true` for any
            // name it cannot resolve, so merely leaving it out would flip the
            // verdict (caught by `attributed_string_not_comparable`).
            Type::Named(name) if name.resolve() == "AttributedString" => false,
            Type::ListOf(_)
            | Type::SetOf(_)
            | Type::MapOf(_, _)
            | Type::Func(..)
            | Type::ResultOf(_)
            | Type::Res(_)
            | Type::ThreadHandle { .. } => false,
            Type::Named(name) => {
                let name = name.resolve();
                if self.resource_registry.is_resource(name) || !seen.insert(name.to_string()) {
                    return false;
                }
                let Some(info) = self.type_infos.get(name) else {
                    return true;
                };
                let result = match info.kind {
                    TypeDeclKind::Enum => true,
                    TypeDeclKind::Type => info
                        .fields
                        .iter()
                        .all(|field| self.is_comparable_with_seen(&field.type_, seen)),
                    TypeDeclKind::Union => false,
                };
                seen.remove(name);
                result
            }
            // `ParameterType` carries variants syntaxcheck's own parser never
            // produces (`Var`, `Arg`, `UserOf`, `MapEntryOf`, `AttributeString`);
            // a decoded package signature can still hold one. Before plan-106-C
            // rung 2e each arrived spelled out as `Type::User(<spelling>)` and so
            // took the NOMINAL arm above — routing the render back through it
            // reproduces that exactly, rather than guessing a new answer for a
            // shape this checker has never had to answer for.
            other => self.is_comparable_with_seen(&Type::named(&other.name()), seen),
        }
    }

    pub(super) fn require_comparable_type(
        &mut self,
        _file: &HirFile,
        _line: usize,
        _context: &str,
        _type_: &Type,
    ) {
        // Comparability is now enforced by `ir::verify` (the sole rejecter for both
        // the source and package paths, plan-20). This relocated syntaxcheck rule
        // emits no diagnostic; the body is intentionally empty.
    }

    /// The argument mode for argument `index` of a call to `callee`. A call to a
    /// resource's *registered close op* consumes its single resource argument
    /// (overhaul invalidation event #1) — for native LINK resources this is the
    /// `LINK` CLOSE wrapper (plan-link-update.md §6). All other resource arguments
    /// do not move ownership by default.
    pub(super) fn call_argument_mode(
        &self,
        callee: &str,
        index: usize,
        sig: &FunctionSig,
    ) -> ExprMode {
        let param_type = sig.params.get(index).map(|param| &param.type_);
        if index == 0 {
            if let Some(Type::Named(name)) = param_type {
                let name = name.resolve();
                let base = crate::codegen::resource::base_resource_name(name);
                let is_close_op = self.resource_registry.close_function(base) == Some(callee)
                    || self.resource_registry.close_function(name) == Some(callee)
                    // A re-export alias of the close op consumes too (§5a).
                    || self
                        .close_op_aliases
                        .get(callee)
                        .is_some_and(|type_name| type_name == base || type_name == name);
                if is_close_op {
                    return ExprMode::Transfer;
                }
            }
        }
        self.argument_mode_for_type(&param_type)
    }

    pub(super) fn argument_mode_for_type(&self, expected: &Option<&Type>) -> ExprMode {
        match expected {
            // An ordinary call does not move a resource's ownership: it uses the handle for
            // the duration of the call but does not take ownership. Only the
            // fixed invalidation events (a registered close op, `thread::transfer`,
            // `RETURN`, and scope-drop) end a resource's life.
            Some(type_) if self.is_resource_type(type_) => ExprMode::Use,
            Some(type_) if !self.is_copyable_type(type_) => ExprMode::Transfer,
            _ => ExprMode::Read,
        }
    }

    pub(super) fn thread_argument_mode(&self, callee: &str, index: usize) -> ExprMode {
        match (callee, index) {
            // `thread.transfer` is resource-plane invalidation event #2: the
            // resource moves to the worker, so the sender binding is consumed.
            ("thread.start", 1) | ("thread.send", 1) | ("thread.transfer", 1) => ExprMode::Transfer,
            ("thread.start", _) | ("thread.send", _) | ("thread.transfer", _) => ExprMode::Use,
            _ => ExprMode::Use,
        }
    }

    /// Argument evaluation mode for a builtin collection op, keyed on the BARE op
    /// name. Callers pass the dequalified member (`append`, not
    /// `collections.append`); this is only ever reached for recognised builtin
    /// calls, so a freed bare name from user code never gets here
    /// (plan-01-functions.md §5).
    pub(super) fn general_argument_mode(&self, callee: &str, index: usize) -> ExprMode {
        if matches!(
            callee,
            "len"
                | "get"
                | "getOr"
                | "find"
                | "keys"
                | "values"
                | "hasKey"
                | "contains"
                | "forEach"
                | "transform"
                | "filter"
                | "reduce"
                | "sum"
        ) {
            return ExprMode::Read;
        }
        if matches!(
            callee,
            "removeAt" | "removeKey" | "replace" | "set" | "append" | "prepend" | "insert"
        ) {
            return if index == 0 {
                ExprMode::Transfer
            } else {
                ExprMode::Read
            };
        }
        ExprMode::Read
    }
}

/// Whether a `Map`/`MapEntry`/`Thread`/`ThreadWorker` `OF`-construct — each of
/// which owns exactly one top-level ` TO ` — begins at byte `at` of `body`. The
/// match must sit on a word boundary so a user template whose name merely ends
/// in `Map` (`MyMap OF T`, which owns no ` TO `) is not counted.

#[cfg(test)]
mod types_tests {
    use crate::syntaxcheck::testutil::*;

    // ---- parse_type arms exercised through type annotations ----------------

    #[test]
    fn scalar_type_annotations_accept() {
        // Boolean, Byte, Fixed, Float, Integer, String, Nothing.
        assert!(accepts(
            "FUNC main AS Integer\n  LET a AS Boolean = TRUE\n  LET b AS Byte = toByte(1)\n  LET c AS Fixed = toFixed(\"1.5\")\n  LET d AS Float = 1.0\n  LET e AS Integer = 1\n  LET f AS String = \"x\"\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn list_map_result_annotations_accept() {
        assert!(accepts(
            "FUNC main AS Integer\n  LET xs AS List OF Integer = [1]\n  LET m AS Map OF String TO Integer = Map OF String TO Integer {}\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn function_type_annotation_accepts() {
        // FUNC(...) AS ... and ISOLATED FUNC(...) AS ... parse arms.
        assert!(accepts(
            "FUNC apply(f AS FUNC(Integer) AS Integer, x AS Integer) AS Integer\n  RETURN f(x)\nEND FUNC\nFUNC dbl(n AS Integer) AS Integer\n  RETURN n * 2\nEND FUNC\nFUNC main AS Integer\n  RETURN apply(dbl, 3)\nEND FUNC\n"
        ));
    }

    #[test]
    fn nested_function_type_empty_params() {
        // FUNC() AS Integer — empty parameter list arm.
        assert!(accepts(
            "FUNC run(f AS FUNC() AS Integer) AS Integer\n  RETURN f()\nEND FUNC\nFUNC zero AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN run(zero)\nEND FUNC\n"
        ));
    }

    #[test]
    fn thread_type_annotation_accepts() {
        // Thread OF ... TO ... parse arm (message/output).
        let src = "IMPORT thread\nFUNC main AS Integer\n  LET t AS Thread OF Integer TO Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn user_type_annotation_accepts() {
        assert!(accepts(
            "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\nFUNC main AS Integer\n  LET p AS Point = Point[1, 2]\n  RETURN 0\nEND FUNC\n"
        ));
    }

    // ---- compatible / expression_compatible --------------------------------

    #[test]
    fn byte_literal_fits_byte() {
        // (Byte, Integer, Number) special case in expression_compatible.
        assert!(accepts(
            "FUNC main AS Integer\n  LET b AS Byte = 200\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn byte_literal_overflow_walks_false_branch() {
        // The `<= u8::MAX` guard's false arm runs here even though the actual
        // rejection for an out-of-range Byte is relocated to ir::verify.
        let _ = check_src("FUNC main AS Integer\n  LET b AS Byte = 300\n  RETURN 0\nEND FUNC\n");
    }

    // ---- bug-41 (3): radix/separator Byte-literal RECOVER range check -------

    fn byte_recover_src(literal: &str) -> String {
        // An inline-TRAP RECOVER against a `Byte` success type is the surviving
        // consumer of `expression_compatible`'s Byte arm (checking.rs:320).
        format!(
            "FUNC parseByte(v AS Integer) AS Byte\n  IF v < 0 THEN FAIL error(404, \"neg\")\n  RETURN toByte(v)\nEND FUNC\nFUNC main AS Integer\n  LET b AS Byte = parseByte(-1) TRAP(e)\n    RECOVER {literal}\n  END TRAP\n  RETURN 0\nEND FUNC\n"
        )
    }

    #[test]
    fn byte_recover_accepts_radix_and_separator_literals() {
        // The lexer canonicalizes radix/separator literals to decimal before the
        // Byte range check (`0xFF`->`255`, `2_00`->`200`), so an in-range Byte is
        // accepted — not spuriously rejected with TYPE_RECOVER_TYPE_MISMATCH
        // (bug-41 (3)). Decimal `200` is the pre-existing baseline.
        for literal in ["200", "0xFF", "0b1111_1111", "2_00"] {
            assert!(
                !check_src(&byte_recover_src(literal))
                    .iter()
                    .any(|rule| rule == "TYPE_RECOVER_TYPE_MISMATCH"),
                "RECOVER {literal} against a Byte type should be accepted"
            );
        }
    }

    #[test]
    fn byte_recover_rejects_out_of_range_radix_literal() {
        // `0x100` == 256 is out of Byte range and must still be rejected.
        assert!(check_src(&byte_recover_src("0x100"))
            .iter()
            .any(|rule| rule == "TYPE_RECOVER_TYPE_MISMATCH"));
    }

    #[test]
    fn fixed_from_integer_literal() {
        // (Fixed, Integer|Float, Number) arm.
        assert!(accepts(
            "FUNC main AS Integer\n  LET f AS Fixed = 5\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn fixed_from_negative_literal() {
        // (Fixed, Integer|Float, Unary '-') arm.
        assert!(accepts(
            "FUNC main AS Integer\n  LET f AS Fixed = -5\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn list_literal_element_compat() {
        // (List, List, ListLiteral) numeric-widening arm.
        assert!(accepts(
            "FUNC main AS Integer\n  LET xs AS List OF Fixed = [1, 2, 3]\n  RETURN 0\nEND FUNC\n"
        ));
    }

    // ---- comparability (list element in contains/find) ---------------------

    #[test]
    fn contains_on_list_of_record_is_walked() {
        // Exercises is_comparable_with_seen over a user Type record.
        let src = "IMPORT collections\nTYPE P\n  x AS Integer\nEND TYPE\nFUNC main AS Integer\n  LET xs AS List OF P = [P[1]]\n  LET b = collections::contains(xs, P[1])\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn enum_comparable() {
        let src = "IMPORT collections\nENUM Color\n  Red\n  Green\nEND ENUM\nFUNC main AS Integer\n  LET xs AS List OF Color = [Color.Red]\n  LET b = collections::contains(xs, Color.Green)\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- string ordering comparison (is_orderable_string) ------------------

    #[test]
    fn string_ordering_comparison_accepts() {
        assert!(accepts(
            "FUNC main AS Boolean\n  RETURN \"a\" < \"b\"\nEND FUNC\n"
        ));
    }

    // ---- RES-marked collection element (Type::Res) --------------------------

    #[test]
    fn res_marked_list_element_parses() {
        let src = "IMPORT fs\nFUNC take(xs AS List OF RES fs::File) AS Integer\n  RETURN len(xs)\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn res_marked_map_value_parses() {
        let src = "IMPORT fs\nFUNC take(m AS Map OF String TO RES fs::File) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- qualified builtin type reference (net.Url) ------------------------

    #[test]
    fn qualified_builtin_type_annotation() {
        let src = "IMPORT net\nFUNC main AS Integer\n  LET u AS net::Url = net::toUrl(\"http://x/\")\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- isolated function type annotation ---------------------------------

    #[test]
    fn isolated_function_type_annotation() {
        let src = "FUNC run(f AS ISOLATED FUNC(Integer) AS Integer) AS Integer\n  RETURN f(1)\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- compatible over Result / Thread / nested Map ----------------------

    #[test]
    fn result_and_thread_compatibility_walk() {
        // A worker whose message type is a nested Map exercises the Map arm of
        // compatible, and returning a Result-typed value walks Result compat.
        let src = "IMPORT thread\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF (Map OF String TO Integer) TO Integer, seed AS Map OF String TO Integer) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- union-variant compatibility (a variant fits its union) ------------

    #[test]
    fn union_variant_fits_union() {
        // Assigning a variant value to a union-typed binding walks the
        // User/User union-variant arm of compatible.
        let src = "TYPE A\n  x AS Integer\nEND TYPE\nTYPE B\n  y AS Integer\nEND TYPE\nUNION AB\n  A\n  B\nEND UNION\nFUNC pick AS AB\n  RETURN A[1]\nEND FUNC\nFUNC main AS Integer\n  LET v AS AB = pick()\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- non-comparable list element (union) rejected in contains ----------

    #[test]
    fn contains_on_union_list_walks_noncomparable() {
        let src = "IMPORT collections\nTYPE A\n  x AS Integer\nEND TYPE\nTYPE B\n  y AS Integer\nEND TYPE\nUNION AB\n  A\n  B\nEND UNION\nFUNC main AS Integer\n  LET xs AS List OF AB = [A[1]]\n  LET b = collections::contains(xs, A[1])\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- close-op argument mode (call_argument_mode Transfer arm) ----------

    #[test]
    fn close_op_consumes_resource() {
        // fs::close is the registered close op for File; calling it consumes the
        // handle (call_argument_mode Transfer arm).
        assert!(accepts(
            "IMPORT fs\nFUNC main AS Integer\n  RES f AS fs::File = fs::openFile(\"x\")\n  fs::close(f)\n  RETURN 0\nEND FUNC\n"
        ));
    }

    // ---- expression_compatible via default parameter values ----------------

    #[test]
    fn default_byte_from_int_literal() {
        // Byte param with an in-range Integer-literal default (Byte/Integer/Number).
        assert!(accepts(
            "FUNC g(a AS Byte = 200) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN g()\nEND FUNC\n"
        ));
    }

    #[test]
    fn default_fixed_from_int_literal() {
        assert!(accepts(
            "FUNC g(a AS Fixed = 5) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN g()\nEND FUNC\n"
        ));
    }

    #[test]
    fn default_fixed_from_negative_literal() {
        // Fixed param with a negated numeric literal default (Unary '-' arm).
        assert!(accepts(
            "FUNC g(a AS Fixed = -5) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN g()\nEND FUNC\n"
        ));
    }

    #[test]
    fn default_list_of_fixed_literal() {
        // List-literal default numeric-widening arm.
        assert!(accepts(
            "FUNC g(a AS List OF Fixed = [1, 2]) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN g()\nEND FUNC\n"
        ));
    }

    // ---- compatible_optional (thread resource plane both present) ----------

    #[test]
    fn thread_resource_plane_optional_compat() {
        // A worker whose declared and inferred thread types both carry a resource
        // plane exercises compatible_optional Some/Some.
        assert!(check_project_dir(std::path::Path::new(&format!(
            "{}/tests/rt-behavior/threads/func_thread_transfer_valid",
            env!("CARGO_MANIFEST_DIR")
        )))
        .is_empty());
    }

    // ---- bug-41 (2): nesting-aware `Map OF K TO V` split -------------------

    #[test]
    fn split_map_body_handles_nested_key_and_value() {
        // plan-106-C deleted `split_map_body`, which had become a bare delegate;
        // these assertions follow the splitter to its one home.
        use crate::types::split_top_level_to as split_map_body;
        // Simple body: leftmost and balanced agree.
        assert_eq!(
            split_map_body("String TO Integer"),
            Some(("String", "Integer"))
        );
        // Nested KEY carries its own ` TO `: the key is the whole inner map, the
        // value is `Boolean` (bug-41 — leftmost split gave `Map OF Map OF String`).
        assert_eq!(
            split_map_body("Map OF String TO Integer TO Boolean"),
            Some(("Map OF String TO Integer", "Boolean"))
        );
        // Nested VALUE map (already correct under leftmost split) still parses.
        assert_eq!(
            split_map_body("String TO Map OF Integer TO Boolean"),
            Some(("String", "Map OF Integer TO Boolean"))
        );
        // A `FUNC(...) AS R` key is kept whole (parens/`AS` carry no top-level TO).
        assert_eq!(
            split_map_body("FUNC(Integer) AS Boolean TO Integer"),
            Some(("FUNC(Integer) AS Boolean", "Integer"))
        );
        // A parenthesized nested-map key round-trips (the caller strips the group).
        assert_eq!(
            split_map_body("(Map OF String TO Integer) TO Boolean"),
            Some(("(Map OF String TO Integer)", "Boolean"))
        );
        // A RES-marked value stays attached (the caller's element parser strips it).
        assert_eq!(
            split_map_body("String TO RES fs::File"),
            Some(("String", "RES fs::File"))
        );
        // No top-level ` TO ` at all → None (caller falls through to a type name).
        assert_eq!(split_map_body("Integer"), None);
    }

    #[test]
    fn parse_type_nested_map_key_structure() {
        use super::{SyntaxChecker, Type};
        let dir = std::path::Path::new(".");
        let project = crate::hir::HirProject {
            name: "t".to_string(),
            files: vec![],
        };
        let checker = SyntaxChecker::new(dir, &project);
        // `Map OF Map OF String TO Integer TO Boolean` must build
        // `Map(Map(String, Integer), Boolean)`, not the mis-split
        // `Map(User("Map OF Map OF String"), …)`.
        let Type::MapOf(key, value) =
            checker.parse_type("Map OF Map OF String TO Integer TO Boolean")
        else {
            panic!("expected a Map type");
        };
        assert!(matches!(*value, Type::Boolean));
        let Type::MapOf(inner_key, inner_value) = *key else {
            panic!("expected the key to be a nested Map");
        };
        assert!(matches!(*inner_key, Type::String));
        assert!(matches!(*inner_value, Type::Integer));
    }

    // ---- bug-41 (1): bare-name User unification needs same declaration -----

    #[test]
    fn bare_name_user_types_need_same_declaration() {
        use super::{FieldInfo, SyntaxChecker, Type, TypeInfo};
        use crate::ast::{TypeDeclKind, Visibility};
        let dir = std::path::Path::new(".");
        let project = crate::hir::HirProject {
            name: "t".to_string(),
            files: vec![],
        };
        let mut checker = SyntaxChecker::new(dir, &project);
        let record = |field: &str| TypeInfo {
            kind: TypeDeclKind::Type,
            visibility: Visibility::Export,
            file_path: String::new(),
            fields: vec![FieldInfo {
                name: field.to_string(),
                type_: Type::Integer,
                visibility: Visibility::Public,
            }],
            variants: Vec::new(),
            members: std::collections::HashSet::new(),
        };
        // Two genuinely distinct declarations that share the final segment `Point`.
        checker
            .type_infos
            .insert("geo.Point".to_string(), record("lat"));
        checker.type_infos.insert("Point".to_string(), record("x"));
        // bug-41: distinct declarations must NOT unify on the shared bare name.
        assert!(!checker.compatible(&Type::named("geo.Point"), &Type::named("Point")));
        // The legitimate qualified==bare case (both resolve to the same registered
        // `TypeInfo`) still unifies: a qualified alias of the bare `Point`.
        assert!(checker.compatible(&Type::named("mod.Point"), &Type::named("Point")));
    }

    // ---- parse_type / compatible direct unit tests -------------------------

    fn empty_project() -> crate::hir::HirProject {
        crate::hir::HirProject {
            name: "t".to_string(),
            files: vec![],
        }
    }

    #[test]
    fn parse_type_bare_result_and_unknown() {
        use super::{SyntaxChecker, Type};
        let project = empty_project();
        let checker = SyntaxChecker::new(std::path::Path::new("."), &project);
        assert!(matches!(checker.parse_type("Result"), Type::ResultOf(_)));
        assert!(matches!(checker.parse_type("Unknown"), Type::Unknown));
    }

    #[test]
    fn parse_type_qualified_builtin_resolves_to_bare_user() {
        use super::{SyntaxChecker, Type};
        let project = empty_project();
        let checker = SyntaxChecker::new(std::path::Path::new("."), &project);
        // `net.Url` is a package-qualified built-in type id (plan-03-http §A.1).
        assert!(matches!(checker.parse_type("net.Url"), Type::Named(_)));
    }

    #[test]
    fn parse_function_type_malformed_yields_unknown() {
        use super::{SyntaxChecker, Type};
        let project = empty_project();
        let checker = SyntaxChecker::new(std::path::Path::new("."), &project);
        // A `FUNC(...` with no `) AS ` return clause cannot split — Unknown.
        assert!(matches!(checker.parse_type("FUNC(Integer"), Type::Unknown));
    }

    #[test]
    fn compatible_result_threadworker_thread_function_arms() {
        use super::{SyntaxChecker, Type};
        let project = empty_project();
        let checker = SyntaxChecker::new(std::path::Path::new("."), &project);
        let int = || Box::new(Type::Integer);
        // Result vs Result.
        assert!(checker.compatible(&Type::ResultOf(int()), &Type::ResultOf(int())));
        // ThreadWorker vs ThreadWorker, with and without a resource plane.
        // plan-106-C rung 2e: an absent resource plane is `Nothing`, not `None`,
        // and one `compatible` call decides the plane (the old `compatible_optional`
        // pair is gone with it).
        let tw = |res: Type| Type::ThreadHandle {
            worker: true,
            msg: int(),
            res: Box::new(res),
            out: int(),
        };
        assert!(checker.compatible(&tw(Type::Nothing), &tw(Type::Nothing)));
        assert!(checker.compatible(&tw(Type::String), &tw(Type::String)));
        // One side carries a resource plane, the other does not.
        assert!(!checker.compatible(&tw(Type::String), &tw(Type::Nothing)));
        // Thread vs Thread resource-plane mismatch.
        let th = |res: Type| Type::ThreadHandle {
            worker: false,
            msg: int(),
            res: Box::new(res),
            out: int(),
        };
        assert!(!checker.compatible(&th(Type::String), &th(Type::Nothing)));
        // A parent handle and a worker handle never unify with each other.
        assert!(!checker.compatible(&th(Type::Nothing), &tw(Type::Nothing)));
        // Function: a non-isolated function fits a non-isolated slot, and an
        // isolated function fits an isolated slot, but not vice versa.
        let func = |iso: bool| Type::Func(vec![Type::Integer], int(), iso);
        assert!(checker.compatible(&func(false), &func(true)));
        assert!(!checker.compatible(&func(true), &func(false)));
    }

    #[test]
    fn compatible_distinct_bare_user_names_reject() {
        use super::{SyntaxChecker, Type};
        let project = empty_project();
        let checker = SyntaxChecker::new(std::path::Path::new("."), &project);
        // Different bare names never unify.
        assert!(!checker.compatible(&Type::named("Point"), &Type::named("Circle")));
    }

    // ---- is_comparable User arms (via `=` on enum / record / union) --------

    #[test]
    fn enum_equality_comparability() {
        let src = "ENUM Color\n  Red, Green\nEND ENUM\nFUNC main AS Integer\n  LET b AS Boolean = Color.Red = Color.Green\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn record_equality_comparability() {
        let src = "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\nFUNC main AS Integer\n  LET p AS Point = Point[1, 2]\n  LET q AS Point = Point[1, 2]\n  LET b AS Boolean = p = q\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn union_equality_not_comparable() {
        let src = "TYPE Dot\n  x AS Integer\nEND TYPE\nTYPE Line\n  a AS Integer\nEND TYPE\nUNION Shape\n  Dot\n  Line\nEND UNION\nFUNC eq(a AS Shape, b AS Shape) AS Boolean\n  RETURN a = b\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- expression_compatible literal-coercion arms -----------------------

    #[test]
    fn fixed_from_negative_integer_literal_accepted() {
        assert!(accepts(
            "FUNC main AS Integer\n  LET f AS Fixed = -3\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn list_of_byte_from_integer_literals_accepted() {
        assert!(accepts(
            "FUNC main AS Integer\n  LET xs AS List OF Byte = [1, 2, 3]\n  RETURN 0\nEND FUNC\n"
        ));
    }

    // ---- plan-89-A: AttributedString opacity + value semantics -------------

    #[test]
    fn attributed_string_annotation_accepts() {
        // The type parses as a first-class annotation and a `MUT` binding with no
        // initializer is accepted (defaulting is enforced in ir::verify).
        assert!(accepts(
            "FUNC use(a AS AttributedString) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  MUT a AS AttributedString\n  RETURN use(a)\nEND FUNC\n"
        ));
    }

    #[test]
    fn attributed_string_record_literal_rejected() {
        // Opaque: no record-literal construction; use astrings::fromString.
        assert!(rejects_with(
            "FUNC main AS Integer\n  LET a AS AttributedString = AttributedString[\"hi\"]\n  RETURN 0\nEND FUNC\n",
            "TYPE_READ_ONLY_RECORD_CONSTRUCTOR"
        ));
    }

    #[test]
    fn attributed_string_field_read_rejected() {
        // Opaque: no user-visible fields — a `.text` read cannot be typed.
        assert!(rejects_with(
            "FUNC main AS Integer\n  MUT a AS AttributedString\n  LET t AS String = a.text\n  RETURN 0\nEND FUNC\n",
            "TYPE_UNKNOWN_VALUE"
        ));
    }

    #[test]
    fn attributed_string_not_comparable() {
        // Wraps a list overlay (like `List`): not comparable, so `=` cannot type.
        assert!(rejects_with(
            "FUNC cmp(a AS AttributedString, b AS AttributedString) AS Boolean\n  RETURN a = b\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
            "TYPE_UNKNOWN_VALUE"
        ));
    }
}
