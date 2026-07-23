use super::*;

impl TypeEnv {
    // 8. Native LINK (cstructs + functions) + resource classification
    // ===========================================================================

    /// Validate the merged `CSTRUCT` table (plan-50-B §4.4) on the package path.
    ///
    /// Every rule the source path applies is applied again here, deliberately
    /// unlike `IrFree` (whose ctypes are dropped at lowering, so the package path
    /// checks strictly less than the frontend). A crafted `.mfp` drives raw C
    /// calls, so this is a marshaling-safety gate, not a convenience.
    ///
    /// Note what is **not** validated: offsets and sizes, because they are never
    /// transported. They are recomputed from the field ctypes here, so a crafted
    /// package has no offset to forge — it can only choose ctypes, each of which
    /// has a known size and alignment.
    pub(super) fn check_link_cstructs(&self, project: &IrProject) {
        self.current_file.replace(String::new());
        self.current_line.set(0);
        // The target is fixed for this build; every supported target is LP64 and
        // agrees on the table, so the choice cannot change a decoded layout.
        let target = "";
        for (index, cstruct) in project.link_cstructs.iter().enumerate() {
            let siblings: Vec<String> = project
                .link_cstructs
                .iter()
                .filter(|other| other.alias == cstruct.alias)
                .map(|other| other.name.clone())
                .collect();

            // A duplicate name within one alias would make slot resolution
            // ambiguous; source rejects it, so the package path must too.
            if project.link_cstructs[..index]
                .iter()
                .any(|prior| prior.alias == cstruct.alias && prior.name == cstruct.name)
            {
                self.emit(
                    "NATIVE_CSTRUCT_INVALID",
                    format!(
                        "LINK alias `{}` declares CSTRUCT `{}` more than once.",
                        cstruct.alias, cstruct.name
                    ),
                );
            }

            let fields: Vec<(String, String)> = cstruct
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.ctype.clone()))
                .collect();
            for fault in crate::ir::check_cstruct(&cstruct.name, &fields, &siblings, target) {
                self.emit(fault.rule, fault.message);
            }
        }

        // plan-50-E: every struct slot's record mapping, re-checked here. A crafted
        // `.mfp` never ran the frontend, so without this the package path would be
        // the weaker of the two — the `IrFree` mistake.
        for function in &project.link_functions {
            for slot in &function.abi_slots {
                let Some(decl) = project
                    .link_cstructs
                    .iter()
                    .find(|c| c.alias == function.alias && c.name == slot.ctype)
                else {
                    continue;
                };
                let Some(rec) = project
                    .types
                    .iter()
                    .find(|t| t.name == decl.maps_to && (t.kind == "type" || t.kind == "record"))
                else {
                    self.emit(
                        "NATIVE_STRUCT_FIELD_MISMATCH",
                        format!(
                            "CSTRUCT `{}` maps to `{}`, which is not a record type.",
                            decl.name, decl.maps_to
                        ),
                    );
                    continue;
                };
                let cfields: Vec<(String, String)> = decl
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), f.ctype.clone()))
                    .collect();
                let record: Vec<(String, String)> = rec
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), f.type_.clone()))
                    .collect();
                let view = crate::ir::StructSlotView {
                    cfields: &cfields,
                    record: &record,
                    cstruct_name: &decl.name,
                    maps_to: &decl.maps_to,
                };
                for fault in crate::ir::check_struct_slot(&view) {
                    self.emit(fault.rule, fault.message);
                }
            }
            // BIND IN must name a real slot and real fields.
            for bind in &function.bind_in {
                let Some(slot) = function.abi_slots.iter().find(|s| s.name == bind.slot) else {
                    self.emit(
                        "NATIVE_BIND_IN_INVALID",
                        format!(
                            "Native function `{}` BIND IN names ABI slot `{}`, which does not exist.",
                            function.name, bind.slot
                        ),
                    );
                    continue;
                };
                let Some(decl) = project
                    .link_cstructs
                    .iter()
                    .find(|c| c.alias == function.alias && c.name == slot.ctype)
                else {
                    self.emit(
                        "NATIVE_BIND_IN_INVALID",
                        format!(
                            "Native function `{}` BIND IN names slot `{}`, which is not a CSTRUCT.",
                            function.name, bind.slot
                        ),
                    );
                    continue;
                };
                for field in &bind.fields {
                    if !decl.fields.iter().any(|f| f.name == field.name) {
                        self.emit(
                            "NATIVE_BIND_IN_INVALID",
                            format!(
                                "Native function `{}` BIND IN sets `{}`, which CSTRUCT `{}` does not declare.",
                                function.name, field.name, decl.name
                            ),
                        );
                    }
                    // Exactly one of param/literal, or the thunk has nothing to write.
                    if field.param.is_some() == field.literal.is_some() {
                        self.emit(
                            "NATIVE_BIND_IN_INVALID",
                            format!(
                                "Native function `{}` BIND IN field `{}` must bind exactly one of a parameter or a literal.",
                                function.name, field.name
                            ),
                        );
                    }
                    if let Some(param) = &field.param {
                        if !function.params.iter().any(|(n, _)| n == param) {
                            self.emit(
                                "NATIVE_BIND_IN_INVALID",
                                format!(
                                    "Native function `{}` BIND IN field `{}` binds unknown parameter `{param}`.",
                                    function.name, field.name
                                ),
                            );
                        }
                    }
                }
            }
        }

        // The C name must not surface in a wrapper's MFBASIC-facing signature. On
        // the source path the resolver usually catches this first (a CSTRUCT name
        // is not a type), but a decoded package never ran the resolver, so this is
        // the only thing standing between a crafted `.mfp` and a private C layout
        // in a public signature.
        for function in &project.link_functions {
            let names: Vec<&str> = project
                .link_cstructs
                .iter()
                .filter(|c| c.alias == function.alias)
                .map(|c| c.name.as_str())
                .collect();
            if names.is_empty() {
                continue;
            }
            for (pname, ptype) in &function.params {
                if names.contains(&ptype.as_str()) {
                    self.emit(
                        "NATIVE_CSTRUCT_ESCAPE",
                        format!(
                            "Native function `{}` parameter `{pname}` uses CSTRUCT `{ptype}`; only its mapped record type is nameable in a wrapper signature.",
                            function.name
                        ),
                    );
                }
            }
            if names.contains(&function.return_type.as_str()) {
                self.emit(
                    "NATIVE_CSTRUCT_ESCAPE",
                    format!(
                        "Native function `{}` returns CSTRUCT `{}`; only its mapped record type is nameable in a wrapper signature.",
                        function.name, function.return_type
                    ),
                );
            }
        }
    }

    /// Validate the merged LINK table (syntaxcheck's `check_link_function_in` on
    /// the IR): C ABI types may not escape into wrapper signatures, every ABI slot
    /// must bind to a parameter / CONST pin / the `return` result marker, every
    /// parameter and CONST pin must name a real slot, and a value-producing
    /// wrapper needs exactly one result marker. Package-path defense: a crafted
    /// .mfp's link table drives raw C calls, so these are marshaling-safety
    /// gates. (Spans are function-level here; syntaxcheck keeps the slot-level
    /// spans on the source path.)
    pub(super) fn check_link_functions(&self, project: &IrProject) {
        fn is_c_abi_type(t: &str) -> bool {
            matches!(
                t,
                "CPtr"
                    | "CString"
                    | "CInt8"
                    | "CInt16"
                    | "CInt32"
                    | "CInt64"
                    | "CUInt8"
                    | "CUInt16"
                    | "CUInt32"
                    | "CUInt64"
                    | "CFloat"
                    | "CDouble"
                    | "CVoid"
            )
        }
        self.current_file.replace(String::new());
        self.current_line.set(0);
        for function in &project.link_functions {
            for (pname, ptype) in &function.params {
                if is_c_abi_type(ptype) {
                    self.emit(
                        "NATIVE_CPTR_ESCAPE",
                        format!(
                            "Native function `{}` parameter `{pname}` uses C ABI type `{ptype}`; raw C types may appear only in ABI slots.",
                            function.name
                        ),
                    );
                }
            }
            if is_c_abi_type(&function.return_type) {
                self.emit(
                    "NATIVE_CPTR_ESCAPE",
                    format!(
                        "Native function `{}` returns C ABI type `{}`; raw C types may appear only in ABI slots.",
                        function.name, function.return_type
                    ),
                );
            }
            let const_slots: HashSet<&str> = function
                .consts
                .iter()
                .map(|(slot, _)| slot.as_str())
                .collect();
            let param_names: HashSet<&str> =
                function.params.iter().map(|(n, _)| n.as_str()).collect();
            // plan-50-A: the slot ctype namespace is closed. An unknown name used
            // to fall through to a raw 64-bit marshal (`link_thunk`'s default
            // arm), so a typo compiled clean and silently moved the wrong width.
            if !crate::ir::abi_ctype_valid_as_return(&function.abi_return_ctype) {
                self.emit(
                    "NATIVE_ABI_UNKNOWN_CTYPE",
                    format!(
                        "Native function `{}` ABI return `{}` uses C type `{}`, which is not a valid ABI return type.",
                        function.name, function.abi_return_name, function.abi_return_ctype
                    ),
                );
            }
            for slot in &function.abi_slots {
                // A slot may name a CSTRUCT declared in the same LINK alias; the
                // struct rules then apply instead of the scalar table (plan-50-E).
                if project
                    .link_cstructs
                    .iter()
                    .any(|c| c.alias == function.alias && c.name == slot.ctype)
                {
                    continue;
                }
                // An OUT slot is a produced *value*, so it carries a return-shaped
                // ctype; an ordinary slot is a C argument.
                let ok = if slot.direction.writes_back() {
                    crate::ir::abi_ctype_valid_as_return(&slot.ctype)
                } else {
                    crate::ir::abi_ctype_valid_as_argument(&slot.ctype)
                };
                if !ok {
                    self.emit(
                        "NATIVE_ABI_UNKNOWN_CTYPE",
                        format!(
                            "Native function `{}` ABI slot `{}` uses C type `{}`, which is not valid in that position.",
                            function.name, slot.name, slot.ctype
                        ),
                    );
                }
                if const_slots.contains(slot.name.as_str()) {
                    if slot.direction.writes_back() {
                        self.emit(
                            "NATIVE_CONST_OUT",
                            format!(
                                "Native function `{}` pins ABI slot `{}` with CONST, which cannot also be OUT.",
                                function.name, slot.name
                            ),
                        );
                    }
                    continue;
                }
                // An OUT slot is native storage the callee fills; it needs no
                // wrapper parameter. It is surfaced (if at all) by `RETURN`.
                if slot.direction.writes_back() {
                    continue;
                }
                // An IN struct slot is satisfied by its `BIND IN` block
                // (plan-50-E): its fields carry the inputs, unbound fields are 0.
                if function.bind_in.iter().any(|b| b.slot == slot.name) {
                    continue;
                }
                if !param_names.contains(slot.name.as_str()) {
                    self.emit(
                        "NATIVE_ABI_UNBOUND_SLOT",
                        format!(
                            "Native function `{}` ABI slot `{}` does not bind to a parameter, CONST pin, or an OUT buffer.",
                            function.name, slot.name
                        ),
                    );
                }
            }
            // plan-58-A: the `CBuffer` position rules, shared verbatim with
            // `syntaxcheck` so a crafted `.mfp` gets exactly the source-path
            // treatment. Spans are function-level here, as every other native-ABI
            // rule on this path is.
            {
                let size_reads: Vec<Vec<&str>> = function
                    .buffers
                    .iter()
                    .map(|b| {
                        let mut names = Vec::new();
                        crate::ir::link_expr_var_names(&b.size, &mut names);
                        names
                    })
                    .collect();
                let view = crate::ir::BufferSlotsView {
                    function: &function.name,
                    slots: function
                        .abi_slots
                        .iter()
                        .map(|s| (s.name.as_str(), s.ctype.as_str(), s.direction))
                        .collect(),
                    buffers: function
                        .buffers
                        .iter()
                        .zip(size_reads)
                        .map(|(b, reads)| (b.slot.as_str(), reads))
                        .collect(),
                    const_slots: function.consts.iter().map(|(s, _)| s.as_str()).collect(),
                    param_names: function.params.iter().map(|(n, _)| n.as_str()).collect(),
                    return_type: &function.return_type,
                    abi_return_name: &function.abi_return_name,
                    abi_return_ctype: &function.abi_return_ctype,
                    result_slot: match &function.result {
                        Some(crate::ir::IrLinkExpr::Var(name)) => Some(name.as_str()),
                        _ => None,
                    },
                    length_reads: function.result_length.as_ref().map(|expr| {
                        let mut names = Vec::new();
                        crate::ir::link_expr_var_names(expr, &mut names);
                        names
                    }),
                };
                for fault in crate::ir::check_buffer_slots(&view) {
                    self.emit(fault.rule, fault.message);
                }
            }
            // plan-50-H: the result is whatever `RETURN <expr>` names. Both
            // magic-name checks are gone.
            let wants_result = function.return_resource || function.return_type != "Nothing";
            if wants_result && function.result.is_none() {
                self.emit(
                    "NATIVE_ABI_NO_RESULT",
                    format!(
                        "Native function `{}` returns a value but declares no `RETURN <expr>` naming its result.",
                        function.name
                    ),
                );
            }
            if !wants_result && function.result.is_some() {
                self.emit(
                    "NATIVE_ABI_RESULT_MARKER",
                    format!(
                        "Native function `{}` returns Nothing but declares a `RETURN`.",
                        function.name
                    ),
                );
            }
            let abi_slot_names: HashSet<&str> = function
                .abi_slots
                .iter()
                .map(|slot| slot.name.as_str())
                .collect();
            for (pname, _) in &function.params {
                // plan-50-E: a parameter may instead be consumed by a `BIND IN`
                // field, which writes it into a struct slot and so has no slot of
                // its own.
                let by_bind = function.bind_in.iter().any(|b| {
                    b.fields
                        .iter()
                        .any(|f| f.param.as_deref() == Some(pname.as_str()))
                });
                // plan-58-B: likewise a parameter that only sizes an OUT CBuffer
                // (`BUFFER buf SIZE pairs * 2`) has no slot of its own, and is
                // consumed all the same.
                let by_buffer_size = function.buffers.iter().any(|b| {
                    let mut names = Vec::new();
                    crate::ir::link_expr_var_names(&b.size, &mut names);
                    names.contains(&pname.as_str())
                });
                if !abi_slot_names.contains(pname.as_str()) && !by_bind && !by_buffer_size {
                    self.emit(
                        "NATIVE_ABI_UNBOUND_PARAM",
                        format!(
                            "Native function `{}` parameter `{pname}` has no matching ABI slot and no BIND IN field.",
                            function.name
                        ),
                    );
                }
            }
            for (slot, _) in &function.consts {
                if !abi_slot_names.contains(slot.as_str()) {
                    self.emit(
                        "NATIVE_CONST_UNKNOWN_SLOT",
                        format!(
                            "Native function `{}` CONST pins unknown ABI slot `{slot}`.",
                            function.name
                        ),
                    );
                }
            }
            // plan-50-I: an identifier in a SUCCESS_ON/RESULT expression must name
            // a real slot (or the ABI return). Before I, `lower_link_expr` mapped
            // EVERY identifier onto one nameless "native return" variable, so
            // `SUCCESS_ON typo = 0` silently meant `status = 0` and no expression
            // could read any other slot — despite the spec saying it could.
            {
                let mut names = Vec::new();
                for expr in [&function.success_on, &function.result]
                    .into_iter()
                    .flatten()
                {
                    crate::ir::link_expr_var_names(expr, &mut names);
                }
                for name in names {
                    if name != function.abi_return_name && !abi_slot_names.contains(name) {
                        self.emit(
                            "NATIVE_ABI_UNBOUND_SLOT",
                            format!(
                                "Native function `{}` SUCCESS_ON/RESULT expression reads `{name}`, which is not an ABI slot or the ABI return.",
                                function.name
                            ),
                        );
                    }
                }
            }
            // The IR's FREE form keeps only slot+symbol (the deallocator's
            // signature check stays in syntaxcheck): the symbol must be present.
            if let Some(free) = &function.free {
                if free.symbol.is_empty() {
                    self.emit(
                        "NATIVE_FREE_INVALID",
                        format!(
                            "Native function `{}` has a malformed FREE block: it must release the `return` CPtr produced slot through a deallocator taking one CPtr parameter and returning CVoid.",
                            function.name
                        ),
                    );
                }
            }
            // plan-53-B: validate `BIND STATE <res> = <out-struct-slot>` at the
            // declaration, not later at thunk emission (a package build never emits
            // the thunk, so a malformed one would otherwise reach a consumer as a
            // hard codegen error rather than a diagnostic). The named slot must be
            // an OUT/INOUT CSTRUCT slot whose mapped record is the resource's STATE
            // type, and the function must actually return that stateful resource.
            if let Some(struct_slot) = &function.bind_state {
                let slot = function.abi_slots.iter().find(|s| &s.name == struct_slot);
                let cstruct = slot.and_then(|s| {
                    project
                        .link_cstructs
                        .iter()
                        .find(|c| c.alias == function.alias && c.name == s.ctype)
                });
                let writes_back = slot.is_some_and(|s| s.direction.writes_back());
                if slot.is_none() || cstruct.is_none() || !writes_back {
                    self.emit(
                        "NATIVE_BIND_STATE_INVALID",
                        format!(
                            "Native function `{}` BIND STATE names `{struct_slot}`, which is not an OUT CSTRUCT slot.",
                            function.name
                        ),
                    );
                } else if !function.return_resource || function.return_state_type.is_none() {
                    self.emit(
                        "NATIVE_BIND_STATE_INVALID",
                        format!(
                            "Native function `{}` has a BIND STATE but does not return a resource with a STATE clause (`AS RES T STATE S`).",
                            function.name
                        ),
                    );
                } else if let (Some(cstruct), Some(state)) = (cstruct, &function.return_state_type)
                {
                    if &cstruct.maps_to != state {
                        self.emit(
                            "NATIVE_BIND_STATE_INVALID",
                            format!(
                                "Native function `{}` BIND STATE marshals `{}` (record `{}`) but the resource's STATE type is `{state}`.",
                                function.name, cstruct.name, cstruct.maps_to
                            ),
                        );
                    }
                }
                // bug-326-A10: the `<res>` half must name the slot the wrapper
                // actually returns. Codegen ignores it (the STATE always attaches
                // to the return), so an unchecked name made `BIND STATE typo =
                // info` compile in silence while the STATE landed on the real
                // return — mandatory syntax that meant nothing. `None` on the
                // package path, where the name never rode the wire.
                if let Some(named) = &function.bind_state_resource {
                    // `RETURN <slot>` names the produced resource; a computed
                    // `RETURN status = 100` names no slot, and the arm above
                    // already rejects that shape for a stateful resource return.
                    let produced = match &function.result {
                        Some(crate::ir::IrLinkExpr::Var(slot)) => Some(slot.as_str()),
                        Some(_) => None,
                        None => Some(function.abi_return_name.as_str()),
                    };
                    if let Some(produced) = produced {
                        if named != produced {
                            self.emit(
                                "NATIVE_BIND_STATE_INVALID",
                                format!(
                                    "Native function `{}` BIND STATE names resource slot `{named}`, but the wrapper returns `{produced}`; the STATE attaches to the returned slot.",
                                    function.name
                                ),
                            );
                        }
                    }
                }
            }
        }

        // plan-53-A: a native resource's STATE type is fixed, so every native
        // declaration that names it — a producer's `AS RES R STATE S`, a
        // consumer's `RES x AS R STATE S` (e.g. the close op) — must agree on `S`.
        // The payload carries no runtime tag, so a producer allocating `S` and a
        // close reading `S2` is the same untagged type confusion plan-52-C closes
        // at an ordinary parameter, at the native boundary. Collect (resource -> S)
        // over all link functions and reject a second, different S.
        let mut resource_state: HashMap<String, String> = HashMap::new();
        let mut check = |base: &str, state: &str, env: &Self| {
            match resource_state.get(base) {
                Some(existing) if existing != state => env.emit(
                    "TYPE_STATE_MISMATCH",
                    format!(
                        "native resource `{base}` is declared with STATE `{existing}` and also STATE `{state}`; a resource's STATE type is fixed and every native declaration of it must agree."
                    ),
                ),
                Some(_) => {}
                None => {
                    resource_state.insert(base.to_string(), state.to_string());
                }
            }
        };
        for function in &project.link_functions {
            if function.return_resource {
                if let Some(state) = &function.return_state_type {
                    check(resource_base_type(&function.return_type), state, self);
                }
            }
            for (_, ptype) in &function.params {
                if let Some(state) = crate::builtins::resource::state_type_name(ptype) {
                    check(resource_base_type(ptype), state, self);
                }
            }
        }
    }

    /// Whether a type contains a resource or thread handle anywhere (mirrors
    /// syntaxcheck's `contains_resource_or_thread` on type strings).
    pub(super) fn contains_resource_or_thread(
        &self,
        type_: &str,
        seen: &mut HashSet<String>,
    ) -> bool {
        let t = resource_base_type(type_);
        if t.starts_with("Thread") || self.is_resource_or_resource_union(t) {
            return true;
        }
        if let Some(e) = t.strip_prefix("List OF ") {
            return self.contains_resource_or_thread(e, seen);
        }
        if let Some((k, v)) = parse_map(t) {
            return self.contains_resource_or_thread(k, seen)
                || self.contains_resource_or_thread(v, seen);
        }
        if !seen.insert(t.to_string()) {
            return false;
        }
        let contained = self.record_field_lists.get(t).is_some_and(|fields| {
            fields
                .iter()
                .any(|(_, ft)| self.contains_resource_or_thread(ft, seen))
        });
        seen.remove(t);
        contained
    }

    /// Whether `base` is positively a non-resource data type: a primitive, a
    /// declared record/enum, a collection/FUNC type, or a union with no
    /// resource variants. Unknown names are NOT provably data (they may be an
    /// external package's resource type).
    pub(super) fn provably_data_type(&self, base: &str) -> bool {
        matches!(
            base,
            "Boolean"
                | "Byte"
                | "Error"
                | "ErrorLoc"
                | "Fixed"
                | "Float"
                | "Integer"
                | "Money"
                | "Nothing"
                | "Scalar"
                | "String"
        ) || base.starts_with("List OF ")
            || base.starts_with("Map OF ")
            || base.starts_with("FUNC")
            || (self.records.contains_key(base) && self.close_op_for(base).is_none())
            || self.enums.contains_key(base)
            || self
                .unions
                .get(base)
                .is_some_and(|u| u.variants.iter().all(|v| self.close_op_for(v).is_none()))
    }

    /// Whether `base` is a resource type or a resource union (a union any of
    /// whose variants is a resource — mixed unions are already rejected).
    pub(super) fn is_resource_or_resource_union(&self, base: &str) -> bool {
        if self.close_op_for(base).is_some() {
            return true;
        }
        self.unions
            .get(base)
            .is_some_and(|u| u.variants.iter().any(|v| self.close_op_for(v).is_some()))
    }

    /// The registered close op for a resource type: user-declared native
    /// resources first (`RESOURCE T CLOSE BY alias.func`), then the builtin
    /// close table.
    pub(super) fn close_op_for(&self, base: &str) -> Option<&str> {
        self.resource_closers
            .get(base)
            .map(String::as_str)
            .or_else(|| builtins::resource::builtin_resource_close_function(base))
    }

    /// The resource binding consumed by an op, if any: a call to the binding's
    /// registered close op with it as the first argument, or `RETURN <binding>`.
    pub(super) fn consumed_resource(
        &self,
        op: &IrOp,
        locals: &HashMap<String, String>,
    ) -> Option<String> {
        let close_consumes = |value: &IrValue| -> Option<String> {
            let (target, args) = match value {
                IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
                    (target, args)
                }
                _ => return None,
            };
            // NOTE: thread::transfer is intentionally NOT treated as a move
            // here. On the failure path of `transfer(...) TRAP(e)` ownership
            // returns to the sender so the handler may close the resource — a
            // straight-line detector cannot see that and would false-reject the
            // valid recover pattern. syntaxcheck models the restore explicitly;
            // the IR checker stays conservative and only tracks close/return.
            // A registered close op consumes the resource at arg 0.
            let IrValue::Local(name) = args.first()? else {
                return None;
            };
            let type_ = locals.get(name)?;
            let base = resource_base_type(type_);
            if self.close_op_for(base) == Some(target.as_str()) {
                Some(name.clone())
            } else {
                None
            }
        };
        match op {
            IrOp::Eval { value, .. } => close_consumes(value),
            IrOp::Bind {
                value: Some(value), ..
            } => close_consumes(value),
            IrOp::Assign { value, .. } => close_consumes(value),
            IrOp::Return {
                value: Some(IrValue::Local(name)),
                ..
            } => {
                let type_ = locals.get(name)?;
                if self.close_op_for(resource_base_type(type_)).is_some() {
                    Some(name.clone())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    // ===========================================================================
}
