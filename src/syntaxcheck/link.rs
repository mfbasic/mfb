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
        for file in &self.hir.files {
            for item in &file.items {
                if let HirItem::Link(link) = item {
                    for function in &link.functions {
                        close_may_fail.insert(
                            format!("{}.{}", link.alias, function.name),
                            function.success_on.is_some(),
                        );
                    }
                }
            }
        }

        for file in &self.hir.files {
            for item in &file.items {
                if let HirItem::Resource(resource) = item {
                    let close_function = resource.close_fn.clone();
                    let may_fail = close_may_fail
                        .get(&close_function)
                        .copied()
                        .unwrap_or(false);
                    self.resource_registry.register(
                        resource.name.clone(),
                        crate::codegen::resource::ResourceInfo {
                            close_function,
                            sendable: resource.thread_sendable,
                            close_may_fail: may_fail,
                            kind: crate::codegen::resource::ResourceKind::Native,
                        },
                    );
                }
            }
        }
    }

    /// Native-specific checks on a `LINK` block: `CPtr` containment and ABI
    /// slot/parameter consistency (plan-link-update.md §5b/§5c/§11/§12).
    pub(super) fn check_link_block(&mut self, file: &HirFile, link: &crate::ast::LinkBlock) {
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
    fn check_buffer_slots(&mut self, file: &HirFile, function: &crate::ast::LinkFunction) {
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
        for file in &self.hir.files {
            for item in &file.items {
                if let HirItem::Type(decl) = item {
                    if decl.name == name && decl.kind == crate::ast::TypeDeclKind::Type {
                        return Some(
                            decl.fields
                                .iter()
                                // `ir::check_struct_slot` compares a CSTRUCT field's
                                // C type against the record field's declared SPELLING,
                                // which is the ABI seam's own vocabulary.
                                .map(|f| (f.name.clone(), f.type_.name().into_owned()))
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
        file: &HirFile,
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
            // (Returning an IN slot is the result-marker rule, `ir::verify`'s
            // since plan-107-C.)
            if matches!(&function.result, Some(crate::ast::Expression::Identifier(n)) if *n == slot.name)
            {
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
    fn check_cstruct_escape(&mut self, file: &HirFile, link: &crate::ast::LinkBlock) {
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
    fn check_link_cstructs(&mut self, file: &HirFile, link: &crate::ast::LinkBlock) {
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
        file: &HirFile,
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

        // The result-marker rules — a value-returning wrapper with no
        // `RETURN <expr>`, a `Nothing` wrapper with one — are `ir::verify`'s
        // (plan-107-C).

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
        // out (17_native-libraries.md). The implemented form frees the produced CPtr —
        // the C return, named by `RETURN` — through a deallocator that takes one
        // CPtr and returns CVoid (e.g. `sqlite3_free`). Anything else is rejected.
        if let Some(free) = &function.free {
            // sec-01: `FREE` is a copy-then-free mechanism for a caller-owned value
            // that the wrapper copies out (a `String`/buffer). An `AS RES` producer
            // instead keeps the native handle alive by storing it into the resource
            // record's `FD@0`; freeing it in the thunk would leave every later use a
            // use-after-free and the scope-drop `CLOSE BY` a double-free. The
            // combination is semantically contradictory, so reject it outright.
            if function.return_resource {
                self.report(
                    "NATIVE_FREE_INVALID",
                    &format!(
                        "Native function `{}` declares a FREE block on an `AS RES` resource producer: a resource producer keeps the native handle alive in its record and must not free it (FREE is only for a caller-owned value copied out of the return).",
                        function.name
                    ),
                    file,
                    free.line,
                );
                return;
            }
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
        for file in &self.hir.files {
            for item in &file.items {
                let HirItem::Link(link) = item else {
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
        for file in &self.hir.files {
            for item in &file.items {
                let HirItem::FuncAlias(alias) = item else {
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

#[cfg(test)]
mod tests {
    use crate::testutil::*;

    // Every fixture is a whole program ending in a valid `FUNC main`. The
    // LINK/CSTRUCT/ABI shapes mirror `tests/rt-behavior/native/*/src/main.mfb`
    // and the `demoLink` pattern in `helpers.rs`. Each `#[test]` drives one
    // `self.report(...)` diagnostic in this file (or an `accepts` path that
    // exercises the branch's success arms).

    // ----- RESOURCE built-in shadow ------------------------------------------

    #[test]
    fn user_resource_named_like_a_builtin_is_accepted() {
        // plan-97 / bug-441: builtin resources are package-qualified (`fs::File`), so a
        // user `RESOURCE File` (bare) names a distinct user resource and is accepted.
        // The RESOURCE_SHADOWS_BUILTIN rule that once drove this test could not fire
        // after that change and was retired (plan-107-B).
        let src = "\
RESOURCE File CLOSE BY demoLink::close

LINK \"demo\" AS demoLink
  FUNC close(RES f AS File) AS Nothing
    SYMBOL \"demo_close\"
    ABI (f CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- collect_native_resources register path (clean, accepts) ----------

    #[test]
    fn clean_native_resource_and_link_accepts() {
        // A user RESOURCE closed by a LINK function with SUCCESS_ON, plus a
        // producer, registers cleanly (close_may_fail = true from success_on).
        let src = "\
RESOURCE Db CLOSE BY demoLink::close

LINK \"demo\" AS demoLink
  FUNC open(path AS String) AS RES Db
    SYMBOL \"demo_open\"
    ABI (path CString, db OUT CPtr) AS status CInt32
    RETURN db
    SUCCESS_ON status = 0
  END FUNC

  FUNC close(RES db AS Db) AS Nothing
    SYMBOL \"demo_close\"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- CSTRUCT declarations / escape ------------------------------------

    #[test]
    fn duplicate_cstruct_name_is_rejected() {
        // link.rs:406 — a LINK block declaring the same CSTRUCT name twice.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_CSTRUCT_INVALID"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn cstruct_named_in_wrapper_param_is_rejected() {
        // link.rs:370 — a wrapper parameter typed as a declared CSTRUCT.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f(x AS Foo) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_CSTRUCT_ESCAPE"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn cstruct_named_in_wrapper_return_is_rejected() {
        // link.rs:384 — a wrapper returning a declared CSTRUCT.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Foo
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_CSTRUCT_ESCAPE"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn cstruct_bad_field_ctype_forwards_check_cstruct_fault() {
        // link.rs:421-430 — a CSTRUCT field using an unknown ctype forwards a
        // crate::ir::check_cstruct fault, pointed at the field line.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CBogus
  END CSTRUCT
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_ABI_UNKNOWN_CTYPE"),
            "{:?}",
            check_src(src)
        );
    }

    // ----- struct slots / BIND IN -------------------------------------------

    #[test]
    fn inout_non_struct_slot_is_rejected() {
        // link.rs:196 — a scalar slot marked INOUT whose ctype is not a CSTRUCT.
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a INOUT CInt64) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_ABI_UNKNOWN_CTYPE"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn cstruct_mapping_to_non_record_is_rejected() {
        // link.rs:210 — a CSTRUCT whose MAPS target is an ENUM, not a record,
        // drives record_fields_of -> None.
        let src = "\
ENUM Color
  Red
  Green
END ENUM

LINK \"c\" AS libc
  CSTRUCT Foo AS Color
    a CInt64
  END CSTRUCT
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (s OUT Foo) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_STRUCT_FIELD_MISMATCH"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn returning_an_in_struct_slot_is_rejected() {
        // link.rs:240 — a wrapper that RETURNs a struct slot declared IN.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Rec
    SYMBOL \"f\"
    ABI (s IN Foo) AS status CInt32
    BIND IN s
      a = 0
    END BIND
    RETURN s
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn returning_struct_slot_with_wrong_return_type_is_rejected() {
        // link.rs:251 — returns a struct slot but the wrapper return type is not
        // the CSTRUCT's mapped record.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

TYPE Other
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Other
    SYMBOL \"f\"
    ABI (s OUT Foo) AS status CInt32
    RETURN s
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_STRUCT_FIELD_MISMATCH"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn cstruct_record_field_type_disagreement_forwards_fault() {
        // link.rs:232 — check_struct_slot fault: CStruct field maps to Integer
        // but the record declares it String.
        let src = "\
TYPE Rec
  a AS String
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Rec
    SYMBOL \"f\"
    ABI (s OUT Foo) AS status CInt32
    RETURN s
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_STRUCT_FIELD_MISMATCH"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn bind_in_unknown_slot_is_rejected() {
        // link.rs:268 — BIND IN names an ABI slot that does not exist.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (s IN Foo) AS status CInt32
    BIND IN nonesuch
      a = 0
    END BIND
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_BIND_IN_INVALID"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn bind_in_non_struct_slot_is_rejected() {
        // link.rs:280 — BIND IN names a slot whose ctype is not a CSTRUCT.
        let src = "\
LINK \"c\" AS libc
  FUNC f(n AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (n CInt64) AS status CInt32
    BIND IN n
      a = 0
    END BIND
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_BIND_IN_INVALID"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn bind_in_out_slot_is_rejected() {
        // link.rs:292 — BIND IN writes an OUT slot.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Rec
    SYMBOL \"f\"
    ABI (s OUT Foo) AS status CInt32
    BIND IN s
      a = 0
    END BIND
    RETURN s
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_BIND_IN_INVALID"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn bind_in_unknown_field_is_rejected() {
        // link.rs:305 — BIND IN sets a field the CSTRUCT does not declare.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (s IN Foo) AS status CInt32
    BIND IN s
      nosuchfield = 0
    END BIND
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_BIND_IN_INVALID"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn bind_in_duplicate_field_is_rejected() {
        // link.rs:316 — BIND IN sets the same field twice.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (s IN Foo) AS status CInt32
    BIND IN s
      a = 0
      a = 1
    END BIND
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_BIND_IN_INVALID"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn bind_in_bad_value_is_rejected() {
        // link.rs:342 — BIND IN sets a field from a string literal (neither a
        // wrapper param nor an int/bool/-int literal).
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (s IN Foo) AS status CInt32
    BIND IN s
      a = \"hello\"
    END BIND
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_BIND_IN_INVALID"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn clean_bind_in_value_shapes_accept() {
        // link.rs:327-339 — a clean BIND IN whose values exercise the Identifier
        // (param), Number, Unary "-", and Boolean ok=true arms.
        let src = "\
TYPE Rec
  a AS Integer
  b AS Integer
  c AS Integer
  d AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
    b CInt64
    c CInt64
    d CInt64
  END CSTRUCT
  FUNC f(p AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (s IN Foo, rem CPtr) AS status CInt32
    CONST rem = NOTHING
    BIND IN s
      a = p
      b = 5
      c = -3
      d = TRUE
    END BIND
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- C ABI escape / ctype validity ------------------------------------

    #[test]
    fn wrapper_cptr_param_and_return_escape() {
        // link.rs:450/463 — raw C ABI types in the wrapper's MFBASIC-facing
        // signature (param and return arms of is_c_abi_type).
        let src = "\
LINK \"demo\" AS demoLink
  FUNC leak(handle AS CPtr) AS Nothing
    SYMBOL \"demo_leak\"
    ABI (handle CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
  FUNC produce() AS CPtr
    SYMBOL \"demo_produce\"
    ABI (out OUT CPtr) AS status CInt32
    RETURN out
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_CPTR_ESCAPE"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn bad_return_ctype_is_rejected() {
        // link.rs:494 — an ABI return ctype not in the closed table.
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CBogus
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_ABI_UNKNOWN_CTYPE"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn bad_slot_ctype_both_directions_are_rejected() {
        // link.rs:511-525 — an unknown ctype in argument position (valid_as_argument)
        // and on an OUT slot (valid_as_return arm).
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CBogus, b OUT CBogus) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_ABI_UNKNOWN_CTYPE"),
            "{:?}",
            check_src(src)
        );
    }

    // ----- CONST pins -------------------------------------------------------

    #[test]
    fn const_pin_on_out_slot_is_rejected() {
        // link.rs:537 — CONST pinning a slot that is also OUT.
        let src = "\
LINK \"c\" AS libc
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (s OUT CInt64) AS status CInt32
    CONST s = 5
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_CONST_OUT"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn non_foldable_const_value_is_rejected() {
        // link.rs:690 — a CONST whose value is an arbitrary identifier.
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64, flag CInt64) AS status CInt32
    CONST flag = bogusIdent
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_CONST_UNKNOWN_SLOT"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn clean_const_foldable_shapes_accept() {
        // link.rs:672-687 — SIZEOF <CStruct>, boolean literal, NOTHING, and a
        // negated integer all fold (the foldable=true arms).
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (sz CInt64, b CInt64, n CPtr, neg CInt64, s IN Foo) AS status CInt32
    CONST sz = SIZEOF Foo
    CONST b = TRUE
    CONST n = NOTHING
    CONST neg = -1
    BIND IN s
      a = 0
    END BIND
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn const_pin_on_unknown_slot_is_rejected() {
        // link.rs:711 — a CONST pinning a slot name not in the ABI.
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    CONST nosuch = 5
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_CONST_UNKNOWN_SLOT"),
            "{:?}",
            check_src(src)
        );
    }

    // ----- unbound slots / params, result markers ---------------------------

    #[test]
    fn unbound_input_slot_is_rejected() {
        // link.rs:561 — an input ABI slot with no matching parameter/CONST/OUT/BIND.
        let src = "\
LINK \"c\" AS libc
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_ABI_UNBOUND_SLOT"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn success_on_reading_unknown_name_is_rejected() {
        // link.rs:594 — a SUCCESS_ON expression reading an identifier that names
        // no ABI slot and is not the ABI return.
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    SUCCESS_ON typo = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_ABI_UNBOUND_SLOT"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn nothing_wrapper_with_return_is_rejected() {
        // link.rs:626 — a Nothing wrapper that declares a RETURN.
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    RETURN status
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn unbound_wrapper_parameter_is_rejected() {
        // link.rs:655 — a wrapper parameter with no matching ABI slot, BIND IN
        // field, or BUFFER SIZE use.
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer, orphan AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_ABI_UNBOUND_PARAM"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn clean_cbuffer_wrappers_accept() {
        // link.rs:99-158 + 648-652 — a clean CBuffer program: check_buffer_slots
        // runs with zero faults, and `pairs` (SIZE-only) drives by_buffer_size.
        let src = "\
LINK \"c\" AS libc
  FUNC preadBytes(fd AS Integer, nbyte AS Integer, offset AS Integer) AS List OF Byte
    SYMBOL \"pread\"
    ABI (fd CInt32, buf OUT CBuffer, nbyte CInt64, offset CInt64) AS got CInt64
    BUFFER buf SIZE nbyte
    RETURN buf LENGTH got
  END FUNC
  FUNC preadPairs(fd AS Integer, nbyte AS Integer, pairs AS Integer, offset AS Integer) AS List OF Byte
    SYMBOL \"pread\"
    ABI (fd CInt32, buf OUT CBuffer, nbyte CInt64, offset CInt64) AS got CInt64
    BUFFER buf SIZE pairs * 2
    RETURN buf LENGTH got
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn cbuffer_without_return_hits_result_slot_none_arm() {
        // A buffer function with no RETURN drives the `_ => None` result_slot arm
        // of the buffer-rule view (the missing-result rejection itself is
        // `ir::verify`'s since plan-107-C, so only the walk is asserted here).
        let src = "\
LINK \"c\" AS libc
  FUNC noReturn(n AS Integer) AS List OF Byte
    SYMBOL \"f\"
    ABI (buf OUT CBuffer) AS got CInt64
    BUFFER buf SIZE n
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        let _ = check_src(src);
    }

    // ----- FREE blocks ------------------------------------------------------

    #[test]
    fn free_on_resource_producer_is_rejected() {
        // link.rs:735 (+ early return :743) — a FREE block on an `AS RES` producer.
        let src = "\
RESOURCE Db CLOSE BY libc::close

LINK \"sqlite3\" AS libc
  FUNC open(path AS String) AS RES Db
    SYMBOL \"sqlite3_open\"
    ABI (path CString, db OUT CPtr) AS status CInt32
    RETURN db
    FREE db
      SYMBOL \"sqlite3_free\"
      ABI (ptr CPtr) AS CVoid
    END FREE
    SUCCESS_ON status = 0
  END FUNC
  FUNC close(RES db AS Db) AS Nothing
    SYMBOL \"sqlite3_close\"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_FREE_INVALID"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn malformed_free_block_is_rejected() {
        // link.rs:770 — a FREE whose deallocator does not return CVoid.
        let src = "\
LINK \"sqlite3\" AS sql
  FUNC expandedSql(stmt AS Integer) AS String
    SYMBOL \"sqlite3_expanded_sql\"
    ABI (stmt CInt64) AS text CPtr
    RETURN text
    FREE text
      SYMBOL \"sqlite3_free\"
      ABI (ptr CPtr) AS CInt32
    END FREE
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_FREE_INVALID"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn well_formed_free_block_accepts() {
        // link.rs:745-767 — a well-formed FREE (CPtr return surfaced by RETURN,
        // deallocator taking one CPtr and returning CVoid) drives the ok=true path.
        let src = "\
LINK \"sqlite3\" AS sql
  FUNC expandedSql(stmt AS Integer) AS String
    SYMBOL \"sqlite3_expanded_sql\"
    ABI (stmt CInt64) AS text CPtr
    RETURN text
    FREE text
      SYMBOL \"sqlite3_free\"
      ABI (ptr CPtr) AS CVoid
    END FREE
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- re-exports / signatures ------------------------------------------

    #[test]
    fn func_alias_reexport_adopts_link_signature() {
        // link.rs:808-823 + 826-860 — a `FUNC open AS demoLink::open` alias adopts
        // the LINK signature; a bare `demoLink::open(...)` call drives
        // native_function_sig's param/return parse_type mapping.
        let src = "\
RESOURCE Db CLOSE BY demoLink::close

LINK \"demo\" AS demoLink
  FUNC open(path AS String) AS RES Db
    SYMBOL \"demo_open\"
    ABI (path CString, db OUT CPtr) AS status CInt32
    RETURN db
    SUCCESS_ON status = 0
  END FUNC
  FUNC close(RES db AS Db) AS Nothing
    SYMBOL \"demo_close\"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC open AS demoLink::open

FUNC main AS Integer
  RES db AS Db = open(\":memory:\")
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }
}
