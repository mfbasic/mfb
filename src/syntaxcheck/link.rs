use super::*;

impl<'a> SyntaxChecker<'a> {
    /// Register native `LINK` resources declared in this package into the
    /// resource registry as `kind = native` (plan-link-update.md §9). The close
    /// op is the dotted `alias.func`; `close_may_fail` is derived from whether the
    /// close wrapper has a `SUCCESS_ON` gate; sendability comes from the
    /// declaration's `THREAD_SENDABLE` opt-in (plan-link-update.md §8).
    pub(super) fn collect_native_resources(&mut self) {
        // Map every LINK function `alias.func` to whether it can fail (has a
        // SUCCESS_ON / ERROR_ON gate).
        let mut close_may_fail: HashMap<String, bool> = HashMap::new();
        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Link(link) = item {
                    for function in &link.functions {
                        close_may_fail.insert(
                            format!("{}.{}", link.alias, function.name),
                            function.success_on.is_some(),
                        );
                    }
                }
            }
        }

        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Resource(resource) = item {
                    let close_function = resource.close_fn.clone();
                    let may_fail = close_may_fail
                        .get(&close_function)
                        .copied()
                        .unwrap_or(false);
                    self.resource_registry.register(
                        resource.name.clone(),
                        builtins::ResourceInfo {
                            close_function,
                            sendable: resource.thread_sendable,
                            close_may_fail: may_fail,
                            kind: builtins::ResourceKind::Native,
                        },
                    );
                }
            }
        }
    }

    /// Native-specific checks on a `RESOURCE … CLOSE BY …` declaration. The
    /// structural close-op checks run during resolve; the sendability opt-in is
    /// recorded into the registry (and the `RESOURCE_TABLE` sendable bit) by
    /// `collect_native_resources` (plan-link-update.md §8/§10).
    pub(super) fn check_resource_decl(
        &mut self,
        file: &AstFile,
        resource: &crate::ast::ResourceDecl,
    ) {
        // bug-373: a user RESOURCE that reuses a built-in resource name is
        // rejected here rather than left to collide. `collect_native_resources`
        // registers it over the built-in entry, but the built-in's close op is
        // still what pulls that helper into the module, so the program reaches
        // codegen believing both meanings of the name at once and dies on the
        // internal "declares unused runtime helper" invariant. Reject the
        // collision uniformly — including when the helper happens to be elided
        // today, since that only makes the failure latent until an unrelated
        // import brings it back.
        if builtins::is_resource_type(&resource.name) {
            self.report(
                "RESOURCE_SHADOWS_BUILTIN",
                &format!(
                    "RESOURCE `{}` reuses the name of a built-in resource type. Rename it (for example `My{}`); a user resource cannot shadow a built-in.",
                    resource.name, resource.name
                ),
                file,
                resource.line,
            );
        }
    }

    /// Native-specific checks on a `LINK` block: `CPtr` containment and ABI
    /// slot/parameter consistency (plan-link-update.md §5b/§5c/§11/§12).
    pub(super) fn check_link_block(&mut self, file: &AstFile, link: &crate::ast::LinkBlock) {
        self.check_link_cstructs(file, link);
        self.check_cstruct_escape(file, link);
        let cstructs: Vec<String> = link.cstructs.iter().map(|c| c.name.clone()).collect();
        for function in &link.functions {
            self.check_link_function_in(file, function, &cstructs);
            self.check_struct_slots(file, link, function);
            self.check_buffer_slots(file, function);
        }
    }

    /// `CBuffer` slot and `BUFFER … SIZE` clause position rules (plan-58-A §4.3),
    /// shared verbatim with the package path via `ir::check_buffer_slots`.
    ///
    /// Spans are the `ABI` line rather than the individual slot line: the shared
    /// `CStructFault` carries only `(rule, message)`, and every message already
    /// names the offending slot. See the plan's Corrections — buying slot-level
    /// spans means widening a carrier four landed rules also use.
    fn check_buffer_slots(&mut self, file: &AstFile, function: &crate::ast::LinkFunction) {
        // Nothing to check unless the function actually uses the feature. The
        // `List OF Byte` return rule (rule 8) is the exception — it fires on a
        // function with no CBuffer and no BUFFER clause at all, which is precisely
        // the pre-existing garbage-codegen hole (§2.3) — so it must not be skipped.
        let uses_buffers = !function.buffers.is_empty()
            || function.abi.slots.iter().any(|s| s.ctype == "CBuffer")
            || function.abi.return_ctype == "CBuffer"
            || function.return_type.as_deref() == Some(crate::ir::BYTE_LIST_TYPE)
            || function.result_length.is_some();
        if !uses_buffers {
            return;
        }

        let size_reads: Vec<Vec<String>> = function
            .buffers
            .iter()
            .map(|b| {
                let mut names = Vec::new();
                link_expr_idents(&b.size, &mut names);
                names
            })
            .collect();
        let length_names: Option<Vec<String>> = function.result_length.as_ref().map(|expr| {
            let mut names = Vec::new();
            link_expr_idents(expr, &mut names);
            names
        });
        let view = crate::ir::BufferSlotsView {
            function: &function.name,
            slots: function
                .abi
                .slots
                .iter()
                .map(|s| (s.name.as_str(), s.ctype.as_str(), s.direction))
                .collect(),
            buffers: function
                .buffers
                .iter()
                .zip(size_reads.iter())
                .map(|(b, reads)| (b.slot.as_str(), reads.iter().map(String::as_str).collect()))
                .collect(),
            const_slots: function.consts.iter().map(|c| c.slot.as_str()).collect(),
            param_names: function.params.iter().map(|p| p.name.as_str()).collect(),
            return_type: function.return_type.as_deref().unwrap_or("Nothing"),
            abi_return_name: &function.abi.return_name,
            abi_return_ctype: &function.abi.return_ctype,
            // A bare `RETURN buf` names a slot; a computed `RETURN status = 0`
            // names none. Same extraction `check_struct_slots` uses.
            result_slot: match &function.result {
                Some(crate::ast::Expression::Identifier(name)) => Some(name.as_str()),
                _ => None,
            },
            length_reads: length_names
                .as_ref()
                .map(|names| names.iter().map(String::as_str).collect::<Vec<&str>>()),
        };
        for fault in crate::ir::check_buffer_slots(&view) {
            self.report(fault.rule, &fault.message, file, function.abi.line);
        }
    }

    /// The `(name, type)` fields of a user record `TYPE`, or `None` when the name
    /// is not a record (a union/enum/unknown cannot back a `CSTRUCT`).
    fn record_fields_of(&self, name: &str) -> Option<Vec<(String, String)>> {
        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Type(decl) = item {
                    if decl.name == name && decl.kind == crate::ast::TypeDeclKind::Type {
                        return Some(
                            decl.fields
                                .iter()
                                .map(|f| (f.name.clone(), f.type_name.clone()))
                                .collect(),
                        );
                    }
                }
            }
        }
        None
    }

    /// Validate a wrapper's struct slots and `BIND IN` blocks (plan-50-E §4.6).
    fn check_struct_slots(
        &mut self,
        file: &AstFile,
        link: &crate::ast::LinkBlock,
        function: &crate::ast::LinkFunction,
    ) {
        let find_cstruct = |name: &str| link.cstructs.iter().find(|c| c.name == name);

        for slot in &function.abi.slots {
            let Some(decl) = find_cstruct(&slot.ctype) else {
                // A non-struct slot marked INOUT has nothing to be in/out *of*:
                // a scalar slot is either a C argument or a produced value.
                if slot.direction == crate::ir::AbiDirection::InOut {
                    self.report(
                        "NATIVE_ABI_UNKNOWN_CTYPE",
                        &format!(
                            "Native function `{}` ABI slot `{}` is INOUT but `{}` is not a CSTRUCT; INOUT is meaningful only for a struct.",
                            function.name, slot.name, slot.ctype
                        ),
                        file,
                        slot.line,
                    );
                }
                continue;
            };
            // The record it maps to must exist and be a record.
            let Some(record) = self.record_fields_of(&decl.maps_to) else {
                self.report(
                    "NATIVE_STRUCT_FIELD_MISMATCH",
                    &format!(
                        "CSTRUCT `{}` maps to `{}`, which is not a record type.",
                        decl.name, decl.maps_to
                    ),
                    file,
                    decl.line,
                );
                continue;
            };
            let cfields: Vec<(String, String)> = decl
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.ctype.clone()))
                .collect();
            let view = crate::ir::StructSlotView {
                cfields: &cfields,
                record: &record,
                cstruct_name: &decl.name,
                maps_to: &decl.maps_to,
            };
            // plan-50-E marshals scalar fields only; plan-50-F lifts CString.
            for fault in crate::ir::check_struct_slot(&view) {
                self.report(fault.rule, &fault.message, file, slot.line);
            }
            // A wrapper returning this struct must declare the mapped record.
            if matches!(&function.result, Some(crate::ast::Expression::Identifier(n)) if *n == slot.name)
            {
                if slot.direction == crate::ir::AbiDirection::In {
                    self.report(
                        "NATIVE_ABI_RESULT_MARKER",
                        &format!(
                            "Native function `{}` returns struct slot `{}`, which is IN — an input slot is zeroed and never read back.",
                            function.name, slot.name
                        ),
                        file,
                        slot.line,
                    );
                }
                if function.return_type.as_deref() != Some(decl.maps_to.as_str()) {
                    self.report(
                        "NATIVE_STRUCT_FIELD_MISMATCH",
                        &format!(
                            "Native function `{}` returns struct slot `{}`, so it must return `{}` (the CSTRUCT's mapped record).",
                            function.name, slot.name, decl.maps_to
                        ),
                        file,
                        function.line,
                    );
                }
            }
        }

        // BIND IN: the slot must exist, be a struct, be readable as input, and
        // every field must be a real field bound to a real value.
        for bind in &function.bind_in {
            let Some(slot) = function.abi.slots.iter().find(|s| s.name == bind.slot) else {
                self.report(
                    "NATIVE_BIND_IN_INVALID",
                    &format!(
                        "Native function `{}` BIND IN names ABI slot `{}`, which does not exist.",
                        function.name, bind.slot
                    ),
                    file,
                    bind.line,
                );
                continue;
            };
            let Some(decl) = find_cstruct(&slot.ctype) else {
                self.report(
                    "NATIVE_BIND_IN_INVALID",
                    &format!(
                        "Native function `{}` BIND IN names slot `{}`, which is `{}` and not a CSTRUCT.",
                        function.name, bind.slot, slot.ctype
                    ),
                    file,
                    bind.line,
                );
                continue;
            };
            if slot.direction == crate::ir::AbiDirection::Out {
                self.report(
                    "NATIVE_BIND_IN_INVALID",
                    &format!(
                        "Native function `{}` BIND IN writes slot `{}`, which is OUT — an OUT slot is zeroed and filled by the callee.",
                        function.name, bind.slot
                    ),
                    file,
                    bind.line,
                );
            }
            let mut seen: Vec<&str> = Vec::new();
            for field in &bind.fields {
                if !decl.fields.iter().any(|f| f.name == field.name) {
                    self.report(
                        "NATIVE_BIND_IN_INVALID",
                        &format!(
                            "Native function `{}` BIND IN sets `{}`, which CSTRUCT `{}` does not declare.",
                            function.name, field.name, decl.name
                        ),
                        file,
                        field.line,
                    );
                }
                if seen.contains(&field.name.as_str()) {
                    self.report(
                        "NATIVE_BIND_IN_INVALID",
                        &format!(
                            "Native function `{}` BIND IN sets `{}` more than once.",
                            function.name, field.name
                        ),
                        file,
                        field.line,
                    );
                }
                seen.push(field.name.as_str());
                // A value is a wrapper parameter or an integer/boolean literal.
                let ok = match &field.value {
                    crate::ast::Expression::Identifier(name) => {
                        function.params.iter().any(|p| p.name == *name)
                    }
                    crate::ast::Expression::Number(_) | crate::ast::Expression::Boolean(_) => true,
                    crate::ast::Expression::Unary {
                        operator, operand, ..
                    } => {
                        operator == "-"
                            && matches!(operand.as_ref(), crate::ast::Expression::Number(_))
                    }
                    _ => false,
                };
                if !ok {
                    self.report(
                        "NATIVE_BIND_IN_INVALID",
                        &format!(
                            "Native function `{}` BIND IN sets `{}` from a value that is neither a wrapper parameter nor an integer literal.",
                            function.name, field.name
                        ),
                        file,
                        field.line,
                    );
                }
            }
        }
    }

    /// A `CSTRUCT` name is a native-side layout descriptor, not a type: it may
    /// appear only in its own declaration, an `ABI (...)` slot's ctype position,
    /// and `SIZEOF`. Naming one in a wrapper's MFBASIC-facing signature would make
    /// a private C layout part of the public API — the same argument that confines
    /// `CPtr` (`NATIVE_CPTR_ESCAPE`). plan-50-B §4.5.
    fn check_cstruct_escape(&mut self, file: &AstFile, link: &crate::ast::LinkBlock) {
        if link.cstructs.is_empty() {
            return;
        }
        let is_cstruct = |name: &str| link.cstructs.iter().any(|c| c.name == name);
        for function in &link.functions {
            for param in &function.params {
                if let Some(type_name) = &param.type_name {
                    if is_cstruct(type_name) {
                        self.report(
                            "NATIVE_CSTRUCT_ESCAPE",
                            &format!(
                                "Native function `{}` parameter `{}` uses CSTRUCT `{}`; name its mapped record type instead — a CSTRUCT is nameable only in an ABI slot or SIZEOF.",
                                function.name, param.name, type_name
                            ),
                            file,
                            param.line,
                        );
                    }
                }
            }
            if let Some(return_type) = &function.return_type {
                if is_cstruct(return_type) {
                    self.report(
                        "NATIVE_CSTRUCT_ESCAPE",
                        &format!(
                            "Native function `{}` returns CSTRUCT `{}`; name its mapped record type instead — a CSTRUCT is nameable only in an ABI slot or SIZEOF.",
                            function.name, return_type
                        ),
                        file,
                        function.line,
                    );
                }
            }
        }
    }

    /// Validate the block's `CSTRUCT` declarations (plan-50-B §4.4).
    ///
    /// Shares `ir::check_cstruct` with the package path so the two cannot drift;
    /// this side adds the per-declaration span and the duplicate-name check.
    fn check_link_cstructs(&mut self, file: &AstFile, link: &crate::ast::LinkBlock) {
        let names: Vec<String> = link.cstructs.iter().map(|c| c.name.clone()).collect();
        for (index, decl) in link.cstructs.iter().enumerate() {
            if link.cstructs[..index].iter().any(|p| p.name == decl.name) {
                self.report(
                    "NATIVE_CSTRUCT_INVALID",
                    &format!(
                        "LINK alias `{}` declares CSTRUCT `{}` more than once.",
                        link.alias, decl.name
                    ),
                    file,
                    decl.line,
                );
            }
            let fields: Vec<(String, String)> = decl
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.ctype.clone()))
                .collect();
            // Every supported target is LP64 and agrees on the layout table.
            for fault in crate::ir::check_cstruct(&decl.name, &fields, &names, "") {
                // Point at the offending field where we can; the declaration line
                // otherwise.
                let line = decl
                    .fields
                    .iter()
                    .find(|f| fault.message.contains(&format!("`{}`", f.name)))
                    .map_or(decl.line, |f| f.line);
                self.report(fault.rule, &fault.message, file, line);
            }
        }
    }

    /// `cstructs` is every `CSTRUCT` name declared in the owning `LINK` block; a
    /// slot may name one as its ctype (plan-50-E).
    pub(super) fn check_link_function_in(
        &mut self,
        file: &AstFile,
        function: &crate::ast::LinkFunction,
        cstructs: &[String],
    ) {
        // `CPtr` (and other raw C ABI types) may never appear in a wrapper's
        // MFBASIC-facing signature — only inside `ABI (...)` slots. A wrapper
        // param or return typed as a C type would let a raw pointer escape into an
        // ordinary API (plan-link-update.md §5/§11).
        for param in &function.params {
            if let Some(type_name) = &param.type_name {
                if is_c_abi_type(type_name) {
                    self.report(
                        "NATIVE_CPTR_ESCAPE",
                        &format!(
                            "Native function `{}` parameter `{}` uses C ABI type `{}`; raw C types may appear only in ABI slots.",
                            function.name, param.name, type_name
                        ),
                        file,
                        param.line,
                    );
                }
            }
        }
        if let Some(return_type) = &function.return_type {
            if is_c_abi_type(return_type) {
                self.report(
                    "NATIVE_CPTR_ESCAPE",
                    &format!(
                        "Native function `{}` returns C ABI type `{}`; raw C types may appear only in ABI slots.",
                        function.name, return_type
                    ),
                    file,
                    function.line,
                );
            }
        }

        // Every ABI slot must be satisfied by exactly one of: a wrapper parameter
        // (matched by name), the OUT/return result marker, or a CONST pin
        // (plan-link-update.md §5c).
        let const_slots: HashSet<&str> = function
            .consts
            .iter()
            .map(|pin| pin.slot.as_str())
            .collect();
        let param_names: HashSet<&str> = function
            .params
            .iter()
            .map(|param| param.name.as_str())
            .collect();

        // plan-50-A: the slot ctype namespace is closed. An unknown name used to
        // fall through to a raw 64-bit marshal in the thunk's default arm, so a
        // typo compiled clean and silently moved the wrong width.
        if !crate::ir::abi_ctype_valid_as_return(&function.abi.return_ctype) {
            self.report(
                "NATIVE_ABI_UNKNOWN_CTYPE",
                &format!(
                    "Native function `{}` ABI return `{}` uses C type `{}`, which is not a valid ABI return type.",
                    function.name, function.abi.return_name, function.abi.return_ctype
                ),
                file,
                function.abi.line,
            );
        }
        for slot in &function.abi.slots {
            // A slot may name a CSTRUCT declared in this LINK block; the struct
            // rules then apply instead of the scalar ctype table (plan-50-E).
            if cstructs.contains(&slot.ctype) {
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
                self.report(
                    "NATIVE_ABI_UNKNOWN_CTYPE",
                    &format!(
                        "Native function `{}` ABI slot `{}` uses C type `{}`, which is not valid in that position.",
                        function.name, slot.name, slot.ctype
                    ),
                    file,
                    slot.line,
                );
            }
        }

        // plan-50-H: the result is named by `RETURN <expr>`. Both magic-name
        // checks are gone — a slot named `return` no longer parses, and the ABI
        // return is an ordinary name.
        for slot in &function.abi.slots {
            // A CONST pin satisfies the slot and is input-only.
            if const_slots.contains(slot.name.as_str()) {
                if slot.direction.writes_back() {
                    self.report(
                        "NATIVE_CONST_OUT",
                        &format!(
                            "Native function `{}` pins ABI slot `{}` with CONST, which cannot also be OUT.",
                            function.name, slot.name
                        ),
                        file,
                        slot.line,
                    );
                }
                continue;
            }
            // An OUT slot is native storage the callee fills; it needs no wrapper
            // parameter. It is surfaced (if at all) by naming it in `RETURN`.
            if slot.direction.writes_back() {
                continue;
            }
            // An IN struct slot is satisfied by its `BIND IN` block: its fields
            // carry the inputs, and everything unbound is zero (plan-50-E).
            if function.bind_in.iter().any(|b| b.slot == slot.name) {
                continue;
            }
            // An ordinary input slot must bind to a wrapper parameter by name.
            if !param_names.contains(slot.name.as_str()) {
                self.report(
                    "NATIVE_ABI_UNBOUND_SLOT",
                    &format!(
                        "Native function `{}` ABI slot `{}` does not bind to a parameter, CONST pin, or an OUT buffer.",
                        function.name, slot.name
                    ),
                    file,
                    slot.line,
                );
            }
        }

        // plan-50-I: an identifier in a SUCCESS_ON/ERROR_ON/RETURN expression must
        // name a real ABI slot (or the ABI return). Before I, `lower_link_expr`
        // mapped EVERY identifier onto one nameless "native return" variable, so
        // `SUCCESS_ON typo = 0` silently meant `status = 0`, and an expression
        // could not read any other slot despite the spec saying it could.
        {
            let mut names: Vec<String> = Vec::new();
            for expr in [&function.success_on, &function.result]
                .into_iter()
                .flatten()
            {
                link_expr_idents(expr, &mut names);
            }
            for name in names {
                // `NOTHING` is a literal, not a slot.
                if name == "NOTHING"
                    || name == function.abi.return_name
                    || function.abi.slots.iter().any(|slot| slot.name == name)
                {
                    continue;
                }
                self.report(
                    "NATIVE_ABI_UNBOUND_SLOT",
                    &format!(
                        "Native function `{}` SUCCESS_ON/RETURN expression reads `{name}`, which is not an ABI slot or the ABI return.",
                        function.name
                    ),
                    file,
                    function.abi.line,
                );
            }
        }

        // A producer (`AS RES X`) and any non-Nothing value-returning wrapper must
        // surface exactly one result; a `Nothing` wrapper surfaces none.
        let wants_result = function.return_resource
            || function
                .return_type
                .as_deref()
                .is_some_and(|return_type| return_type != "Nothing");
        if wants_result && function.result.is_none() {
            self.report(
                "NATIVE_ABI_NO_RESULT",
                &format!(
                    "Native function `{}` returns a value but declares no `RETURN <expr>` naming its result.",
                    function.name
                ),
                file,
                function.line,
            );
        }
        // A `Nothing` wrapper surfaces no value, so a RETURN has nothing to name.
        if !wants_result && function.result.is_some() {
            self.report(
                "NATIVE_ABI_RESULT_MARKER",
                &format!(
                    "Native function `{}` returns Nothing but declares a `RETURN`.",
                    function.name
                ),
                file,
                function.line,
            );
        }

        // Every wrapper parameter must be consumed: by an ABI slot of the same
        // name, by a `BIND IN` field that binds it (plan-50-E — a parameter
        // feeding a struct field has no slot of its own), or by a `BUFFER … SIZE`
        // expression (plan-58-B — a parameter that only sizes an OUT CBuffer,
        // e.g. `BUFFER buf SIZE pairs * 2`, likewise has no slot of its own).
        for param in &function.params {
            let by_slot = function.abi.slots.iter().any(|s| s.name == param.name);
            let by_bind = function.bind_in.iter().any(|b| {
                b.fields.iter().any(|f| {
                    matches!(&f.value, crate::ast::Expression::Identifier(n) if *n == param.name)
                })
            });
            let by_buffer_size = function.buffers.iter().any(|b| {
                let mut names = Vec::new();
                link_expr_idents(&b.size, &mut names);
                names.contains(&param.name)
            });
            if !by_slot && !by_bind && !by_buffer_size {
                self.report(
                    "NATIVE_ABI_UNBOUND_PARAM",
                    &format!(
                        "Native function `{}` parameter `{}` has no matching ABI slot and no BIND IN field.",
                        function.name, param.name
                    ),
                    file,
                    param.line,
                );
            }
        }

        // plan-50-G: a CONST pin must fold to an immediate. Until now an
        // unrecognized expression silently pinned **0** (`eval_link_const`'s
        // `_ => 0`) — the same "default rather than diagnose" mistake as the
        // unvalidated slot ctype and the nameless link-expr Var. This is the gate
        // that makes that catch-all unreachable.
        for pin in &function.consts {
            fn foldable(expr: &crate::ast::Expression, cstructs: &[String]) -> bool {
                match expr {
                    crate::ast::Expression::Number(_) | crate::ast::Expression::Boolean(_) => true,
                    crate::ast::Expression::Identifier(name) => name == "NOTHING",
                    crate::ast::Expression::Unary {
                        operator, operand, ..
                    } if operator == "SIZEOF" => matches!(
                        operand.as_ref(),
                        crate::ast::Expression::Identifier(n) if cstructs.contains(n)
                    ),
                    crate::ast::Expression::Unary {
                        operator, operand, ..
                    } if operator == "-" || operator == "+" => foldable(operand, cstructs),
                    _ => false,
                }
            }
            if !foldable(&pin.value, cstructs) {
                self.report(
                    "NATIVE_CONST_UNKNOWN_SLOT",
                    &format!(
                        "Native function `{}` CONST pin `{}` is not a constant the compiler can fold: it must be an integer or boolean literal, NOTHING, or SIZEOF <CStruct>.",
                        function.name, pin.slot
                    ),
                    file,
                    pin.line,
                );
            }
        }

        // A CONST pin must name a real ABI slot.
        let abi_slot_names: HashSet<&str> = function
            .abi
            .slots
            .iter()
            .map(|slot| slot.name.as_str())
            .collect();
        for pin in &function.consts {
            if !abi_slot_names.contains(pin.slot.as_str()) {
                self.report(
                    "NATIVE_CONST_UNKNOWN_SLOT",
                    &format!(
                        "Native function `{}` CONST pins unknown ABI slot `{}`.",
                        function.name, pin.slot
                    ),
                    file,
                    pin.line,
                );
            }
        }

        // A FREE block releases a caller-owned native return after it is copied
        // out (mfbasic.md §17). The implemented form frees the produced CPtr —
        // the C return, named by `RETURN` — through a deallocator that takes one
        // CPtr and returns CVoid (e.g. `sqlite3_free`). Anything else is rejected.
        if let Some(free) = &function.free {
            let mut ok = true;
            // plan-50-H: `FREE <slot>` names the real slot rather than the magic
            // `return`. The freed slot must be the C return, and that return must
            // be what `RETURN` surfaces — freeing a value the wrapper never
            // produced would release a pointer nothing copied.
            let returns_the_c_value = matches!(
                &function.result,
                Some(crate::ast::Expression::Identifier(name)) if *name == function.abi.return_name
            );
            if free.slot != function.abi.return_name || !returns_the_c_value {
                ok = false;
            }
            // That return must be a CPtr copied into an owned wrapper value.
            if function.abi.return_ctype != "CPtr" {
                ok = false;
            }
            // The deallocator: one pointer parameter, void return.
            if free.param_ctype != "CPtr" || free.return_ctype != "CVoid" {
                ok = false;
            }
            if free.symbol.is_empty() {
                ok = false;
            }
            if !ok {
                self.report(
                    "NATIVE_FREE_INVALID",
                    &format!(
                        "Native function `{}` has a malformed FREE block: it must name the CPtr produced slot that `RETURN` surfaces, and its deallocator must take one CPtr parameter and return CVoid.",
                        function.name
                    ),
                    file,
                    free.line,
                );
            }
        }
    }

    /// Register native `LINK` function signatures (keyed `alias.func`) and any
    /// `FUNC alias AS alias::func` re-exports, so wrapper code that calls
    /// `sqliteLink::open(...)` or importers that call `sqlite::close(...)` get a
    /// type (plan-link-update.md §5a/§5b).
    pub(super) fn collect_native_functions(&mut self) {
        // First gather every LINK function's signature so aliases can adopt them.
        let mut link_sigs: HashMap<String, (FunctionSig, String)> = HashMap::new();
        for file in &self.ast.files {
            for item in &file.items {
                let Item::Link(link) = item else {
                    continue;
                };
                for function in &link.functions {
                    let sig = self.native_function_sig(function, &file.path);
                    let key = format!("{}.{}", link.alias, function.name);
                    self.functions
                        .entry(key.clone())
                        .or_default()
                        .push(sig.clone());
                    link_sigs.insert(key, (sig, file.path.clone()));
                }
            }
        }

        // Then register re-export aliases, adopting the target's signature with
        // the alias's declared visibility (plan-link-update.md §5a).
        for file in &self.ast.files {
            for item in &file.items {
                let Item::FuncAlias(alias) = item else {
                    continue;
                };
                if let Some((sig, _)) = link_sigs.get(&alias.target) {
                    let mut adopted = sig.clone();
                    adopted.visibility = alias.visibility;
                    adopted.owner_file_path = file.path.clone();
                    self.functions
                        .entry(alias.name.clone())
                        .or_default()
                        .push(adopted);
                }
            }
        }
    }

    pub(super) fn native_function_sig(
        &self,
        function: &crate::ast::LinkFunction,
        owner_file_path: &str,
    ) -> FunctionSig {
        let return_type = function
            .return_type
            .as_deref()
            .map(|name| self.parse_type(name))
            .unwrap_or(Type::Nothing);
        let params = function
            .params
            .iter()
            .map(|param| ParamSig {
                name: param.name.clone(),
                type_: param
                    .type_name
                    .as_deref()
                    .map(|name| self.parse_type(name))
                    .unwrap_or(Type::Unknown),
                has_default: param.default.is_some(),
            })
            .collect();
        FunctionSig {
            kind: FunctionKind::Func,
            params,
            return_type,
            isolated: false,
            imported_package_export: false,
            // A LINK block is package-local; its functions are reachable from any
            // file of the declaring package via the alias namespace.
            visibility: Visibility::Public,
            owner_file_path: owner_file_path.to_string(),
        }
    }
}
