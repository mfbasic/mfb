use super::*;

impl TypeEnv {
    // 8. Native LINK (cstructs + functions) + resource classification
    // ===========================================================================
    //
    // the former source checker's `check_link_block`, on the IR. The walk order mirrors the
    // front end's — per LINK block: CSTRUCT declarations (duplicates + layout
    // faults), CSTRUCT escape, then per function: the signature/ABI rules, the
    // struct-slot and BIND IN rules, the buffer rules — so a relocated rule
    // reorders only across streams, never within the family.
    //
    // Locations come from `link_spans` (plan-107-C): on the source path every
    // emission points at the slot/parameter/field/declaration line the former source checker
    // used; a decoded package has no spans and reports unlocated (file `""`,
    // line 0 — the `<generated>` form), as the package path always has.
    //
    // A crafted `.mfp` drives raw C calls, so every one of these is a
    // marshaling-safety gate, not a convenience. Note what is NOT validated:
    // struct offsets and sizes, because they are never transported — they are
    // recomputed from the field ctypes, so a package has no offset to forge.

    fn function_spans(&self, function: &crate::ir::IrLinkFunction) -> crate::ir::LinkFunctionSpans {
        self.link_spans
            .functions
            .get(&(function.alias.clone(), function.name.clone()))
            .cloned()
            .unwrap_or_default()
    }

    fn cstruct_spans(&self, cstruct: &crate::ir::IrCStruct) -> crate::ir::CStructSpans {
        self.link_spans
            .cstructs
            .get(&(cstruct.alias.clone(), cstruct.name.clone()))
            .cloned()
            .unwrap_or_default()
    }

    /// Point the next emissions at `line` of `file` (`""`/0 = unlocated).
    fn locate(&self, file: &str, line: u32) {
        self.current_file.replace(file.to_string());
        self.current_line.set(line);
    }

    pub(super) fn check_link_blocks(&self, project: &IrProject) {
        // Block order: first appearance of an alias across the function table,
        // then any CSTRUCT-only alias (both tables are in file order).
        let mut aliases: Vec<&str> = Vec::new();
        for function in &project.link_functions {
            if !aliases.contains(&function.alias.as_str()) {
                aliases.push(&function.alias);
            }
        }
        for cstruct in &project.link_cstructs {
            if !aliases.contains(&cstruct.alias.as_str()) {
                aliases.push(&cstruct.alias);
            }
        }
        for alias in aliases {
            self.check_cstruct_decls(project, alias);
            self.check_cstruct_escape(project, alias);
            for function in project.link_functions.iter().filter(|f| f.alias == alias) {
                self.check_link_function(project, function);
                self.check_struct_slots(project, function);
                self.check_buffer_faults(function);
            }
        }
        self.check_link_state_agreement(project);
        self.locate("", 0);
    }

    /// Validate one LINK block's `CSTRUCT` declarations (plan-50-B §4.4): a
    /// duplicate name would make slot resolution ambiguous, and every layout
    /// fault the shared `check_cstruct` reports (pointed at the offending field
    /// where the message names one, as the former source checker does).
    fn check_cstruct_decls(&self, project: &IrProject, alias: &str) {
        let names: Vec<String> = project
            .link_cstructs
            .iter()
            .filter(|c| c.alias == alias)
            .map(|c| c.name.clone())
            .collect();
        for (index, cstruct) in project.link_cstructs.iter().enumerate() {
            if cstruct.alias != alias {
                continue;
            }
            let spans = self.cstruct_spans(cstruct);
            self.locate(&spans.file, spans.line);
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
            let fields: Vec<(String, ParameterType)> = cstruct
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.ctype.clone()))
                .collect();
            // Every supported target is LP64 and agrees on the layout table, so
            // the target choice cannot change a decoded layout.
            for fault in crate::ir::check_cstruct(&cstruct.name, &fields, &names, "") {
                let line = cstruct
                    .fields
                    .iter()
                    .position(|f| fault.message.contains(&format!("`{}`", f.name)))
                    .and_then(|i| spans.fields.get(i).copied())
                    .unwrap_or(spans.line);
                self.current_line.set(line);
                self.emit(fault.rule, fault.message);
            }
        }
    }

    /// A `CSTRUCT` name is a native-side layout descriptor, not a type: it may
    /// appear only in its own declaration, an `ABI (...)` slot and `SIZEOF`.
    /// Naming one in a wrapper's MFBASIC-facing signature would make a private C
    /// layout part of the public API (plan-50-B §4.5). On the source path the
    /// resolver usually catches this first; a decoded package never ran it.
    fn check_cstruct_escape(&self, project: &IrProject, alias: &str) {
        let names: Vec<&str> = project
            .link_cstructs
            .iter()
            .filter(|c| c.alias == alias)
            .map(|c| c.name.as_str())
            .collect();
        if names.is_empty() {
            return;
        }
        for function in project.link_functions.iter().filter(|f| f.alias == alias) {
            let spans = self.function_spans(function);
            for (index, (pname, ptype)) in function.params.iter().enumerate() {
                if names.contains(&ptype.name().as_ref()) {
                    self.locate(&spans.file, spans.params.get(index).copied().unwrap_or(0));
                    self.emit(
                        "NATIVE_CSTRUCT_ESCAPE",
                        format!(
                            "Native function `{}` parameter `{pname}` uses CSTRUCT `{}`; name its mapped record type instead — a CSTRUCT is nameable only in an ABI slot or SIZEOF.",
                            function.name,
                            ptype.name()
                        ),
                    );
                }
            }
            if names.contains(&function.return_type.name().as_ref()) {
                self.locate(&spans.file, spans.line);
                self.emit(
                    "NATIVE_CSTRUCT_ESCAPE",
                    format!(
                        "Native function `{}` returns CSTRUCT `{}`; name its mapped record type instead — a CSTRUCT is nameable only in an ABI slot or SIZEOF.",
                        function.name,
                        function.return_type.name()
                    ),
                );
            }
        }
    }

    /// The signature/ABI rules of one wrapper (the former source checker's
    /// `check_link_function_in`): C ABI types may not escape into the wrapper
    /// signature, the slot ctype namespace is closed, every slot binds to a
    /// parameter / CONST pin / BIND IN block or is OUT, expressions read real
    /// slots, a value-producing wrapper names its result and a `Nothing` one
    /// does not, every parameter is consumed, CONST pins name real slots, FREE
    /// and BIND STATE are well formed.
    fn check_link_function(&self, project: &IrProject, function: &crate::ir::IrLinkFunction) {
        // bug-342 A2: this is a *reject* predicate — a raw C ABI type appearing as
        // a wrapper's MFB-facing parameter or return type is a `NATIVE_CPTR_ESCAPE`
        // (C types belong only in `ABI` slots). It deliberately INCLUDES `CVoid`,
        // making it a defensive superset of the spec's ABI-slot allow-list
        // (the former source checker's `helpers::is_c_abi_type`, which omits `CVoid` per
        // `17_native-libraries.md:92`). The two are NOT contradictory: they serve
        // different purposes — an allow-list for what may sit in an ABI slot vs. a
        // reject-list for what must never leak into an MFB signature — and on the
        // package (`.mfp`) path a crafted `return_type: "CVoid"` SHOULD be rejected.
        // So they are kept separate on purpose; do not "unify" by dropping `CVoid`.
        fn is_c_abi_type(t: &ParameterType) -> bool {
            use crate::types::CAbiType;
            // plan-113: every C ABI spelling is now a `ParameterType::C`, so
            // this asks the variant instead of the interned `Symbol` a `Named`
            // used to hold.
            //
            // **13 of the 16, and must NOT become `t.c_abi().is_some()`.** It
            // deliberately includes `CVoid` (see the block comment above) but
            // excludes `CBool`, `CByte` and `CBuffer`; widening it changes what
            // `NATIVE_CPTR_ESCAPE` rejects on the `.mfp` path.
            matches!(
                t.c_abi(),
                Some(
                    CAbiType::Ptr
                        | CAbiType::Str
                        | CAbiType::Int8
                        | CAbiType::Int16
                        | CAbiType::Int32
                        | CAbiType::Int64
                        | CAbiType::UInt8
                        | CAbiType::UInt16
                        | CAbiType::UInt32
                        | CAbiType::UInt64
                        | CAbiType::Float
                        | CAbiType::Double
                        | CAbiType::Void
                )
            )
        }
        let spans = self.function_spans(function);
        let file = spans.file.as_str();
        let slot_line = |i: usize| spans.slots.get(i).copied().unwrap_or(0);
        let param_line = |i: usize| spans.params.get(i).copied().unwrap_or(0);
        // plan-113: a CSTRUCT-named slot stays a `Named` -- that namespace is
        // open, unlike the closed 16 the `C` variant holds.
        let is_cstruct_slot = |ctype: &ParameterType| {
            project
                .link_cstructs
                .iter()
                .any(|c| c.alias == function.alias && ctype.is_named(&c.name))
        };

        for (index, (pname, ptype)) in function.params.iter().enumerate() {
            if is_c_abi_type(ptype) {
                self.locate(file, param_line(index));
                self.emit(
                    "NATIVE_CPTR_ESCAPE",
                    format!(
                        "Native function `{}` parameter `{pname}` uses C ABI type `{}`; raw C types may appear only in ABI slots.",
                        function.name,
                        ptype.name()
                    ),
                );
            }
        }
        if is_c_abi_type(&function.return_type) {
            self.locate(file, spans.line);
            self.emit(
                "NATIVE_CPTR_ESCAPE",
                format!(
                    "Native function `{}` returns C ABI type `{}`; raw C types may appear only in ABI slots.",
                    function.name,
                    function.return_type.name()
                ),
            );
        }

        // plan-50-A: the slot ctype namespace is closed. An unknown name used to
        // fall through to a raw 64-bit marshal (`link_thunk`'s default arm), so a
        // typo compiled clean and silently moved the wrong width.
        if !crate::ir::abi_ctype_valid_as_return(&function.abi_return_ctype) {
            self.locate(file, spans.abi_line);
            self.emit(
                "NATIVE_ABI_UNKNOWN_CTYPE",
                format!(
                    "Native function `{}` ABI return `{}` uses C type `{}`, which is not a valid ABI return type.",
                    function.name, function.abi_return_name, function.abi_return_ctype
                ),
            );
        }
        for (index, slot) in function.abi_slots.iter().enumerate() {
            // A slot may name a CSTRUCT declared in the same LINK alias; the
            // struct rules then apply instead of the scalar table (plan-50-E).
            if is_cstruct_slot(&slot.ctype) {
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
                self.locate(file, slot_line(index));
                self.emit(
                    "NATIVE_ABI_UNKNOWN_CTYPE",
                    format!(
                        "Native function `{}` ABI slot `{}` uses C type `{}`, which is not valid in that position.",
                        function.name, slot.name, slot.ctype
                    ),
                );
            }
        }

        // Every ABI slot must be satisfied by exactly one of: a wrapper parameter
        // (matched by name), an OUT direction (native storage the callee fills,
        // surfaced if at all by `RETURN`), a CONST pin, or — for an IN struct
        // slot — its `BIND IN` block (plan-50-E). A struct slot is NOT exempt
        // (the front end's second slot pass does not skip it): an IN struct slot
        // with neither a parameter nor a BIND IN is unbound.
        let const_slots: HashSet<&str> = function
            .consts
            .iter()
            .map(|(slot, _)| slot.as_str())
            .collect();
        let param_names: HashSet<&str> = function.params.iter().map(|(n, _)| n.as_str()).collect();
        for (index, slot) in function.abi_slots.iter().enumerate() {
            if const_slots.contains(slot.name.as_str()) {
                if slot.direction.writes_back() {
                    self.locate(file, slot_line(index));
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
            if slot.direction.writes_back() {
                continue;
            }
            if function.bind_in.iter().any(|b| b.slot == slot.name) {
                continue;
            }
            if !param_names.contains(slot.name.as_str()) {
                self.locate(file, slot_line(index));
                self.emit(
                    "NATIVE_ABI_UNBOUND_SLOT",
                    format!(
                        "Native function `{}` ABI slot `{}` does not bind to a parameter, CONST pin, or an OUT buffer.",
                        function.name, slot.name
                    ),
                );
            }
        }

        // plan-50-I: an identifier in a SUCCESS_ON/RETURN expression must name a
        // real slot (or the ABI return). Before I, `lower_link_expr` mapped EVERY
        // identifier onto one nameless "native return" variable, so
        // `SUCCESS_ON typo = 0` silently meant `status = 0` and no expression
        // could read any other slot — despite the spec saying it could.
        let abi_slot_names: HashSet<&str> = function
            .abi_slots
            .iter()
            .map(|slot| slot.name.as_str())
            .collect();
        {
            let mut names = Vec::new();
            for expr in [&function.success_on, &function.result]
                .into_iter()
                .flatten()
            {
                crate::ir::link_expr_var_names(expr, &mut names);
            }
            for name in names {
                // `NOTHING` is a literal, not a slot.
                if name == "NOTHING"
                    || name == function.abi_return_name
                    || abi_slot_names.contains(name)
                {
                    continue;
                }
                self.locate(file, spans.abi_line);
                self.emit(
                    "NATIVE_ABI_UNBOUND_SLOT",
                    format!(
                        "Native function `{}` SUCCESS_ON/RETURN expression reads `{name}`, which is not an ABI slot or the ABI return.",
                        function.name
                    ),
                );
            }
        }

        // plan-50-H: the result is whatever `RETURN <expr>` names. A producer
        // (`AS RES X`) and any non-Nothing wrapper must surface exactly one
        // result; a `Nothing` wrapper surfaces none.
        // plan-111-B: `!= "Nothing"` on the rendered name is the `Nothing`
        // variant question.
        let wants_result =
            function.return_resource || !matches!(function.return_type, ParameterType::Nothing);
        if wants_result && function.result.is_none() {
            self.locate(file, spans.line);
            self.emit(
                "NATIVE_ABI_NO_RESULT",
                format!(
                    "Native function `{}` returns a value but declares no `RETURN <expr>` naming its result.",
                    function.name
                ),
            );
        }
        if !wants_result && function.result.is_some() {
            self.locate(file, spans.line);
            self.emit(
                "NATIVE_ABI_RESULT_MARKER",
                format!(
                    "Native function `{}` returns Nothing but declares a `RETURN`.",
                    function.name
                ),
            );
        }

        // Every wrapper parameter must be consumed: by an ABI slot of the same
        // name, by a `BIND IN` field that binds it (plan-50-E — a parameter
        // feeding a struct field has no slot of its own), or by a `BUFFER … SIZE`
        // expression (plan-58-B — a parameter that only sizes an OUT CBuffer
        // likewise has no slot of its own).
        for (index, (pname, _)) in function.params.iter().enumerate() {
            let by_bind = function.bind_in.iter().any(|b| {
                b.fields
                    .iter()
                    .any(|f| f.param.as_deref() == Some(pname.as_str()))
            });
            let by_buffer_size = function.buffers.iter().any(|b| {
                let mut names = Vec::new();
                crate::ir::link_expr_var_names(&b.size, &mut names);
                names.contains(&pname.as_str())
            });
            if !abi_slot_names.contains(pname.as_str()) && !by_bind && !by_buffer_size {
                self.locate(file, param_line(index));
                self.emit(
                    "NATIVE_ABI_UNBOUND_PARAM",
                    format!(
                        "Native function `{}` parameter `{pname}` has no matching ABI slot and no BIND IN field.",
                        function.name
                    ),
                );
            }
        }

        // A CONST pin must name a real ABI slot. (Its other rule — that the pin
        // FOLDS to an immediate — reads the pin's expression, which lowering has
        // already folded away; that form is the shape pass's, plan-107-D.)
        for (index, (slot, _)) in function.consts.iter().enumerate() {
            if !abi_slot_names.contains(slot.as_str()) {
                self.locate(file, spans.consts.get(index).copied().unwrap_or(0));
                self.emit(
                    "NATIVE_CONST_UNKNOWN_SLOT",
                    format!(
                        "Native function `{}` CONST pins unknown ABI slot `{slot}`.",
                        function.name
                    ),
                );
            }
        }

        // The IR's FREE form keeps only slot + symbol (the deallocator's
        // signature check is the shape pass's, plan-107-D): the symbol must be
        // present, and — sec-01 — a decoded `.mfp` bypasses the front end, so
        // `FREE` on an `AS RES` producer is rejected here too: it would free the
        // live handle stored in the resource record (UAF + double-free). This
        // guard is load-bearing for binary packages.
        if let Some(free) = &function.free {
            self.locate(file, spans.free_line);
            if function.return_resource {
                self.emit(
                    "NATIVE_FREE_INVALID",
                    format!(
                        "Native function `{}` declares a FREE block on an `AS RES` resource producer: a resource producer keeps the native handle alive in its record and must not free it (FREE is only for a caller-owned value copied out of the return).",
                        function.name
                    ),
                );
            } else if free.symbol.is_empty() {
                self.emit(
                    "NATIVE_FREE_INVALID",
                    format!(
                        "Native function `{}` has a malformed FREE block: it must name the CPtr produced slot that `RETURN` surfaces, and its deallocator must take one CPtr parameter and return CVoid.",
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
        // Reported at the wrapper's line: the `BIND STATE` clause carries no line
        // of its own (it has always been this checker's rule alone).
        if let Some(struct_slot) = &function.bind_state {
            self.locate(file, spans.line);
            let slot = function.abi_slots.iter().find(|s| &s.name == struct_slot);
            let cstruct = slot.and_then(|s| {
                project
                    .link_cstructs
                    .iter()
                    .find(|c| c.alias == function.alias && s.ctype.is_named(&c.name))
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
            } else if let (Some(cstruct), Some(state)) = (cstruct, &function.return_state_type) {
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

    /// A wrapper's struct slots and `BIND IN` blocks (plan-50-E §4.6;
    /// the former source checker's `check_struct_slots`). A crafted `.mfp` never ran the front
    /// end, so without this the package path would be the weaker of the two.
    fn check_struct_slots(&self, project: &IrProject, function: &crate::ir::IrLinkFunction) {
        let spans = self.function_spans(function);
        let file = spans.file.as_str();
        let slot_line = |i: usize| spans.slots.get(i).copied().unwrap_or(0);
        let find_cstruct = |ctype: &ParameterType| {
            project
                .link_cstructs
                .iter()
                .find(|c| c.alias == function.alias && ctype.is_named(&c.name))
        };

        for (index, slot) in function.abi_slots.iter().enumerate() {
            let Some(decl) = find_cstruct(&slot.ctype) else {
                // A non-struct slot marked INOUT has nothing to be in/out *of*:
                // a scalar slot is either a C argument or a produced value.
                if slot.direction == crate::ir::AbiDirection::InOut {
                    self.locate(file, slot_line(index));
                    self.emit(
                        "NATIVE_ABI_UNKNOWN_CTYPE",
                        format!(
                            "Native function `{}` ABI slot `{}` is INOUT but `{}` is not a CSTRUCT; INOUT is meaningful only for a struct.",
                            function.name, slot.name, slot.ctype
                        ),
                    );
                }
                continue;
            };
            // The record it maps to must exist and be a record.
            let Some(rec) = project.types.iter().find(|t| {
                decl.maps_to.is_named(&t.name) && (t.kind == "type" || t.kind == "record")
            }) else {
                let decl_spans = self.cstruct_spans(decl);
                self.locate(&decl_spans.file, decl_spans.line);
                self.emit(
                    "NATIVE_STRUCT_FIELD_MISMATCH",
                    format!(
                        "CSTRUCT `{}` maps to `{}`, which is not a record type.",
                        decl.name,
                        decl.maps_to.name()
                    ),
                );
                continue;
            };
            let cfields: Vec<(String, ParameterType)> = decl
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.ctype.clone()))
                .collect();
            let record: Vec<(String, String)> = rec
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.type_.name().into_owned()))
                .collect();
            // `maps_to` is diagnostic TEXT in the view, so it renders here.
            let maps_to = decl.maps_to.name();
            let view = crate::ir::StructSlotView {
                cfields: &cfields,
                record: &record,
                cstruct_name: &decl.name,
                maps_to: &maps_to,
            };
            self.locate(file, slot_line(index));
            for fault in crate::ir::check_struct_slot(&view) {
                self.emit(fault.rule, fault.message);
            }
            // A wrapper returning this struct must declare the mapped record,
            // and cannot return an IN slot (zeroed, never read back).
            if matches!(&function.result, Some(crate::ir::IrLinkExpr::Var(n)) if *n == slot.name) {
                if slot.direction == crate::ir::AbiDirection::In {
                    self.locate(file, slot_line(index));
                    self.emit(
                        "NATIVE_ABI_RESULT_MARKER",
                        format!(
                            "Native function `{}` returns struct slot `{}`, which is IN — an input slot is zeroed and never read back.",
                            function.name, slot.name
                        ),
                    );
                }
                if function.return_type != decl.maps_to {
                    self.locate(file, spans.line);
                    self.emit(
                        "NATIVE_STRUCT_FIELD_MISMATCH",
                        format!(
                            "Native function `{}` returns struct slot `{}`, so it must return `{}` (the CSTRUCT's mapped record).",
                            function.name, slot.name, decl.maps_to
                        ),
                    );
                }
            }
        }

        // BIND IN: the slot must exist, be a struct, be readable as input, and
        // every field must be a real field bound to a real value.
        for (bind_index, bind) in function.bind_in.iter().enumerate() {
            let (bind_line, field_lines) = spans
                .bind_in
                .get(bind_index)
                .cloned()
                .unwrap_or((0, Vec::new()));
            let field_line = |i: usize| field_lines.get(i).copied().unwrap_or(0);
            let Some(slot) = function.abi_slots.iter().find(|s| s.name == bind.slot) else {
                self.locate(file, bind_line);
                self.emit(
                    "NATIVE_BIND_IN_INVALID",
                    format!(
                        "Native function `{}` BIND IN names ABI slot `{}`, which does not exist.",
                        function.name, bind.slot
                    ),
                );
                continue;
            };
            let Some(decl) = find_cstruct(&slot.ctype) else {
                self.locate(file, bind_line);
                self.emit(
                    "NATIVE_BIND_IN_INVALID",
                    format!(
                        "Native function `{}` BIND IN names slot `{}`, which is `{}` and not a CSTRUCT.",
                        function.name, bind.slot, slot.ctype
                    ),
                );
                continue;
            };
            if slot.direction == crate::ir::AbiDirection::Out {
                self.locate(file, bind_line);
                self.emit(
                    "NATIVE_BIND_IN_INVALID",
                    format!(
                        "Native function `{}` BIND IN writes slot `{}`, which is OUT — an OUT slot is zeroed and filled by the callee.",
                        function.name, bind.slot
                    ),
                );
            }
            let mut seen: Vec<&str> = Vec::new();
            for (field_index, field) in bind.fields.iter().enumerate() {
                self.locate(file, field_line(field_index));
                if !decl.fields.iter().any(|f| f.name == field.name) {
                    self.emit(
                        "NATIVE_BIND_IN_INVALID",
                        format!(
                            "Native function `{}` BIND IN sets `{}`, which CSTRUCT `{}` does not declare.",
                            function.name, field.name, decl.name
                        ),
                    );
                }
                if seen.contains(&field.name.as_str()) {
                    self.emit(
                        "NATIVE_BIND_IN_INVALID",
                        format!(
                            "Native function `{}` BIND IN sets `{}` more than once.",
                            function.name, field.name
                        ),
                    );
                }
                seen.push(field.name.as_str());
                // Lowering binds a wrapper parameter as `param` and an integer /
                // boolean literal as `literal`; any other source value lowers to
                // NEITHER (representable as invalid rather than folded to 0).
                match (&field.param, &field.literal) {
                    (None, None) => self.emit(
                        "NATIVE_BIND_IN_INVALID",
                        format!(
                            "Native function `{}` BIND IN sets `{}` from a value that is neither a wrapper parameter nor an integer literal.",
                            function.name, field.name
                        ),
                    ),
                    // Both set is not a source shape — only crafted IR.
                    (Some(_), Some(_)) => self.emit(
                        "NATIVE_BIND_IN_INVALID",
                        format!(
                            "Native function `{}` BIND IN field `{}` must bind exactly one of a parameter or a literal.",
                            function.name, field.name
                        ),
                    ),
                    (Some(param), None) => {
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
                    (None, Some(_)) => {}
                }
            }
        }
    }

    /// plan-58-A: the `CBuffer` position rules, shared verbatim with the front
    /// end via `ir::check_buffer_slots` so a crafted `.mfp` gets exactly the
    /// source-path treatment. Reported at the `ABI` line, as the front end does
    /// (the shared fault carries only rule + message, and every message names
    /// the offending slot).
    fn check_buffer_faults(&self, function: &crate::ir::IrLinkFunction) {
        let spans = self.function_spans(function);
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
                .map(|s| (s.name.as_str(), &s.ctype, s.direction))
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
        self.locate(&spans.file, spans.abi_line);
        for fault in crate::ir::check_buffer_slots(&view) {
            self.emit(fault.rule, fault.message);
        }
    }

    /// plan-53-A: a native resource's STATE type is fixed, so every native
    /// declaration that names it — a producer's `AS RES R STATE S`, a
    /// consumer's `RES x AS R STATE S` (e.g. the close op) — must agree on `S`.
    /// The payload carries no runtime tag, so a producer allocating `S` and a
    /// close reading `S2` is the same untagged type confusion plan-52-C closes
    /// at an ordinary parameter, at the native boundary. Collect (resource → S)
    /// over all link functions and reject a second, different S, at the
    /// disagreeing declaration.
    fn check_link_state_agreement(&self, project: &IrProject) {
        // plan-111-B: keyed by the resource TYPE, and the state is a type too.
        // Both used to be spellings, which is why the parameter side had to
        // re-split `ptype` through `codegen::resource::state_type_name` and the
        // return side through `resource_base_type_name` — the last two callers
        // of that name-domain twin, now deleted.
        let mut resource_state: HashMap<ParameterType, ParameterType> = HashMap::new();
        let mut check = |base: &ParameterType, state: &ParameterType, env: &Self| {
            match resource_state.get(base) {
                Some(existing) if existing != state => {
                    let (base, existing, state) = (base.name(), existing.name(), state.name());
                    env.emit(
                        "TYPE_STATE_MISMATCH",
                        format!(
                            "native resource `{base}` is declared with STATE `{existing}` and also STATE `{state}`; a resource's STATE type is fixed and every native declaration of it must agree."
                        ),
                    )
                }
                Some(_) => {}
                None => {
                    resource_state.insert(base.clone(), state.clone());
                }
            }
        };
        for function in &project.link_functions {
            let spans = self.function_spans(function);
            self.locate(&spans.file, spans.line);
            if function.return_resource {
                if let Some(state) = &function.return_state_type {
                    check(&resource_base_type(&function.return_type), state, self);
                }
            }
            for (index, (_, ptype)) in function.params.iter().enumerate() {
                if let Some(state) = ptype.state() {
                    self.current_line
                        .set(spans.params.get(index).copied().unwrap_or(0));
                    check(&resource_base_type(ptype), &state, self);
                }
            }
        }
    }

    /// Whether a type contains a resource or thread handle anywhere (mirrors
    /// the former source checker's `contains_resource_or_thread` on type strings).
    /// plan-106-B: structural. The `Thread`/`ThreadWorker` prefix test is the
    /// [`ThreadHandle`](ParameterType::ThreadHandle) variant, and the `List OF `/
    /// `Map OF ` descent is a variant match; the resource and record-field
    /// lookups stay name-keyed (they read declaration tables).
    pub(super) fn contains_resource_or_thread(
        &self,
        type_: &ParameterType,
        seen: &mut HashSet<ParameterType>,
    ) -> bool {
        let t = resource_base_type(type_);
        if is_thread_type(&t) || self.is_resource_or_resource_union(&t) {
            return true;
        }
        match &t {
            ParameterType::ListOf(e) => return self.contains_resource_or_thread(e, seen),
            ParameterType::MapOf(k, v) => {
                return self.contains_resource_or_thread(k, seen)
                    || self.contains_resource_or_thread(v, seen);
            }
            _ => {}
        }
        if !seen.insert(t.clone()) {
            return false;
        }
        let contained = self.any_field_of(&t, |ft| self.contains_resource_or_thread(ft, seen));
        seen.remove(&t);
        contained
    }

    /// Whether a type transitively contains a thread handle — the former source checker's
    /// `contains_thread`. Threads may never live in a collection; resources may
    /// (as pointers, §15.6), so a collection ELEMENT and a `Map` VALUE use this
    /// rather than the combined resource-or-thread predicate.
    pub(super) fn contains_thread(
        &self,
        type_: &ParameterType,
        seen: &mut HashSet<ParameterType>,
    ) -> bool {
        let t = resource_base_type(type_);
        if is_thread_type(&t) {
            return true;
        }
        match &t {
            ParameterType::ListOf(e) | ParameterType::SetOf(e) | ParameterType::ResultOf(e) => {
                return self.contains_thread(e, seen);
            }
            ParameterType::MapOf(k, v) => {
                return self.contains_thread(k, seen) || self.contains_thread(v, seen);
            }
            ParameterType::Res(inner) => return self.contains_thread(inner, seen),
            _ => {}
        }
        if !seen.insert(t.clone()) {
            return false;
        }
        let contained = self.any_field_of(&t, |ft| self.contains_thread(ft, seen));
        seen.remove(&t);
        contained
    }

    /// `pred` over every field of record `name`, or over every variant's fields
    /// when `name` is a union (the former source checker's `type_infos` walk covers both
    /// kinds; the union arm is what a Map key of a thread-carrying union needs).
    fn any_field_of(
        &self,
        type_: &ParameterType,
        mut pred: impl FnMut(&ParameterType) -> bool,
    ) -> bool {
        if let Some(fields) = self.record_field_lists.get(type_) {
            return fields.iter().any(|(_, ft)| pred(ft));
        }
        self.unions.get(type_).is_some_and(|union| {
            union.variant_order.iter().any(|variant| {
                self.record_field_lists
                    .get(&ParameterType::declared(variant))
                    .is_some_and(|fields| fields.iter().any(|(_, ft)| pred(ft)))
            })
        })
    }

    /// Whether `base` is positively a non-resource data type: a primitive, a
    /// declared record/enum, a collection/FUNC type, or a union with no
    /// resource variants. Unknown names are NOT provably data (they may be an
    /// external package's resource type).
    pub(super) fn provably_data_type(&self, base: &ParameterType) -> bool {
        // The `PRIMITIVE_TYPES` base plus the `Error`/`ErrorLoc` delta (both are
        // ordinary data values); this is `is_comparable_defaultable_primitive`
        // minus `Unknown`, since an unresolved name is NOT provably data
        // (bug-342 A9). Derived from the base so a new primitive flows here.
        //
        // plan-111-B: the two shape questions are variant matches now —
        // `is_collection_type(&name)` parsed the spelling it was handed, and
        // `base.starts_with("FUNC")` was a hand-rolled prefix test for `Func`.
        // The `PRIMITIVE_TYPES` membership is a name-set lookup and still
        // renders; that set is a `&'static str` table letter G retires.
        PRIMITIVE_TYPES.contains(&base.name().as_ref())
            || base.is_named("Error")
            || base.is_named("ErrorLoc")
            || base.is_named("AttributedString")
            || crate::codegen::engine::types::typed_is_collection_type(base)
            || matches!(base, ParameterType::Func(..))
            || (self.records.contains_key(base) && self.close_op_for(base).is_none())
            || self.enums.contains_key(base)
            || self
                .unions
                .get(base)
                .is_some_and(|u| u.variants.iter().all(|v| self.close_op_for(v).is_none()))
    }

    /// Whether `base` is a resource type or a resource union (a union any of
    /// whose variants is a resource — mixed unions are already rejected).
    pub(super) fn is_resource_or_resource_union(&self, base: &ParameterType) -> bool {
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
    pub(super) fn close_op_for(&self, base: &ParameterType) -> Option<&str> {
        self.resource_closers
            .get(base)
            .map(String::as_str)
            .or_else(|| {
                // plan-111-B: the builtin close table is registry surface that still
                // speaks names; its `&str` signature dies in letter E, so the type
                // renders only for that one lookup.
                crate::codegen::resource::builtin_resource_close_function(&base)
            })
    }

    /// The resource binding consumed by an op, if any: a call to the binding's
    /// registered close op with it as the first argument, or `RETURN <binding>`.
    pub(super) fn consumed_resource(
        &self,
        op: &IrOp,
        locals: &HashMap<String, ParameterType>,
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
            // valid recover pattern. The former source checker models the restore explicitly;
            // the IR checker stays conservative and only tracks close/return.
            // A registered close op consumes the resource at arg 0.
            let IrValue::Local(name) = args.first()? else {
                return None;
            };
            let type_ = locals.get(name)?;
            let base = resource_base_type(type_);
            if self.close_op_for(&base) == Some(target.as_str()) {
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
                if self.close_op_for(&resource_base_type(type_)).is_some() {
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
