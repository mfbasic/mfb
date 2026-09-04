// --- codegen tier imports (migration) ---
use crate::codegen::builtins;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::typed_map_entry_type_parts;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::operators::{BinaryOp, UnaryOp};
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;
impl CodeBuilder<'_> {
    /// Resolve a resource *value* pointer (held in `value_ptr`) of declared
    /// `type_` to the pointer to its resource **record**, whose `STATE`
    /// slot is at `RESOURCE_OFFSET_STATE` (24, free in every backend layout —
    /// plan-80). For a concrete resource the value already
    /// IS the record. For a resource union (plan-74) the value is a
    /// `{ tag @0, record-ptr @8 }` block, so the record is loaded from `+8` — the
    /// single indirection that makes every STATE path work for a union.
    pub(crate) fn emit_resource_record_ptr(
        &mut self,
        value_ptr: impl Into<Operand>,
        type_: &ParameterType,
    ) -> Result<String, String> {
        if self.is_resource_union_type(type_) {
            let record = self.allocate_register();
            self.emit(abi::load_u64(&record, value_ptr, 8));
            Ok(record.render())
        } else {
            Ok(value_ptr.into().render())
        }
    }

    /// Default-initialize a `RES` binding's `STATE` payload. The resource value
    /// at `resource_slot` is a pointer to its record (a concrete resource) or to a
    /// `{tag, record-ptr}` union block whose record is at `+8` (`resource_type`
    /// selects, plan-74); if the state slot (`RESOURCE_OFFSET_STATE`) of the record is
    /// null, allocate and store a default `state_type` record. A resource that
    /// already carries state (moved/returned in) is left untouched. Values are
    /// spilled to the stack across allocations to avoid register aliasing.
    pub(crate) fn emit_resource_state_init(
        &mut self,
        resource_slot: usize,
        state_type: &ParameterType,
        resource_type: &ParameterType,
    ) -> Result<(), String> {
        let block = self.allocate_register();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), resource_slot));
        let ptr = self.emit_resource_record_ptr(&block, resource_type)?;
        let current = self.allocate_register();
        self.emit(abi::load_u64(&current, &ptr, RESOURCE_OFFSET_STATE));
        let done = self.label("resource_state_init_done");
        self.emit(abi::compare_immediate(&current, "0"));
        self.emit(abi::branch_ne(&done));
        let default = self.lower_default_value(state_type)?;
        let default_slot = self.allocate_stack_object("resource_state_default", 8);
        self.emit(abi::store_u64(
            &default.location,
            abi::stack_pointer(),
            default_slot,
        ));
        let block2 = self.allocate_register();
        self.emit(abi::load_u64(&block2, abi::stack_pointer(), resource_slot));
        let ptr2 = self.emit_resource_record_ptr(&block2, resource_type)?;
        let value = self.allocate_register();
        self.emit(abi::load_u64(&value, abi::stack_pointer(), default_slot));
        self.emit(abi::store_u64(&value, &ptr2, RESOURCE_OFFSET_STATE));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Materialize a fresh CLOSED resource record: an arena record zeroed
    /// (invalid internals) with its shared `RESOURCE_OFFSET_CLOSED` (16) flag
    /// set. This is the record a `RES x = <fallible> TRAP` error path binds when
    /// it needs a resource value it can never re-open — every later op then
    /// short-circuits safely (`close` is an idempotent no-op; `read`/`write`/…
    /// raise via their closed guard), and no null handle is ever exposed. The
    /// caller attaches STATE (a concrete resource IS this record; a resource
    /// union wraps it at `+8`), so this returns the bare record register.
    fn emit_closed_resource_record(&mut self) -> Result<VirtualRegister, String> {
        let record = self.allocate_register();
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            RESOURCE_RECORD_SIZE,
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_symbol_call(ARENA_ALLOC_SYMBOL);
        let alloc_ok = self.label("default_resource_alloc_ok");
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::move_register(&record, abi::mfb_return(1)));
        // Zero the record (invalid internals), then mark it closed.
        let bytes: usize = RESOURCE_RECORD_SIZE_BYTES;
        let mut offset = 0;
        while offset < bytes {
            self.emit(abi::store_u64(abi::ZERO, &record, offset));
            offset += 8;
        }
        let one = self.allocate_register();
        self.emit(abi::move_immediate(&one, "Integer", "1"));
        // The single canonical closed-flag offset, shared by every built-in
        // resource record (enforced by the compile-time asserts beside each
        // per-resource closed-offset constant).
        self.emit(abi::store_u64(&one, &record, RESOURCE_OFFSET_CLOSED));
        Ok(record)
    }

    /// Materialize the default value of `type_`. The site that needs one is the
    /// never-observed `bind $trap_valN : T = <default>` temp the inline-`TRAP`
    /// desugar emits for a fallible call (plus `STATE` payload init and the
    /// MATCH-default paths in `builder_control`).
    pub(crate) fn lower_default_value(
        &mut self,
        type_: &ParameterType,
    ) -> Result<ValueResult, String> {
        self.lower_default_value_inner(type_, &mut Vec::new())
    }

    /// `defaulting_unions` carries the data unions currently being defaulted up
    /// the recursion (union -> variant record -> field -> union ...), so a
    /// self-reachable union picks a variant that does not loop back into it —
    /// and codegen cannot recurse forever on a pathological type.
    fn lower_default_value_inner(
        &mut self,
        type_: &ParameterType,
        defaulting_unions: &mut Vec<ParameterType>,
    ) -> Result<ValueResult, String> {
        match type_ {
            ParameterType::Nothing => {
                let register = self.allocate_register();
                self.emit(abi::move_immediate(&register, "Integer", "0"));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register.render()),
                    text: "default Nothing".to_string(),
                })
            }
            ParameterType::Boolean => {
                let register = self.allocate_register();
                self.emit(abi::move_immediate(&register, "Boolean", "0"));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register.render()),
                    text: "default Boolean".to_string(),
                })
            }
            __t if matches!(
                __t,
                ParameterType::Byte
                    | ParameterType::Integer
                    | ParameterType::Float
                    | ParameterType::Fixed
                    | ParameterType::Money
            ) || __t.is_named("Scalar") =>
            {
                let register = self.allocate_register();
                self.emit(abi::move_immediate(
                    &register,
                    &abi::immediate_class(type_),
                    "0",
                ));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register.render()),
                    text: format!("default {type_}"),
                })
            }
            ParameterType::String => {
                let register = self.load_empty_string_constant()?;
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register.render()),
                    text: "default String".to_string(),
                })
            }
            _ if self
                .type_model
                .enum_members
                .keys()
                .any(|(enum_type, _)| enum_type == type_) =>
            {
                // An enum value IS its ordinal at run time, so its default is
                // ordinal 0 — the first declared variant — exactly as `Integer`'s
                // default is 0. Without this arm, ANY enum-typed binding reached
                // through an inline `TRAP` fails to build ("cannot materialize
                // default value"), and so does any record carrying an enum field,
                // because the record arm below defaults each field in turn. As with
                // every other default here, the value is superseded by the
                // `RECOVER` value (or handler divergence) on the taken error path,
                // so no program observes it.
                let register = self.allocate_register();
                self.emit(abi::move_immediate(&register, "Integer", "0"));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register.render()),
                    text: format!("default {type_}"),
                })
            }
            _ if typed_is_collection_type(type_) => {
                let result = self.lower_empty_collection(&type_.clone())?;
                Ok(ValueResult {
                    origin: None,
                    type_: result.type_,
                    location: result.location,
                    text: format!("default {type_}"),
                })
            }
            _ if self.is_resource_union_type(type_) => {
                // A resource UNION has no reconstructible default either; the site
                // that needs one is the error-path binding of a
                // `RES x = <fallible> TRAP` whose result is a resource union
                // (`Stream STATE PendingState`, bug-429). Return a real
                // `{tag@0, record-ptr@8}` union value whose record is CLOSED, so
                // its tag-dispatched drop is a safe no-op and a `RECOVER`ed
                // union's `.state`/`MATCH` reads a valid (closed) record rather
                // than dereferencing null. Which variant tag is used is
                // immaterial — the record is closed, so every variant's close op
                // short-circuits on the shared closed flag.
                let record = self.emit_closed_resource_record()?;
                let record_slot = self.allocate_stack_object("default_union_record", 8);
                self.emit(abi::store_u64(&record, abi::stack_pointer(), record_slot));
                // The union block: `{tag@0, record-ptr@8}`, 16 bytes.
                let block = self.allocate_register();
                self.emit(abi::move_immediate(abi::return_register(), "Integer", "16"));
                self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
                self.emit_arena_alloc_call();
                let alloc_ok = self.label("default_union_alloc_ok");
                self.emit(abi::branch_eq(&alloc_ok));
                self.raise_error_bare("ErrOutOfMemory")?;
                self.emit(abi::label(&alloc_ok));
                self.emit(abi::move_register(&block, abi::mfb_return(1)));
                let variants = self.resource_union_cleanup(type_).ok_or_else(|| {
                    format!("native code cannot resolve resource-union variants for '{type_}'")
                })?;
                let (tag, _) = variants.first().ok_or_else(|| {
                    format!("resource union '{type_}' has no variants for a default value")
                })?;
                let tag_register = self.allocate_register();
                self.emit(abi::move_immediate(
                    &tag_register,
                    abi::IMMEDIATE_CLASS_UNION_TAG,
                    &tag.to_string(),
                ));
                self.emit(abi::store_u64(&tag_register, &block, 0));
                let record_reg = self.allocate_register();
                self.emit(abi::load_u64(record_reg, abi::stack_pointer(), record_slot));
                self.emit(abi::store_u64(&record_reg, &block, 8));
                // Initialize the active variant record's uniform STATE through the
                // union value (`emit_resource_record_ptr` derefs `+8`), so a
                // `RECOVER`ed union's `.state` never dereferences null — the same
                // guarantee the concrete branch gives.
                if let Some(state) = type_.state() {
                    let block_slot = self.allocate_stack_object("default_union_block", 8);
                    self.emit(abi::store_u64(&block, abi::stack_pointer(), block_slot));
                    self.emit_resource_state_init(block_slot, &state, type_)?;
                    self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
                }
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(block.render()),
                    text: format!("closed union {type_}"),
                })
            }
            _ if crate::codegen::builtins::is_resource_type(&type_)
                || self
                    .type_model
                    .resource_names
                    .contains(&ParameterType::declared(&type_.without_state().name())) =>
            {
                // A resource wraps an OS handle we cannot re-open, so it has no
                // reconstructible default. The site that needs one is the
                // error-path binding of `RES x = <fallible> TRAP`. Return a CLOSED
                // resource record (see `emit_closed_resource_record`); every
                // operation then short-circuits safely and no null handle is ever
                // exposed to a program.
                let record = self.emit_closed_resource_record()?;
                // A stateful resource's `STATE` payload is null in the zeroed
                // record, so give it the same default record a real `RES … STATE`
                // binding gets — otherwise a `RECOVER`ed closed resource's
                // `.state` would dereference null. The pointer is spilled across
                // the state allocation, which clobbers every caller-saved
                // register.
                if let Some(state) = type_.state() {
                    let slot = self.allocate_stack_object("default_resource_record", 8);
                    self.emit(abi::store_u64(&record, abi::stack_pointer(), slot));
                    self.emit_resource_state_init(slot, &state, type_)?;
                    self.emit(abi::load_u64(&record, abi::stack_pointer(), slot));
                }
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(record.render()),
                    text: format!("closed {type_}"),
                })
            }
            _ if self.union_is_data(type_) => {
                // A data union's default is the first canonically-ordered
                // variant whose payload is itself defaultable, wrapped in the
                // canonical flat data-union layout `{tag@0, size@8,
                // variant-record-block@16}` (plan-02 §4.3) — identical to a
                // real `UnionWrap` value, so tag dispatch, member access,
                // sizing, and drop treat it as well-formed. The site that
                // needs one is the trap temp of a fallible union-returning
                // call bound through `TRAP` (bug-444); on the taken error
                // path the `RECOVER` value (or handler divergence) supersedes
                // it, so no program observes the synthesized default. This
                // arm is ordered after the resource checks: a resource union
                // keeps its closed-record default (`union_is_data` is false
                // for it either way).
                if defaulting_unions.iter().any(|u| u == type_) {
                    return Err(format!(
                        "native code cannot materialize default value for recursive union '{type_}'"
                    ));
                }
                let variant = self
                    .type_model
                    .variants_for_union(type_)
                    .find(|variant| {
                        let mut visited = defaulting_unions.clone();
                        visited.push(type_.clone());
                        self.default_record_materializable(&variant, &mut visited)
                    })
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "native code cannot materialize default value for type '{type_}': \
                             no variant has a defaultable payload"
                        )
                    })?;
                let tag = *self
                    .type_model
                    .union_variant_tags
                    .get(&variant)
                    .ok_or_else(|| {
                        format!("native code union variant '{variant}' does not resolve")
                    })?;
                defaulting_unions.push(type_.clone());
                let record = self.lower_default_value_inner(&variant, defaulting_unions);
                defaulting_unions.pop();
                let record = record?;
                let record_slot = self.allocate_stack_object("default_union_record", 8);
                self.emit(abi::store_u64(
                    &record.location,
                    abi::stack_pointer(),
                    record_slot,
                ));
                let register = self.emit_wrap_record_in_union(&variant, tag, record_slot)?;
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register.render()),
                    text: format!("default {type_}"),
                })
            }
            _ => {
                let Some(fields) = self.type_model.record_fields.get(type_).cloned() else {
                    return Err(format!(
                        "native code cannot materialize default value for type '{type_}'"
                    ));
                };
                let mut field_slots = Vec::with_capacity(fields.len());
                for (_, field_type) in &fields {
                    let value = self.lower_default_value_inner(field_type, defaulting_unions)?;
                    let slot = self.allocate_stack_object("default_record_field", 8);
                    self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
                    field_slots.push(slot);
                }
                // Inline `String` defaults (empty String blocks) into the record's
                // data region; scalar/pointer defaults stay inline (plan-02 §4.2).
                let register = self.emit_build_inlined_record(type_, &field_slots)?;
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register.render()),
                    text: format!("default {type_}"),
                })
            }
        }
    }

    /// True when `lower_default_value_inner` can materialize a default for
    /// `type_` — the static mirror of its arms, so the data-union arm's variant
    /// choice and the emission it then commits to always agree (a variant is
    /// only chosen if every type its payload reaches is defaultable). `visited`
    /// carries the data unions already being defaulted on this path; a variant
    /// that loops back into one is not defaultable through it.
    fn default_value_materializable(
        &self,
        type_: &ParameterType,
        visited: &mut Vec<ParameterType>,
    ) -> bool {
        match type_ {
            __t if matches!(
                __t,
                ParameterType::Nothing
                    | ParameterType::Boolean
                    | ParameterType::Byte
                    | ParameterType::Integer
                    | ParameterType::Float
                    | ParameterType::Fixed
                    | ParameterType::Money
                    | ParameterType::String
            ) || __t.is_named("Scalar") =>
            {
                true
            }
            _ if typed_is_collection_type(type_) => true,
            _ if self.is_resource_union_type(type_) => true,
            _ if builtins::is_resource_type(&type_)
                || self
                    .type_model
                    .resource_names
                    .contains(&ParameterType::declared(&type_.without_state().name())) =>
            {
                true
            }
            _ if self.union_is_data(type_) => {
                if visited.iter().any(|u| u == type_) {
                    return false;
                }
                visited.push(type_.clone());
                let variants: Vec<ParameterType> =
                    self.type_model.variants_for_union(type_).cloned().collect();
                let ok = variants
                    .iter()
                    .any(|variant| self.default_record_materializable(variant, visited));
                visited.pop();
                ok
            }
            _ => self.default_record_materializable(type_, visited),
        }
    }

    /// Record half of `default_value_materializable`: every field of the record
    /// (or union-variant record) must itself be defaultable.
    fn default_record_materializable(
        &self,
        type_: &ParameterType,
        visited: &mut Vec<ParameterType>,
    ) -> bool {
        let Some(fields) = self.type_model.record_fields.get(type_) else {
            return false;
        };
        fields
            .iter()
            .all(|(_, field_type)| self.default_value_materializable(field_type, visited))
    }

    pub(crate) fn lower_field_access(
        &mut self,
        target: &NirValue,
        member: &str,
    ) -> Result<ValueResult, String> {
        let target_value = self.lower_value(target)?;
        // plan-01-vector: a field read of a register-native vector is a direct lane
        // read — no block load, no materialization.
        if let Some(lane) = self.vector_native_field(&target_value, member) {
            return Ok(lane);
        }
        // `s.state` on a `RES` value loads the shared `STATE` payload pointer
        // from the resource record. Because a resource value is a pointer to its
        // record, an alias and the owner address the same payload.
        if member == "state" {
            if let Some(state_type) = target_value.type_.state() {
                // A resource union value is a `{tag, record-ptr}` block; the STATE
                // lives in the active variant's record reached via `+8` (plan-74).
                // For a concrete resource the value already IS the record.
                let record =
                    self.emit_resource_record_ptr(&target_value.location, &target_value.type_)?;
                let register = self.allocate_register();
                self.emit(abi::load_u64(&register, &record, RESOURCE_OFFSET_STATE));
                return Ok(ValueResult {
                    origin: None,
                    type_: state_type.clone(),
                    location: Operand::from(register.render()),
                    text: "state".to_string(),
                });
            }
        }
        let (field_index, field_type, payload_offset, inline_string) =
            if let Some((key_type, value_type)) = typed_map_entry_type_parts(&target_value.type_)
                .map(|(key, value)| (key.clone(), value.clone()))
            {
                match member {
                    "key" => (0, key_type, 0, false),
                    "value" => (1, value_type, 0, false),
                    _ => {
                        return Err(format!(
                            "native code map entry '{}' has no field '{}'",
                            target_value.type_, member
                        ));
                    }
                }
            } else if let Some(fields) = self.type_model.record_fields.get(&target_value.type_) {
                let Some((index, (_, field_type))) = fields
                    .iter()
                    .enumerate()
                    .find(|(_, (name, _))| name == member)
                else {
                    return Err(format!(
                        "native code record '{}' has no field '{}'",
                        target_value.type_, member
                    ));
                };
                let inline_string = self.record_field_is_inlined(&target_value.type_, field_type);
                // plan-114-E: a record field read yields the field type with its
                // top-level `RES ` marker stripped, leaving `Stateful { base,
                // state }` (or the bare resource when the field carries no
                // `STATE`). The VALUE is unchanged — the slot already holds the
                // record pointer; only the type spelling differs.
                //
                // This is what lets `h.handle.state` work. `split_state` matches
                // `Stateful` only at the TOP level (`src/types.rs:629`), so
                // `Res(Stateful{..}).state()` is `None` and the `.state` arm above
                // would fall through to "record has no field 'state'" — a message
                // naming the wrong problem entirely.
                //
                // Stripping is unconditional, exactly as the collection element
                // does it (`list_element("List OF RES Socket") == "Socket"`): one
                // rule for both keeps the two positions from drifting, which is
                // the same reasoning §4.1 gives.
                let field_type = typed_strip_res_marker(field_type).clone();
                (index, field_type, 0, inline_string)
            } else if let Some(fields) = self
                .type_model
                .union_variant_fields
                .get(&target_value.type_)
            {
                let Some((index, (_, field_type))) = fields
                    .iter()
                    .enumerate()
                    .find(|(_, (name, _))| name == member)
                else {
                    return Err(format!(
                        "native code variant '{}' has no field '{}'",
                        target_value.type_, member
                    ));
                };
                (index, field_type.clone(), 8, false)
            } else if self.type_model.union_names.contains(&target_value.type_) {
                // bug-147: a field name shared by two variants must resolve to a
                // deterministic offset. Walk the variants in the stable
                // canonical order (`variants_for_union`) rather than iterating
                // `union_variant_fields` in HashMap order, which produced a
                // build-nondeterministic offset for ambiguous field names.
                let Some((index, field_type)) = self
                    .type_model
                    .variants_for_union(&target_value.type_)
                    .filter_map(|variant| self.type_model.union_variant_fields.get(variant))
                    .find_map(|fields| {
                        fields
                            .iter()
                            .enumerate()
                            .find(|(_, (name, _))| name == member)
                            .map(|(index, (_, field_type))| (index, field_type.clone()))
                    })
                else {
                    return Err(format!(
                        "native code union '{}' has no payload field '{}'",
                        target_value.type_, member
                    ));
                };
                (index, field_type, 8, false)
            } else {
                return Err(format!(
                    "native code field access target '{}' is not a record or variant",
                    target_value.type_
                ));
            };
        // main made `allocate_register` infallible (the value-numbering landing);
        // the if/else this closes is already closed above.
        let register = self.allocate_register();
        self.emit(abi::load_u64(
            &register,
            &target_value.location,
            payload_offset + 8 * field_index,
        ));
        if inline_string {
            // The slot holds a block-relative offset; the alias pointer to the
            // inlined `String` block is the record base plus that offset
            // (plan-02 §4.2). `target_value.location` survives this add.
            self.emit(abi::add_registers(
                &register,
                &target_value.location,
                &register,
            ));
        }
        Ok(ValueResult {
            origin: None,
            type_: field_type.clone(),
            location: Operand::from(register.render()),
            text: format!("{}.{}", target_value.text, member),
        })
    }

    pub(crate) fn lower_with_update(
        &mut self,
        type_: &ParameterType,
        target: &NirValue,
        updates: &[NirRecordUpdate],
    ) -> Result<ValueResult, String> {
        let fields = self
            .type_model
            .record_fields
            .get(type_)
            .cloned()
            .ok_or_else(|| format!("native code WITH target '{type_}' is not a record"))?;
        let base_reg = self.temporary_vreg();
        let field_reg = self.temporary_vreg();
        let base = &base_reg;
        let field = &field_reg;
        let target_value = self.lower_value(target)?;
        let target_slot = self.allocate_stack_object("with_target", 8);
        self.emit(abi::store_u64(
            &target_value.location,
            abi::stack_pointer(),
            target_slot,
        ));

        // Resolve each updated field to its new value up front (evaluation order
        // matches source order).
        let mut updated: Vec<(usize, usize)> = Vec::with_capacity(updates.len());
        for update in updates {
            let Some(index) = fields
                .iter()
                .position(|(field_name, _)| field_name == &update.field)
            else {
                return Err(format!(
                    "native code WITH update references unknown field '{}'",
                    update.field
                ));
            };
            let value = self.lower_value(&update.value)?;
            // Observation boundary: a `Float` field updated via WITH must be
            // finite (plan-17).
            self.observe_float(&update.value, &value)?;
            // Materialize a `d`-native float or a register-native vector before
            // the field-payload spill (plan-01).
            let value = self.materialize_value(value)?;
            let value_slot = self.allocate_stack_object("with_update_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            updated.push((index, value_slot));
        }

        // Gather one value slot per field — the new value where updated, else the
        // old field value read from the target (a `String` field yields the
        // alias pointer `base + offset`) — then rebuild the inlined record so a
        // resized `String` is re-laid-out with correct offsets (plan-02 §4.5).
        let mut field_slots = Vec::with_capacity(fields.len());
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if let Some((_, value_slot)) = updated.iter().find(|(i, _)| *i == index) {
                field_slots.push(*value_slot);
                continue;
            }
            let slot = self.allocate_stack_object("with_old_field", 8);
            self.emit(abi::load_u64(base, abi::stack_pointer(), target_slot));
            self.emit(abi::load_u64(field, base, 8 * index));
            if self.record_field_is_inlined(type_, field_type) {
                self.emit(abi::add_registers(field, base, field));
            }
            self.emit(abi::store_u64(field, abi::stack_pointer(), slot));
            field_slots.push(slot);
        }
        let register = self.emit_build_inlined_record(type_, &field_slots)?;
        Ok(ValueResult {
            origin: None,
            type_: type_.clone(),
            location: Operand::from(register.render()),
            text: format!("with {}", target_value.text),
        })
    }

    pub(crate) fn lower_string_concat(
        &mut self,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(left)?;
        if left.type_ != ParameterType::String {
            return Err(format!(
                "native string concat left operand must be String, got {}",
                left.type_
            ));
        }
        let left_slot = self.allocate_stack_object("concat_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = self.lower_value(right)?;
        if right.type_ != ParameterType::String {
            return Err(format!(
                "native string concat right operand must be String, got {}",
                right.type_
            ));
        }
        let right_slot = self.allocate_stack_object("concat_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));

        // plan-118-C: the allocation, the header store and both copy loops are
        // in `_mfb_rt_string_concat`, emitted once per module. This was 169
        // instructions per site over 17,221 sites — the largest single expansion
        // category in the compiler. Both operands are reloaded from their slots
        // rather than used from their registers: the call clobbers the
        // caller-saved set, and the slots are already there because the right
        // operand's lowering could clobber the left's register anyway.
        let alloc_ok = self.label("string_concat_alloc_ok");
        self.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            left_slot,
        ));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            right_slot,
        ));
        self.emit(abi::branch_link(STRING_CONCAT_SYMBOL));
        self.push_internal_call_relocation(STRING_CONCAT_SYMBOL);
        // Capture the result before raising: `raise_error_bare` emits a call of
        // its own, and the physical return register does not survive it.
        let result_ptr = self.allocate_register();
        self.emit(abi::move_register(&result_ptr, abi::mfb_return(0)));
        self.emit(abi::compare_immediate(&result_ptr, "0"));
        self.emit(abi::branch_ne(&alloc_ok));
        // Null means the arena allocation failed. The error is raised HERE, not
        // in the helper, so its `ErrorLoc` names the concatenation the program
        // actually wrote.
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));

        Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(result_ptr.render()),
            text: format!("({} & {})", left.text, right.text),
        })
    }

    /// String concat / rope fusion — a Level-3 catalog row
    /// (`planning/optimizations.md`): lower a whole `a & b & c & …` chain into
    /// **one** pre-sized allocation and one pass of writes, instead of an
    /// intermediate String per operator.
    ///
    /// `&` is left-associative, so `a & b & c` parses as `(a & b) & c` and the
    /// pairwise lowering allocates and fills a whole intermediate for `a & b`
    /// that is then copied again into the final result and abandoned. For a
    /// chain of `n` operands that is `n - 1` allocations and quadratic copying;
    /// this is `1` allocation and one copy of each byte.
    ///
    /// **What is preserved.** Operands are lowered left to right, exactly as
    /// the nested form evaluates them, so a failing operand fails at the same
    /// point with the same earlier operands already evaluated. The only
    /// difference is that the intermediate allocations never happen — and an
    /// arena block nothing can observe is not observable. The pairwise
    /// [`Self::lower_string_concat`] still handles the two-operand case
    /// verbatim, so nothing about ordinary `a & b` changes.
    pub(crate) fn lower_string_concat_chain(
        &mut self,
        parts: &[&NirValue],
    ) -> Result<ValueResult, String> {
        // 1. Every operand, left to right, parked in its own slot. Parking is
        //    what lets the length pass and the copy pass both revisit them
        //    without holding n registers live across the allocation call.
        let mut slots = Vec::with_capacity(parts.len());
        let mut texts = Vec::with_capacity(parts.len());
        for part in parts {
            let value = self.lower_value(part)?;
            if value.type_ != ParameterType::String {
                return Err(format!(
                    "native string concat operand must be String, got {}",
                    value.type_
                ));
            }
            let slot = self.allocate_stack_object("concat_part", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            slots.push(slot);
            texts.push(value.text);
        }

        let total_slot = self.allocate_stack_object("concat_total", 8);
        let total_len_v = self.temporary_vreg();
        let part_ptr_v = self.temporary_vreg();
        let part_len_v = self.temporary_vreg();
        let write_cur_v = self.temporary_vreg();
        let read_cur_v = self.temporary_vreg();
        let remaining_v = self.temporary_vreg();
        let byte_v = self.temporary_vreg();
        let total_len = &total_len_v;
        let part_ptr = &part_ptr_v;
        let part_len = &part_len_v;
        let write_cur = &write_cur_v;
        let read_cur = &read_cur_v;
        let remaining = &remaining_v;
        let byte = &byte_v;

        // 2. Sum the byte lengths. Every String's length is its header word, so
        //    this is one load per operand — no scanning.
        self.emit(abi::move_immediate(total_len, "Integer", "0"));
        for slot in &slots {
            self.emit(abi::load_u64(part_ptr, abi::stack_pointer(), *slot));
            self.emit(abi::load_u64(part_len, part_ptr, 0));
            self.emit(abi::add_registers(total_len, total_len, part_len));
        }
        self.emit(abi::store_u64(total_len, abi::stack_pointer(), total_slot));

        // 3. One allocation for the whole chain (8-byte header + bytes + NUL).
        let alloc_ok = self.label("string_concat_chain_alloc_ok");
        self.emit(abi::add_immediate(abi::c_arg(0), total_len, 9));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        // Carry the result out of the physical return register immediately: the
        // copy loops' back edges break the result-vs-argument dataflow on ISAs
        // whose result and argument registers differ (the same reason the
        // pairwise lowering does this).
        let result_ptr = self.allocate_register();
        self.emit(abi::move_register(&result_ptr, abi::mfb_return(1)));
        self.emit(abi::load_u64(total_len, abi::stack_pointer(), total_slot));
        self.emit(abi::store_u64(total_len, &result_ptr, 0));
        self.emit(abi::add_immediate(write_cur, &result_ptr, 8));

        // 4. Copy each operand's bytes in order into the single buffer.
        for slot in &slots {
            let loop_label = self.label("string_concat_chain_loop");
            let done_label = self.label("string_concat_chain_done");
            self.emit(abi::load_u64(read_cur, abi::stack_pointer(), *slot));
            self.emit(abi::load_u64(remaining, read_cur, 0));
            self.emit(abi::add_immediate(read_cur, read_cur, 8));
            self.emit(abi::label(&loop_label));
            self.emit(abi::compare_immediate(remaining, "0"));
            self.emit(abi::branch_eq(&done_label));
            self.emit(abi::load_u8(byte, read_cur, 0));
            self.emit(abi::store_u8(byte, write_cur, 0));
            self.emit(abi::add_immediate(read_cur, read_cur, 1));
            self.emit(abi::add_immediate(write_cur, write_cur, 1));
            self.emit(abi::subtract_immediate(remaining, remaining, 1));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&done_label));
        }
        self.emit(abi::move_immediate(byte, "Integer", "0"));
        self.emit(abi::store_u8(byte, write_cur, 0));

        Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(result_ptr.render()),
            text: format!("({})", texts.join(" & ")),
        })
    }

    pub(crate) fn global_value(&self, name: &str) -> Result<GlobalValue, String> {
        self.globals
            .get(name)
            .cloned()
            .ok_or_else(|| format!("native code global '{name}' does not resolve"))
    }

    pub(crate) fn load_global_address(&mut self, name: &str) -> Result<String, String> {
        let global = self.global_value(name)?;
        let register = self.allocate_register();
        self.emit(abi::add_immediate(
            &register,
            ARENA_STATE_REGISTER,
            global.offset,
        ));
        Ok(register.render())
    }

    pub(crate) fn local_constant_value(&self, value: &NirValue) -> Option<NirValue> {
        match value {
            NirValue::Const { .. } => Some(value.clone()),
            NirValue::Local(name) => self
                .locals
                .get(name)
                .and_then(|local| local.constant.clone()),
            NirValue::Global { .. } => None,
            NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => self
                .static_primitive_text(&args[0])
                .map(|value| NirValue::Const {
                    type_: ParameterType::String,
                    value,
                }),
            NirValue::RuntimeCall { target, args, .. }
                if target == "toString" && args.len() == 1 =>
            {
                self.static_primitive_text(&args[0])
                    .map(|value| NirValue::Const {
                        type_: ParameterType::String,
                        value,
                    })
            }
            NirValue::Call { target, args, .. } | NirValue::RuntimeCall { target, args, .. }
                if target == "typeName" && args.len() == 1 =>
            {
                // `typeName` folds to the argument type's SPELLING.
                self.static_type_name(&args[0])
                    .map(|type_| NirValue::Const {
                        type_: ParameterType::String,
                        value: type_.name().into_owned(),
                    })
            }
            NirValue::Binary { op, .. } if *op == BinaryOp::Concat => self
                .static_string_value(value)
                .map(|value| NirValue::Const {
                    type_: ParameterType::String,
                    value,
                }),
            _ => None,
        }
    }

    /// [`static_string_value`](Self::static_string_value) for a **pre-lowered**
    /// `abi_inline` arg: the constant-folding reads the source `NirValue` off the
    /// `ValueResult::origin` (the value is already lowered, but its source node is
    /// kept for exactly this kind of compile-time fold).
    pub(crate) fn static_string_value_vr(&self, value: &ValueResult) -> Option<String> {
        value
            .origin
            .as_ref()
            .and_then(|nir| self.static_string_value(nir))
    }

    pub(crate) fn static_string_value(&self, value: &NirValue) -> Option<String> {
        match value {
            NirValue::Const { type_, value } if matches!(type_, ParameterType::String) => {
                Some(value.clone())
            }
            NirValue::Local(name) => self
                .locals
                .get(name)
                .and_then(|local| local.constant.as_ref())
                .and_then(|constant| self.static_string_value(constant)),
            NirValue::Global { .. } => None,
            NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
                self.static_primitive_text(&args[0])
            }
            NirValue::RuntimeCall { target, args, .. }
                if target == "toString" && args.len() == 1 =>
            {
                self.static_primitive_text(&args[0])
            }
            NirValue::Call { target, args, .. } | NirValue::RuntimeCall { target, args, .. }
                if target == "typeName" && args.len() == 1 =>
            {
                // MUST use the SAME resolver the builder's `typeName` fold uses
                // (`static_type_name_for_fold`, via `resolve_call_return_type`), not
                // the coarser `static_type_name` whose call arm only recognizes a
                // hardcoded builtin list. The builder folds `typeName(<any call>)`
                // — e.g. `typeName(math::abs(x))` or `typeName(io::isBuffered())` —
                // to a rodata `String` constant; if this ownership classifier misses
                // that fold (as `static_type_name` does for a package call not in its
                // list), `value_needs_owning_copy` returns false and `LET t =
                // typeName(math::abs(x))` binds `t` straight to the rodata pointer
                // with no deep copy — scope-drop then `arena_free`s a read-only
                // constant (a write into rodata → SIGBUS). Same fold-mismatch class
                // as the strings-package note below.
                self.static_type_name_for_fold(&args[0])
                    .map(|type_| type_.name().into_owned())
            }
            NirValue::Binary {
                op, left, right, ..
            } if *op == BinaryOp::Concat => {
                let left = self.static_string_value(left)?;
                let right = self.static_string_value(right)?;
                Some(format!("{left}{right}"))
            }
            // The unicode case/normalization builtins fold a static-string argument
            // to a static result (each `strings::` `func_*` lowering consults
            // `static_strings_package_string`). This resolver MUST recognize the
            // same folds: `value_needs_owning_copy` consults it to decide whether a
            // bound value is a rodata constant needing a deep copy. If it misses the
            // fold, `LET r = caseFold("HELLO")` binds `r` straight to the folded
            // rodata pointer with no copy, and scope-drop then `arena_free`s a
            // read-only constant — an arena free-list corruption that only surfaces
            // on a *later* allocation (e.g. the next `mfb test` case).
            NirValue::Call { target, args, .. } | NirValue::RuntimeCall { target, args, .. }
                if args.len() == 1
                    && matches!(
                        target.as_str(),
                        "strings.upper"
                            | "strings.lower"
                            | "strings.caseFold"
                            | "strings.normalizeNfc"
                    ) =>
            {
                let value = self.static_string_value(&args[0])?;
                Some(match target.as_str() {
                    "strings.upper" => crate::unicode::backend::upper(&value),
                    "strings.lower" => crate::unicode::backend::lower(&value),
                    "strings.caseFold" => crate::unicode::backend::case_fold(&value),
                    "strings.normalizeNfc" => crate::unicode::backend::normalize_nfc(&value),
                    _ => unreachable!(),
                })
            }
            _ => None,
        }
    }

    pub(crate) fn static_primitive_text(&self, value: &NirValue) -> Option<String> {
        match value {
            NirValue::Const { type_, value } => match type_ {
                // Float/Fixed constants fold to the runtime formatter's
                // default-precision rendering (2 places; bug-358, plan-28-B).
                ParameterType::Float | ParameterType::Fixed => {
                    crate::numeric::default_to_string_text(type_, value)
                }
                ParameterType::Integer | ParameterType::Byte | ParameterType::String => {
                    Some(value.clone())
                }
                ParameterType::Boolean => match value.as_str() {
                    "true" => Some("TRUE".to_string()),
                    "false" => Some("FALSE".to_string()),
                    _ => None,
                },
                _ => None,
            },
            NirValue::Local(name) => self
                .locals
                .get(name)
                .and_then(|local| local.constant.as_ref())
                .and_then(|constant| self.static_primitive_text(constant)),
            NirValue::Global { .. } => None,
            _ => None,
        }
    }

    /// The static type of a value for the **in-place collection gates**, widened
    /// past [`Self::static_type_name`]'s hand-written builtin-name table to the
    /// declared return type of a call to a user (or `LINK`) function.
    ///
    /// Those gates ask one question — "is this operand statically exactly `T`?" —
    /// to tell a single-element `append(list, x)` from a bulk
    /// `append(list, otherList)`. `static_type_name` answers `None` for ANY call it
    /// does not have a hard-coded row for, so every
    /// `list = collections::append(list, someFunc(…))` fell off the in-place path
    /// and copied the whole buffer per element: the accumulate loop ran O(n^2)
    /// (50 000 appends took 60 s against 3 ms for the same loop appending a plain
    /// local). A callee's declared `returns` is exactly as static as a local's
    /// declared type, so answering from it is sound for a gate that only asks the
    /// operand's type — it is NOT a widening of `static_type_name`, whose other
    /// consumers (the float-numeric-error gate, module analysis, binary typing)
    /// must keep their exact current answers.
    pub(crate) fn static_item_type(&self, value: &NirValue) -> Option<ParameterType> {
        if let Some(type_) = self.static_type_name(value) {
            return Some(type_);
        }
        match value {
            NirValue::Call { target, .. } => self
                .functions
                .get(target)
                .map(|function| function.returns.clone())
                .or_else(|| self.package_return_types.get(target).cloned()),
            _ => None,
        }
    }

    pub(crate) fn static_type_name(&self, value: &NirValue) -> Option<ParameterType> {
        match value {
            NirValue::Const { type_, .. } => Some(type_.clone()),
            NirValue::Local(name) => self.locals.get(name).map(|local| local.type_.clone()),
            NirValue::LocalRef { type_, .. } => Some(type_.clone()),
            NirValue::Global { name, type_ } => {
                if is_unset_type(type_) {
                    self.globals.get(name).map(|global| global.type_.clone())
                } else {
                    Some(type_.clone())
                }
            }
            NirValue::FunctionRef { type_, .. }
            | NirValue::Closure { type_, .. }
            | NirValue::Capture { type_, .. }
            | NirValue::Constructor { type_, .. }
            | NirValue::WithUpdate { type_, .. }
            | NirValue::ListLiteral { type_, .. }
            | NirValue::SetLiteral { type_, .. }
            | NirValue::MapLiteral { type_, .. } => Some(type_.clone()),
            NirValue::UnionWrap { union_type, .. } => Some(union_type.clone()),
            NirValue::UnionExtract { type_, .. } => Some(type_.clone()),
            NirValue::ResultIsOk { .. } => Some(ParameterType::Boolean),
            NirValue::ResultValue { value } => match self.static_type_name(value)? {
                // A non-`Result` operand answers with its own type, as the
                // `strip_prefix(…).or_else(…)` this replaces did.
                ParameterType::ResultOf(success) => Some(*success),
                other => Some(other),
            },
            NirValue::ResultError { .. } => Some(error_type()),
            // bug-471: the success type, matching the `CallResult` arm below.
            NirValue::Checked { type_, .. } => Some(type_.clone()),
            NirValue::Call { target, args, .. }
            | NirValue::CallResult { target, args, .. }
            | NirValue::RuntimeCall { target, args, .. } => match target.as_str() {
                "replace" | "typeName" | "toString" => Some(ParameterType::String),
                "find" | "len" | "toInt" => Some(ParameterType::Integer),
                "mid" => Some(ParameterType::String),
                "toFloat" => Some(ParameterType::Float),
                "toFixed" => Some(ParameterType::Fixed),
                "toByte" => Some(ParameterType::Byte),
                "toMoney" => Some(ParameterType::Money),
                "toScalar" => Some(scalar_type()),
                "isNumeric" => Some(ParameterType::Boolean),
                // A list element read resolves to the list's element type, so an
                // append/set whose item is a `get` (or arithmetic over `get`s)
                // stays on the allocation-free in-place fast path instead of the
                // value-semantic fallback that copies the whole list each op
                // (bug-01). Only lists are resolved here (map value reads fall
                // through to the conservative `None`).
                "get" | "getOr" | "collections.get" | "collections.getOr" => args
                    .first()
                    .and_then(|arg| self.static_type_name(arg))
                    .and_then(|type_| match type_ {
                        ParameterType::ListOf(element) => Some(*element),
                        _ => None,
                    }),
                "math.floor" | "math.ceil" | "math.round" => Some(ParameterType::Integer),
                "math.sqrt" | "math.exp" | "math.log" | "math.log10" | "math.sin" | "math.cos"
                | "math.tan" | "math.asin" | "math.acos" | "math.atan" => {
                    args.first().and_then(|arg| self.static_type_name(arg))
                }
                "math.pow" | "math.atan2" => {
                    args.first().and_then(|arg| self.static_type_name(arg))
                }
                _ => None,
            },
            NirValue::Binary {
                op, left, right, ..
            } => {
                if op.is_comparison() || matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor)
                {
                    return Some(ParameterType::Boolean);
                }
                if *op == BinaryOp::Concat {
                    return Some(ParameterType::String);
                }
                let left = self.static_type_name(left)?;
                let right = self.static_type_name(right)?;
                Some(promoted_binary_type(*op, &left, &right))
            }
            NirValue::Unary { op, operand, .. } => {
                if *op == UnaryOp::Not {
                    Some(ParameterType::Boolean)
                } else {
                    self.static_type_name(operand)
                }
            }
            NirValue::MemberAccess { target, member } => {
                let target_type = self.static_type_name(target)?;
                self.member_type_of(&target_type, member)
            }
        }
    }

    /// The type of `member` read off a value of `target_type`: the thread
    /// handle's `result`, a record or union-variant field, or a typed map
    /// entry's `key`/`value`. Split out of [`Self::static_type_name`] so
    /// [`Self::overload_arg_type`] can apply the same lookup to a target that
    /// only IT can type (bug-497: `makeRec().body`).
    fn member_type_of(&self, target_type: &ParameterType, member: &str) -> Option<ParameterType> {
        if member == "result" {
            if let ParameterType::ThreadHandle {
                worker: false, out, ..
            } = target_type
            {
                return Some(ParameterType::result_of((**out).clone()));
            }
        }
        // A record or union-variant field, read from the same tables the
        // field-access lowering itself uses. Without this arm the builder
        // could not name the type of `rec.field`, so `typeName(rec.field)`
        // failed to lower at all with "cannot determine typeName argument
        // type" (bug-366).
        // The two field tables are keyed by nominal type NAME, so the
        // lookup renders the (scalar-cheap) name — a name-keyed table
        // probe, not a type-string derivation.
        let owner = target_type.name();
        let field_type = self
            .type_model
            .record_fields
            .get(&ParameterType::declared(owner.as_ref()))
            .or_else(|| {
                self.type_model
                    .union_variant_fields
                    .get(&ParameterType::declared(owner.as_ref()))
            })
            .and_then(|fields| {
                fields
                    .iter()
                    .find(|(name, _)| name == member)
                    .map(|(_, type_)| type_.clone())
            });
        if field_type.is_some() {
            return field_type;
        }
        let (key_type, value_type) = typed_map_entry_type_parts(target_type)?;
        match member {
            "key" => Some(key_type.clone()),
            "value" => Some(value_type.clone()),
            _ => None,
        }
    }

    /// Static type of `value`, resolving builtin calls the hand-written
    /// [`Self::static_type_name`] table misses by delegating to the authoritative
    /// per-package resolvers (`builtins::resolve_call_return_type`).
    ///
    /// This is used **only** for the `typeName` compile-time fold (bug-354).
    /// `typeName` must fold its argument's type to a string constant; before this,
    /// `static_type_name`'s table named zero `strings.*`, so `typeName` of every
    /// `strings::` call — plus `math::abs/min/max` and
    /// `collections::find/contains/hasKey` — failed to lower on valid source.
    ///
    /// It deliberately does NOT widen `static_type_name` itself: that function
    /// also gates the in-place-append fast path, numeric-result typing, and the
    /// slice specialization, and widening it would shift their codegen for every
    /// program using these builtins (including inside inlined package bodies). The
    /// fold is the only place a table miss is a hard error, so the resolver
    /// fallback is scoped to exactly here. Recurses through itself so nested calls
    /// (`typeName(strings::upper(strings::lower(s)))`) resolve too.
    pub(crate) fn static_type_name_for_fold(&self, value: &NirValue) -> Option<ParameterType> {
        if let Some(type_) = self.static_type_name(value) {
            return Some(type_);
        }
        match value {
            NirValue::Call { target, args, .. }
            | NirValue::CallResult { target, args, .. }
            | NirValue::RuntimeCall { target, args, .. } => {
                let arg_types = args
                    .iter()
                    .map(|arg| self.static_type_name_for_fold(arg))
                    .collect::<Option<Vec<_>>>()?;
                builtins::resolve_call_return_type_typed(target, &arg_types, false)
            }
            _ => None,
        }
    }

    /// Static type of a runtime-call ARGUMENT, for the code-form (overload)
    /// selection in [`Self::lower_runtime_helper_call`].
    ///
    /// bug-476: several native members collapse two overloads into one name and
    /// choose the *lowering* here, from an argument's static type —
    /// `tcp::write`/`tls::write`/`udp::send` (bytes vs text),
    /// `tcp::connect`/`tls::connect` (host/port vs `Address`), `net::ping`,
    /// `tcp`/`udp`/`tls` `poll` (scalar vs list), `tls::localAddress`
    /// (`Socket` vs `Listener`) and `io::print`'s `AttributedString` rewrite.
    /// [`Self::static_type_name`]'s `NirValue::Call` arm is a hand-written table
    /// of a dozen builtins, so **any other call answered `None`** and every one
    /// of those selectors silently took its fallback form. For `tcp::write` that
    /// meant `tcp::write(sock, buildHead(x))` marshalling a `String*` through the
    /// collection path: a garbage element count, a failed `write(2)`, and an
    /// `ErrConnectionClosed` raised with nothing on the wire — which is how
    /// `http::handleRequest` came to serve an empty reply to every request.
    ///
    /// The miss is only in the *call* arm, and only user/package functions and
    /// untabulated builtins are added: a call is resolved against the same
    /// return-type tables `emit_call` uses (the NIR function set, then the
    /// package return types), falling back to
    /// [`Self::static_type_name_for_fold`]'s registry resolver. It deliberately
    /// does NOT widen `static_type_name` itself — that also gates the in-place
    /// append/set fast path, numeric-result typing and the slice specialisation,
    /// where naming more call results changes codegen (and, for
    /// `x = collections::append(x, f())`, the aliasing decision) far outside an
    /// overload choice.
    ///
    /// bug-497 widened the call arm past named functions to a call through a
    /// FUNC-typed value, and added the `MemberAccess`/`ResultValue` arms, after
    /// `tcp::write(sock, f(x))` and `tcp::write(sock, makeRec().body)` were
    /// measured still taking the byte-list form — the same peer-controlled
    /// out-of-bounds read the named-function case was (OS-50). The write
    /// selectors now refuse an unresolved payload outright
    /// (`net_write_payload_form`), so a shape this cannot type is a build error.
    pub(crate) fn overload_arg_type(&self, value: &NirValue) -> Option<ParameterType> {
        if let Some(type_) = self.static_type_name(value) {
            return Some(type_);
        }
        match value {
            NirValue::Call { target, .. }
            | NirValue::CallResult { target, .. }
            | NirValue::RuntimeCall { target, .. } => {
                if let Some(type_) = self
                    .functions
                    .get(target.as_str())
                    .map(|function| function.returns.clone())
                    .or_else(|| self.package_return_types.get(target.as_str()).cloned())
                {
                    return Some(type_);
                }
                // bug-497: a call THROUGH a FUNC-typed value — `LET f AS
                // FUNC(String) AS String = reply` then `f(x)` — is a `Call` whose
                // target is the VALUE's name, so neither function table above
                // knows it. Its declared type carries the return type, exactly
                // as a named function's `returns` does.
                if let Some(ParameterType::Func(_, returns, _)) = self
                    .locals
                    .get(target.as_str())
                    .map(|local| local.type_.clone())
                    .or_else(|| {
                        self.globals
                            .get(target.as_str())
                            .map(|global| global.type_.clone())
                    })
                {
                    return Some(*returns);
                }
                self.static_type_name_for_fold(value)
            }
            // bug-497: `makeRec().body` — a field read off a call result.
            // `static_type_name`'s own arm gives up because it cannot type the
            // call; type the target with THIS resolver and reuse its field lookup.
            NirValue::MemberAccess { target, member } => {
                let target_type = self.overload_arg_type(target)?;
                self.member_type_of(&target_type, member).or_else(|| {
                    // `res.state` on a STATE-carrying resource (or resource
                    // union). `member_type_of` knows only record/union-variant
                    // fields, a thread handle's `result`, and a typed map's
                    // key/value — so every payload reached THROUGH the STATE
                    // block (`client.state.raw`) was unresolved, and the
                    // fail-closed write selector this bug added then refused a
                    // VALID program at build time:
                    //   error: native runtime tls.write: payload static type
                    //   <unresolved> is neither String nor List OF Byte
                    // (`tests/rt_macos_d4_union_state_tls.rs`). The lowering
                    // itself has always known this type — `emit_member_access`
                    // reads it off `type_.state()` — so only the static side
                    // was missing.
                    //
                    // Resolved HERE and not in `member_type_of`, because that
                    // is shared with `static_type_name`, which also gates the
                    // in-place append/set fast path: typing `client.state.raw`
                    // there would retype
                    // `client.state.raw = collections::append(client.state.raw, …)`
                    // and shift that statement's codegen and aliasing decision,
                    // far outside an overload choice.
                    if member == "state" {
                        target_type.state()
                    } else {
                        None
                    }
                })
            }
            NirValue::ResultValue { value } => match self.overload_arg_type(value)? {
                ParameterType::ResultOf(success) => Some(*success),
                other => Some(other),
            },
            _ => None,
        }
    }

    pub(crate) fn thread_runtime_return_type(
        &self,
        target: &str,
        args: &[NirValue],
    ) -> Option<ParameterType> {
        match target {
            // The worker entry's FIRST parameter — `thread::start`'s runtime
            // return type. plan-106-E: the isolated-FUNC spelling is a variant, so
            // the parameter is read off it instead of being re-split out of
            // `ISOLATED FUNC(` … `) AS `.
            "thread.start" => match self.static_type_name(args.first()?)? {
                ParameterType::Func(params, _, true) => params.first().cloned(),
                _ => None,
            },
            "thread.isRunning" | "thread.poll" | "thread.isCancelled" => {
                Some(ParameterType::Boolean)
            }
            "thread.cancel"
            | "thread.send"
            | "thread.transferResource"
            | "thread.emitResource"
            | "thread.openStdIn"
            | "thread.closeStdIn" => Some(ParameterType::Nothing),
            // plan-111-E: the three thread planes are `ThreadHandle` fields, so
            // each is read off the variant rather than re-split from a spelling
            // by `parent_thread_output` / `thread_message` / `thread_resource`.
            "thread.waitFor" => match self.static_type_name(args.first()?)? {
                ParameterType::ThreadHandle {
                    worker: false, out, ..
                } => Some((*out).clone()),
                _ => None,
            },
            "thread.receive" => match self.static_type_name(args.first()?)? {
                ParameterType::ThreadHandle { msg, .. } => Some((*msg).clone()),
                _ => None,
            },
            // The resource plane: accept yields the thread's resource type
            // (worker reads the inbound queue, parent reads the outbound queue).
            "thread.acceptResource" | "thread.readResource" => {
                match self.static_type_name(args.first()?)? {
                    ParameterType::ThreadHandle { res, .. } => Some((*res).clone()),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub(crate) fn lower_match_compare(
        &mut self,
        matched: &ValueResult,
        pattern: &NirValue,
        label: &str,
    ) -> Result<(), String> {
        match pattern {
            NirValue::MemberAccess { target, member } => {
                let NirValue::Local(type_name) = target.as_ref() else {
                    return Err("native code enum match pattern must name enum type".to_string());
                };
                let ordinal = self
                    .type_model
                    .enum_members
                    .get(&(ParameterType::declared(type_name), member.clone()))
                    .copied()
                    .ok_or_else(|| {
                        format!("native code enum member '{type_name}.{member}' does not resolve")
                    })?;
                self.emit(abi::compare_immediate(
                    &matched.location,
                    &ordinal.to_string(),
                ));
                self.emit(abi::branch_eq(label));
            }
            NirValue::Local(variant)
                if self
                    .type_model
                    .union_variants
                    .contains_key(&ParameterType::declared(variant)) =>
            {
                let tag = self
                    .type_model
                    .union_variant_tags
                    .get(&ParameterType::declared(variant))
                    .copied()
                    .ok_or_else(|| {
                        format!("native code union variant '{variant}' does not resolve")
                    })?;
                let tag_register = self.allocate_register();
                self.emit(abi::load_u64(&tag_register, &matched.location, 0));
                self.emit(abi::compare_immediate(&tag_register, &tag.to_string()));
                self.emit(abi::branch_eq(label));
            }
            _ => {
                let pattern = self.lower_value(pattern)?;
                // String (and other block-typed) scrutinees are block pointers, so
                // `compare_registers` would test pointer identity — never true for a
                // literal pattern whose block is distinct from the scrutinee's, so
                // every such CASE was dead (bug-140). Route content-typed scrutinees
                // through the byte-comparison helper; scalars keep the register test.
                if matched.type_ == ParameterType::String
                    || self.type_model.record_fields.contains_key(&matched.type_)
                {
                    let not_equal = self.label("match_compare_not_equal");
                    self.emit_comparable_values_match_branch(
                        &matched.type_,
                        &matched.location,
                        &pattern.location,
                        label,
                        &not_equal,
                    )?;
                    self.emit(abi::label(&not_equal));
                } else {
                    self.emit(abi::compare_registers(&matched.location, &pattern.location));
                    self.emit(abi::branch_eq(label));
                }
            }
        }
        Ok(())
    }

    /// True when a `Result` payload of `payload_type` is a heap block addressed by
    /// pointer (inlined whole), versus an inline scalar (stored in the 8-byte
    /// payload word). Mirrors the record/collection inline rules (plan-02 §4.3).
    pub(crate) fn result_payload_is_block(&self, payload_type: &ParameterType) -> bool {
        *payload_type == ParameterType::String
            || *payload_type == ParameterType::named("Error")
            || typed_is_collection_type(payload_type)
            || matches!(payload_type, ParameterType::ResultOf(_))
            || self.type_model.record_fields.contains_key(payload_type)
            // A **data** union is inlined whole; a **resource** union is a scalar
            // pointer to its `{tag, ptr}` block, like a concrete resource, so it
            // occupies the 8-byte payload word — not an inlinable block (plan-75
            // gap 4). `union_is_data` base-strips any `STATE T` suffix.
            || self.union_is_data(payload_type)
    }
}
