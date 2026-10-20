//! plan-176-M: resolving type names — leaves, package-qualified names and unknown-type
//! reports. Item resolution lives in `resolver/resolution/mod.rs`.

use super::*;

impl Resolver<'_> {
    /// Resolve a type spelling that HIR still stores as a `String`.
    ///
    /// Resolve every nominal reachable from `type_`, structurally.
    ///
    /// This replaces `resolve_type_name(&str)`, which was 120 lines of grammar
    /// re-implementation — `strip_type_group`, `starts_with("Result OF ")`,
    /// `strip_prefix("FUNC(")` + `split_func_params_and_return`,
    /// `thread_parts_full`, `state_type_name`/`base_resource_name`, and four
    /// `ParameterType::parse` arms that immediately re-rendered each child with
    /// `.name()` so the recursion could go back through a string. Every one of
    /// those decisions is a variant here.
    ///
    /// The leaf tail is still a NAME lookup, and that is not a residue of the old
    /// design: a nominal's identity *is* its name, and `self.types` is a symbol
    /// table. What is gone is deriving a type's SHAPE from its spelling.
    pub(super) fn resolve_type(
        &mut self,
        file: &HirFile,
        type_: &ParameterType,
        line: usize,
        imports: &HashMap<String, String>,
    ) {
        match type_ {
            // plan-114-D: a `RES`-marked type in its OWN right — a record field,
            // `handle AS RES fs::File`. The marker rides on the field and names a
            // type that must still be resolved, exactly as a collection element's
            // does below.
            //
            // Before this arm, a `Res` reaching the top level fell through to the
            // leaf tail, which sees a name containing `.` and calls
            // `resolve_package_qualified_name` on the WHOLE spelling — reporting
            // `SYMBOL_UNKNOWN_IMPORT` for a package named `RES fs`. The same
            // failure shape as bug-463, and the same cause: a `ParameterType`
            // variant that a structural walk does not have an arm for is silent
            // until some spelling reaches it.
            ParameterType::Res(inner) => {
                self.resolve_type(file, inner, line, imports);
            }
            // A `List OF RES fs::File` element carries the resource ownership-axis
            // marker (§15.6); the marker rides on the element and names a type that
            // must still be resolved.
            ParameterType::ListOf(element) => {
                self.resolve_type(file, peel_res(element), line, imports);
            }
            // `Set OF T` (plan-63): a single element type, no `RES`, no `TO`. A
            // `RES`-marked element in this position is not peeled — it reaches the
            // leaf tail and is reported, exactly as the string cascade left it.
            // The internal `#Result OF T` (bug-945: no user spelling reaches it)
            // resolves its success type like any single-child shape.
            ParameterType::SetOf(element) | ParameterType::ResultOf(element) => {
                self.resolve_type(file, element, line, imports);
            }
            ParameterType::MapOf(key, value) => {
                self.resolve_type(file, key, line, imports);
                self.resolve_type(file, peel_res(value), line, imports);
            }
            ParameterType::MapEntryOf(key, value) => {
                self.resolve_type(file, key, line, imports);
                self.resolve_type(file, value, line, imports);
            }
            ParameterType::Func(params, returns, _isolated) => {
                for param in params {
                    self.resolve_type(file, param, line, imports);
                }
                self.resolve_type(file, returns, line, imports);
            }
            // A parametric thread type's three planes. An absent resource plane is
            // `Nothing`, whose arm below is a no-op — so this resolves exactly the
            // planes the old `thread_parts_full` handed back.
            ParameterType::ThreadHandle { msg, res, out, .. } => {
                self.resolve_type(file, msg, line, imports);
                self.resolve_type(file, res, line, imports);
                self.resolve_type(file, out, line, imports);
            }
            // A user generic (`Stack OF T`, `Pair OF A, B`). An unknown head is
            // deliberately NOT consumed: it falls to the unknown-type report, which
            // names the whole spelling.
            ParameterType::UserOf(head, args) => {
                let head = head.resolve();
                if self.types.contains(&ParameterType::declared(head))
                    || self.active_template_params.contains(head)
                {
                    for arg in args {
                        self.resolve_type(file, arg, line, imports);
                    }
                } else {
                    self.report_unknown_type(file, &type_.name(), line);
                }
            }
            // The absent annotation.
            ParameterType::Unknown => {}
            // A type variable in a generic signature. `hir::elaborate` classified it
            // from the enclosing declaration's `template_params`, which is the same
            // set `active_template_params` holds — so the membership check agrees
            // with the classification, and a stray variable is still reported.
            ParameterType::Var(name) => {
                let name = name.resolve();
                if !self.active_template_params.contains(name) {
                    self.report_unknown_type(file, name, line);
                }
            }
            // A leaf: a scalar, a nominal, or a `RES`-marked type in a position the
            // grammar does not allow one. All three resolve by NAME, which is what
            // the string cascade's tail did after every structural peel.
            _ => self.resolve_leaf(file, type_, line, imports),
        }
    }

    /// The leaf tail: a `STATE` clause, then the nominal itself.
    fn resolve_leaf(
        &mut self,
        file: &HirFile,
        type_: &ParameterType,
        line: usize,
        imports: &HashMap<String, String>,
    ) {
        // A resource type carrying `STATE T` (plan-52/54): resolve the base resource
        // type and the STATE payload independently — `fs::File STATE Cursor` is not
        // one symbol. Reached only after the compound arms above have peeled any
        // enclosing `Thread`/`List`/`Map` (whose own spellings also contain
        // ` STATE `), so this fires on the bare resource element: a stateful
        // binding/return, or a thread plane's `RES` element (plan-54).
        //
        // `split_state` is TOP-LEVEL only, which is what keeps a composite like
        // `List OF RES Handle STATE Cursor` splitting at its element rather than at
        // the container (bug-429).
        let (base, state) = type_.split_state();
        if let Some(state) = state {
            self.resolve_type(file, &base, line, imports);
            self.resolve_type(file, &state, line, imports);
            return;
        }

        // plan-111-B: `Unknown` is a nominal here — the `ParameterType::Unknown`
        // variant has its own arm above, so a spelling reaching this leaf tail
        // is a decoded-IR `Named("Unknown")`. The template-parameter membership
        // is a NAME set (the declaring scope's parameter list), so it renders.
        let name = type_.name();
        let name = name.as_ref();
        if type_.is_named("Unknown") || self.active_template_params.contains(name) {
            return;
        }
        if let Some((root, _)) = name.split_once('.') {
            // bug-632: an imported user package's type, already canonicalized to
            // `<package>.<Name>` by the parser (`IMPORT ov AS o` / `o::A` arrives
            // as `ov.A`). It resolves only where this file imports that package.
            if imports.values().any(|package| package == root) && self.types.contains(type_) {
                return;
            }
            self.resolve_package_qualified_name(file, name, line, imports);
        } else if !self.types.contains(&ParameterType::declared(name)) {
            self.report_unknown_type(file, name, line);
        }
    }

    fn report_unknown_type(&mut self, file: &HirFile, name: &str, line: usize) {
        self.report(
            "SYMBOL_UNKNOWN_TYPE",
            &format!("Type `{name}` is not a built-in or top-level project type."),
            file,
            line,
        );
    }

    pub(super) fn resolve_package_qualified_name(
        &mut self,
        file: &HirFile,
        name: &str,
        line: usize,
        imports: &HashMap<String, String>,
    ) {
        let root = name.split('.').next().unwrap_or(name);
        // bug-480 Phase 4b: a built-in package's injected companion refers to its
        // OWN package-qualified types (`http.Response` inside `builtins/http.mfb`).
        // That is a self-reference, not an import — the file IS the package — so it
        // neither needs nor can have an `IMPORT http` line. Same rule `IMPORT self`
        // encodes for a source package; without it every companion that names one
        // of its own value types reports SYMBOL_UNKNOWN_IMPORT against a file the
        // developer never wrote.
        if file
            .path
            .strip_prefix("builtins/")
            .and_then(|rest| rest.strip_suffix(".mfb"))
            == Some(root)
        {
            return;
        }
        // A LINK alias is a package-local namespace, not an import: resolve its
        // members against the block's native functions (plan-link-update.md §5b).
        if let Some(link) = self.link_functions.get(root) {
            let member = name.split_once('.').map(|(_, rest)| rest).unwrap_or("");
            if !link.contains_key(member) {
                self.report(
                    "SYMBOL_UNKNOWN_IDENTIFIER",
                    &format!("LINK `{root}` does not declare a native function `{member}`."),
                    file,
                    line,
                );
            }
            return;
        }
        let Some(package) = imports.get(root) else {
            self.report(
                "SYMBOL_UNKNOWN_IMPORT",
                &format!("Package `{root}` is used but not imported in this file."),
                file,
                line,
            );
            return;
        };

        if builtins::is_builtin_import(package) {
            let qualified_name = qualify_package_name(name, root, package);
            // A package-qualified built-in type (`net::Url`, `http::Result`)
            // resolves to its bare internal id (plan-03-http.md §A.1/§B.2).
            if builtins::qualified_builtin_type(&qualified_name).is_some() {
                return;
            }
            // An internal-only member resolves for the injected package glue that
            // needs it and is invisible to user source, which is what makes it
            // internal (bug-337-D9).
            let internal_only = builtins::is_internal_only_call(&qualified_name) && !file.internal;
            if internal_only || !builtins::is_builtin_member(&qualified_name) {
                self.report(
                    "SYMBOL_UNKNOWN_IDENTIFIER",
                    &format!("Built-in package `{package}` does not export `{qualified_name}`."),
                    file,
                    line,
                );
            }
            return;
        }

        // bug-480: the same gate for an imported USER package. Without it a
        // `pkg::member` naming nothing the package exports typed as `Unknown` and
        // was reported hundreds of lines away as `TYPE_CALL_ARGUMENT_MISMATCH ...
        // argument type(s) (Unknown)` against whatever consumed it, naming
        // neither the package nor the member.
        //
        // Only a package whose interface was READ contributes an entry, so an
        // unreadable or not-yet-installed dependency still resolves silently here
        // and is reported by the import gate that saw the real reason. `IMPORT
        // self` never records one either: a package's own members are ordinary
        // top-level declarations, resolved by the bare-name path.
        let Some(exports) = self.package_exports.get(package) else {
            return;
        };
        let Some((_, member)) = name.split_once('.') else {
            return;
        };
        // The rule governs the HEAD of a path, not field selection: `pkg::rec.field`
        // resolves `rec` against the package and leaves `.field` to the type
        // checker.
        let head = member.split('.').next().unwrap_or(member);
        if exports.contains(member) || exports.contains(head) {
            return;
        }
        self.report(
            "SYMBOL_UNKNOWN_IDENTIFIER",
            &format!("Package `{package}` does not export `{head}`."),
            file,
            line,
        );
    }
}

/// A collection element/value with its `RES` ownership-axis marker peeled off
/// (§15.6). The marker rides on the element, not the container, and wraps a type
/// that must still be resolved — `List OF RES fs::File` resolves `fs::File`.
///
/// plan-106-D: was `strip_res(&str) -> String`, which round-tripped the element
/// through `parse` and `name()` to drop one variant. Peeling the variant is the
/// whole operation.
pub(super) fn peel_res(type_: &ParameterType) -> &ParameterType {
    match type_ {
        ParameterType::Res(inner) => inner,
        other => other,
    }
}
