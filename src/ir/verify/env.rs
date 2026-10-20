//! plan-176-N: building the verifier's `TypeEnv` and its core queries — record fields, union
//! variants, closure arity and expression type inference. The `TypeEnv` struct lives in
//! `ir/verify/mod.rs`.

use super::*;

/// Rules whose failure leaves the failing expression's type undeterminable in
/// the former source checker (its `infer_*` returns `Unknown` after reporting them).
const POISONING_RULES: &[&str] = &[
    "TYPE_BINARY_OPERATOR_MISMATCH",
    "TYPE_UNARY_OPERATOR_MISMATCH",
    "TYPE_UNARY_OPERATOR_UNKNOWN",
    "TYPE_REQUIRES_COMPARABLE",
    "TYPE_CALL_ARGUMENT_MISMATCH",
    "TYPE_CALL_ARITY_MISMATCH",
    "TYPE_CONSTRUCTOR_REQUIRES_RECORD",
    "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
    "TYPE_READ_ONLY_RECORD_UPDATE",
    "TYPE_FIELD_ACCESS_REQUIRES_RECORD",
    "TYPE_UNKNOWN_FIELD",
];

impl TypeEnv {
    // ===========================================================================
    // 1. Construction, diagnostic emission, closure-capture arity
    // ===========================================================================

    pub(super) fn build(project: &IrProject) -> Self {
        let mut records = HashMap::new();
        let mut unions = HashMap::new();
        // plan-111-B: these nine tables are keyed BY THE TYPE, not by its
        // spelling. A declared type is a nominal, so the key is
        // `ParameterType::named(<decl name>)` — the same interned `Symbol` a
        // `Named` arriving from `parse` carries, so a lookup with a type in hand
        // needs no rendering at all.
        let mut enums: HashMap<ParameterType, HashSet<String>> = HashMap::new();
        let mut field_types: HashMap<ParameterType, HashMap<String, ParameterType>> =
            HashMap::new();
        let mut record_field_lists: HashMap<ParameterType, Vec<(String, ParameterType)>> =
            HashMap::new();
        let mut private_fields: HashMap<ParameterType, HashSet<String>> = HashMap::new();
        let mut record_decls: HashSet<ParameterType> = HashSet::new();
        let type_decl_info: HashMap<ParameterType, (String, String)> = project
            .types
            .iter()
            .map(|t| {
                (
                    ParameterType::declared(&t.name),
                    (t.file.clone(), t.visibility.clone()),
                )
            })
            .collect();
        for ty in &project.types {
            match ty.kind.as_str() {
                "enum" => {
                    enums.insert(
                        ParameterType::declared(&ty.name),
                        ty.members.iter().map(|m| m.name.clone()).collect(),
                    );
                }
                "type" | "record" => {
                    let record_type = ParameterType::declared(&ty.name);
                    record_decls.insert(record_type.clone());
                    records.insert(
                        record_type,
                        RecordInfo {
                            fields: ty.fields.iter().map(|f| f.name.clone()).collect(),
                            includes: ty.includes.clone(),
                        },
                    );
                    field_types.insert(
                        ParameterType::declared(&ty.name),
                        field_type_map(&ty.fields),
                    );
                    record_field_lists.insert(
                        ParameterType::declared(&ty.name),
                        ty.fields
                            .iter()
                            .map(|f| (f.name.clone(), f.type_.clone()))
                            .collect(),
                    );
                    let private: HashSet<String> = ty
                        .fields
                        .iter()
                        .filter(|f| f.visibility.as_deref() == Some("private"))
                        .map(|f| f.name.clone())
                        .collect();
                    if !private.is_empty() {
                        private_fields.insert(ParameterType::declared(&ty.name), private);
                    }
                }
                "union" => {
                    unions.insert(
                        ParameterType::declared(&ty.name),
                        UnionInfo {
                            variants: ty
                                .variants
                                .iter()
                                .map(|v| ParameterType::declared(&v.name))
                                .collect(),
                            variant_order: ty.variants.iter().map(|v| v.name.clone()).collect(),
                            includes: ty.includes.clone(),
                        },
                    );
                    // Each variant is a record type in its own right; register
                    // its payload fields so `variant.field` accesses resolve.
                    for variant in &ty.variants {
                        records
                            .entry(ParameterType::declared(&variant.name))
                            .or_insert_with(|| RecordInfo {
                                fields: variant.fields.iter().map(|f| f.name.clone()).collect(),
                                includes: Vec::new(),
                            });
                        field_types
                            .entry(ParameterType::declared(&variant.name))
                            .or_insert_with(|| field_type_map(&variant.fields));
                        record_field_lists
                            .entry(ParameterType::declared(&variant.name))
                            .or_insert_with(|| {
                                variant
                                    .fields
                                    .iter()
                                    .map(|f| (f.name.clone(), f.type_.clone()))
                                    .collect()
                            });
                    }
                }
                _ => {}
            }
        }

        let mut functions = HashMap::new();
        for function in &project.functions {
            functions.insert(
                function.name.clone(),
                FnSig {
                    total: function.params.len(),
                    optional: function
                        .params
                        .iter()
                        .filter(|p| p.default.is_some())
                        .count(),
                    params: function.params.iter().map(|p| p.type_.clone()).collect(),
                    returns: function.returns.clone(),
                },
            );
        }

        let globals = project
            .bindings
            .iter()
            .map(|b| (b.name.clone(), b.type_.clone()))
            .collect();
        let global_muts = project
            .bindings
            .iter()
            .map(|b| (b.name.clone(), b.mutable))
            .collect();
        let resource_closers = project
            .native_resources
            .iter()
            .map(|r| (ParameterType::declared(&r.name), r.close_function.clone()))
            .collect();
        let resource_sendable = project
            .native_resources
            .iter()
            .map(|r| (ParameterType::declared(&r.name), r.sendable))
            .collect();

        let mut closure_counts: HashMap<String, HashSet<usize>> = HashMap::new();
        for function in &project.functions {
            for param in &function.params {
                if let Some(default) = &param.default {
                    collect_closures(default, &mut closure_counts);
                }
            }
            collect_closures_ops(&function.body, &mut closure_counts);
        }
        for binding in &project.bindings {
            if let Some(value) = &binding.value {
                collect_closures(value, &mut closure_counts);
            }
        }

        TypeEnv {
            records,
            unions,
            functions,
            declared_functions: project.functions.iter().map(|f| f.name.clone()).collect(),
            total_functions: trap_init::total_functions(&project.functions, &globals),
            globals,
            global_muts,
            resource_closers,
            resource_sendable,
            link_spans: crate::ir::LinkSpans::default(),
            closure_counts,
            field_types,
            record_field_lists,
            record_decls,
            enums,
            diags: RefCell::new(Vec::new()),
            current_line: Cell::new(0),
            trap_closed: RefCell::new(HashSet::new()),
            back_edge_scan: RefCell::new(None),
            transfer_failure_flags: RefCell::new(HashMap::new()),
            current_file: RefCell::new(String::new()),
            current_sites: RefCell::new(Vec::new()),
            current_return: RefCell::new(ParameterType::Unknown),
            current_function: RefCell::new(String::new()),
            current_muts: RefCell::new(HashMap::new()),
            poisoned: Cell::new(false),
            source_path: Cell::new(false),
            // Strict by default: the package path (and every unit test) builds the
            // env directly and has the full merged type table. Only
            // `collect_source_diagnostics` opts into the leniency.
            imported_types_unknown: false,
            checking_state_assign: Cell::new(false),
            loop_stack: RefCell::new(Vec::new()),
            sound_hash_selector: Cell::new(false),
            current_owners: RefCell::new(HashSet::new()),
            current_opaque_params: RefCell::new(HashSet::new()),
            type_decl_info,
            private_fields,
        }
    }

    /// Record one diagnostic at the current line/file.
    pub(super) fn emit(&self, rule: &str, detail: String) {
        if POISONING_RULES.contains(&rule) {
            self.poisoned.set(true);
        }
        let sites = self.current_sites.borrow();
        if !sites.is_empty() {
            let mut diags = self.diags.borrow_mut();
            for site in sites.iter() {
                diags.push(Diagnostic {
                    rule: rule.to_string(),
                    detail: site.describe(&detail),
                    file: site.file.clone(),
                    line: site.line as u32,
                });
            }
            return;
        }
        drop(sites);
        self.diags.borrow_mut().push(Diagnostic {
            rule: rule.to_string(),
            detail,
            file: self.current_file.borrow().clone(),
            line: self.current_line.get(),
        });
    }

    /// The captured-slot bound for a closure-body function, or `None` when the
    /// function is never used as a closure body.
    ///
    /// Ambiguity must not disarm the capture-bounds check: returning `None` when a
    /// body was seen with two different capture-vector lengths let a crafted
    /// package pair `Closure{name:"$l", captures:[a]}` with
    /// `Closure{name:"$l", captures:[a,b]}` and then read `Capture{index:9999}`
    /// out of the environment. Bound against the *smallest* observed count — the
    /// only slot count every call site is guaranteed to have — so the check still
    /// runs. `check_closure_capture_arity` rejects the ambiguous shape itself.
    pub(super) fn closure_slot_count(&self, function: &str) -> Option<usize> {
        self.closure_counts.get(function)?.iter().min().copied()
    }

    /// Reject a closure-body function reached by capture vectors of differing
    /// length. Lowering emits one `Closure` node per body function, so differing
    /// arities cannot arise from source: it is a structural signal of a tampered
    /// package, and it is what disarmed the capture-bounds check above.
    pub(super) fn check_closure_capture_arity(&self, function: &str) {
        let Some(counts) = self.closure_counts.get(function) else {
            return;
        };
        if counts.len() < 2 {
            return;
        }
        let mut arities = counts.iter().copied().collect::<Vec<_>>();
        arities.sort_unstable();
        let arities = arities
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        self.emit(
            VERIFY_TYPE,
            format!(
                "closure body `{function}` is captured with differing capture counts ({arities})"
            ),
        );
    }

    // ===========================================================================
    // 13. Type-model lookup helpers (record_fields, union_variants, infer_type)
    // ===========================================================================

    /// The complete set of field names for a record type, expanding `includes`
    /// transitively. Returns `None` when the type is not a known record or when
    /// an include cannot be resolved (so the field set is incomplete and the
    /// member-existence check must be skipped).
    pub(super) fn record_fields(&self, type_: &ParameterType) -> Option<HashSet<String>> {
        // The read-only compiler/runtime records `Error`/`ErrorLoc` carry their
        // fields in a local table rather than the project type table.
        if let Some(fields) = builtin_type_fields(type_) {
            return Some(fields.iter().map(|(name, _)| (*name).to_string()).collect());
        }
        let mut out = HashSet::new();
        let mut seen = HashSet::new();
        if self.collect_record_fields(type_, &mut out, &mut seen) {
            Some(out)
        } else {
            None
        }
    }

    pub(super) fn collect_record_fields(
        &self,
        type_: &ParameterType,
        out: &mut HashSet<String>,
        seen: &mut HashSet<ParameterType>,
    ) -> bool {
        if !seen.insert(type_.clone()) {
            // A cycle in `includes` — treat as fully expanded to avoid looping.
            return true;
        }
        let Some(info) = self.records.get(type_) else {
            return false;
        };
        for field in &info.fields {
            out.insert(field.clone());
        }
        // An `includes` entry is a declared type NAME, so it names a nominal.
        for include in &info.includes {
            if !self.collect_record_fields(&ParameterType::declared(include), out, seen) {
                return false;
            }
        }
        true
    }

    /// The complete variant-name set of a union, expanding included unions.
    /// `None` when the union or one of its includes is unknown.
    pub(super) fn union_variants(&self, union_type: &ParameterType) -> Option<HashSet<String>> {
        let mut out = HashSet::new();
        let mut seen = HashSet::new();
        if self.collect_union_variants(union_type, &mut out, &mut seen) {
            Some(out)
        } else {
            None
        }
    }

    pub(super) fn collect_union_variants(
        &self,
        union_type: &ParameterType,
        out: &mut HashSet<String>,
        seen: &mut HashSet<ParameterType>,
    ) -> bool {
        if !seen.insert(union_type.clone()) {
            return true;
        }
        let Some(info) = self.unions.get(union_type) else {
            return false;
        };
        for variant in &info.variants {
            out.insert(variant.name().into_owned());
        }
        for include in &info.includes {
            if !self.collect_union_variants(&ParameterType::declared(include), out, seen) {
                return false;
            }
        }
        true
    }

    /// Best-effort static type of a value. Returns `None` whenever the type
    /// cannot be determined with certainty; callers treat `None` as "unknown"
    /// and skip type-dependent rejections.
    ///
    /// Since format v3 (plan-20-B) every computed node carries its result type,
    /// so this resolves `Call`/`CallResult`/`Binary`/`Unary`/`ResultValue`/… as
    /// well — a member access on a *computed* primitive result is now caught,
    /// not just one on a local or constructor. `Local`/`Global` resolve through
    /// the binding environment (their type is not on the node); the `"Unknown"`
    /// marker a node carries when lowering could not name its type is treated as
    /// unresolved so it never forces a rejection (plan-20-C).
    pub(super) fn infer_type(
        &self,
        value: &IrValue,
        locals: &HashMap<String, ParameterType>,
    ) -> Option<ParameterType> {
        self.infer_type_depth(value, locals, 0)
    }

    /// Depth-bounded body of `infer_type`. Member-access chains recurse on
    /// expression depth, so — mirroring `check_ops`' cap — the recursion is
    /// bounded to `MAX_DEPTH` levels; past that it fails gracefully by leaving
    /// the type underived (`None`), which the type-relational rules treat
    /// permissively.
    pub(super) fn infer_type_depth(
        &self,
        value: &IrValue,
        locals: &HashMap<String, ParameterType>,
        depth: usize,
    ) -> Option<ParameterType> {
        if depth > MAX_DEPTH {
            return None;
        }
        match value {
            IrValue::Local(name) => return locals.get(name).cloned(),
            IrValue::Global(name) => return self.globals.get(name).cloned(),
            IrValue::MemberAccess { target, member, .. } => {
                // Prefer the annotated member type; fall back to resolving the
                // field through the target's record type for older shapes.
                if let Some(annotated) = usable_type(value.annotated_parameter_type()) {
                    return Some(annotated);
                }
                let target_type = self.infer_type_depth(target, locals, depth + 1)?;
                return self.field_type(&target_type, member);
            }
            _ => {}
        }
        usable_type(value.annotated_parameter_type())
    }

    /// The declared type of a record member, for chained member-access
    /// inference. Only resolves through record types whose fields are known.
    pub(super) fn field_type(&self, type_: &ParameterType, member: &str) -> Option<ParameterType> {
        if let Some(fields) = builtin_type_fields(type_) {
            return fields
                .iter()
                .find(|(name, _)| *name == member)
                .map(|(_, type_)| type_.clone());
        }
        // Project records store field types on the IrType; look them up via the
        // dedicated map built alongside `records`. plan-111-B: keyed by the
        // type, so nothing renders here any more.
        //
        // plan-114-E: the result is the field type with its top-level `RES `
        // marker stripped, leaving `Stateful { base, state }` (or the bare
        // resource when the field carries no `STATE`). The VALUE is unchanged —
        // reading the field yields the same handle pointer either way; only the
        // type spelling differs.
        //
        // This is the front-end twin of the same strip in
        // `lower_field_access` (`memory/value/builder_value_semantics.rs`), and
        // BOTH are needed. `split_state` matches `Stateful` only at the top
        // level, so `Res(Stateful{..}).state()` is `None`: without the strip
        // here, `h.handle.state` is refused by the type checker before codegen
        // ever sees it — with `TYPE_STATE_INVALID` claiming "`fs.File` here has
        // no STATE", which names the wrong problem entirely.
        //
        // Unconditional, as for a collection element
        // (`list_element("List OF RES Socket") == "Socket"`): one rule for both
        // positions keeps them from drifting.
        self.field_types
            .get(type_)
            .and_then(|fields| fields.get(member).cloned())
            .map(|field_type| strip_res(&field_type).clone())
    }
}
