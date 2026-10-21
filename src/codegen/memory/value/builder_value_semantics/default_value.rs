//! plan-176-P: default values — resource records and their `STATE`, and the default of any type.
//! String concatenation and `MATCH` comparison live in `memory/value/builder_value_semantics/mod.rs`.

use super::*;
use crate::codegen::collection::layout::field_is_inlined_union_box;

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
        value_ptr: impl Into<abi::Reg>,
        type_: &ParameterType,
    ) -> Result<abi::Reg, String> {
        if self.is_resource_union_type(type_) {
            let record = self.allocate_register();
            self.emit(abi::load_u64(&record, value_ptr, 8));
            Ok(record.into())
        } else {
            Ok(value_ptr.into())
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
                    location: Operand::from(register),
                    text: "default Nothing".to_string(),
                })
            }
            ParameterType::Boolean => {
                let register = self.allocate_register();
                self.emit(abi::move_immediate(&register, "Boolean", "0"));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register),
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
                    location: Operand::from(register),
                    text: format!("default {type_}"),
                })
            }
            ParameterType::String => {
                let register = self.load_empty_string_constant()?;
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register),
                    text: "default String".to_string(),
                })
            }
            // bug-781: a function value's default is the null word. A `FUNC` has no
            // value to make up — no function to point at — and the sites that need a
            // default (the temp an inline `TRAP` binds, a defaulted record field) never
            // let a program call it: the `RECOVER` value or the handler's divergence
            // supersedes it on the error path, as for every default here. The null word
            // is the one function value no holder copies or frees (`_mfb_rt_closure_copy`
            // returns it, `_mfb_rt_closure_drop` skips it). Without this arm every inline
            // `TRAP` on a call returning a function value failed to build.
            ParameterType::Func(..) => {
                let register = self.allocate_register();
                self.emit(abi::move_immediate(&register, "Integer", "0"));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register),
                    text: format!("default {type_}"),
                })
            }
            // A record field declared `AS RES T` carries the `RES` marker in its type
            // (`model.record_fields` keeps it), and its default is `T`'s: the closed
            // resource record, or a resource union's `{tag, record-ptr}` box around
            // one. Without this arm a record holding a `RES` field next to a field
            // that makes it not freeable-flat (a function value) failed every inline
            // `TRAP` that bound it: the trap temp needs the record's default.
            ParameterType::Res(inner) => {
                let value = self.lower_default_value_inner(inner, defaulting_unions)?;
                Ok(ValueResult {
                    type_: type_.clone(),
                    ..value
                })
            }
            _ if self.is_enum_type(type_) => {
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
                    location: Operand::from(register),
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
                    location: Operand::from(block),
                    text: format!("closed union {type_}"),
                })
            }
            // bug-479: a `Thread` view, for the error path of an inline `TRAP` on
            // `thread::start` — a CLOSED resource record, exactly as for any other
            // resource (plan-156-C), tagged as a thread view. Every op on it answers
            // `ErrResourceClosed` before reaching for a control block (it names none),
            // and its drop is the benign re-close. Ordered before the resource arms
            // because `is_resource_type` does not answer for a thread type.
            ParameterType::ThreadHandle { .. } => {
                let handle = self.emit_closed_resource_record()?;
                let tag = self.allocate_register();
                self.emit(abi::move_immediate(&tag, "Integer", RESOURCE_TAG_THREAD));
                self.emit(abi::store_u64(&tag, &handle, RESOURCE_OFFSET_TAG));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(handle),
                    text: format!("closed {type_}"),
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
                    location: Operand::from(record),
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
                // The wrap copied the record's bytes into the union, so the record block itself
                // is dead: every inline-`TRAP` bind of a union leaked it (16 B per bind).
                let union_slot = self.allocate_stack_object("default_union_block", 8);
                self.emit(abi::store_u64(&register, abi::stack_pointer(), union_slot));
                self.emit_shallow_block_free(&variant, record_slot)?;
                let register = self.allocate_register();
                self.emit(abi::load_u64(&register, abi::stack_pointer(), union_slot));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register),
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
                let mut inlined_defaults = Vec::new();
                let mut union_boxes = Vec::new();
                for (_, field_type) in &fields {
                    let value = self.lower_default_value_inner(field_type, defaulting_unions)?;
                    let slot = self.allocate_stack_object("default_record_field", 8);
                    self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
                    field_slots.push(slot);
                    // bug-1012: the record holds a `RES` resource union's box inline, so
                    // it copies the default box's two words and the box block built
                    // above is dead. The closed record it names now belongs to the
                    // record's copy.
                    if field_is_inlined_union_box(&self.type_model, field_type) {
                        union_boxes.push((slot, field_type.clone()));
                    }
                    // bug-625: an inlined default other than the shared empty String
                    // constant is a block built here, which the record byte-copies and
                    // nothing else owns.
                    if *field_type != ParameterType::String
                        && self.record_field_is_inlined(field_type)
                        && self.is_freeable_flat_value(field_type)
                    {
                        inlined_defaults.push((slot, field_type.clone()));
                    }
                }
                // Inline `String` defaults (empty String blocks) into the record's
                // data region; scalar/pointer defaults stay inline (plan-02 §4.2).
                let register = self.emit_build_inlined_record(type_, &field_slots)?;
                let mut record = ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register),
                    text: format!("default {type_}"),
                };
                for (slot, field_type) in inlined_defaults {
                    record = self.free_intermediate_collection(slot, &field_type, record)?;
                }
                if !union_boxes.is_empty() {
                    // The frees call `arena_free`, which clobbers every caller-saved
                    // register: carry the record across them in a slot.
                    let keep = self.allocate_stack_object("default_record_keep", 8);
                    self.emit(abi::store_u64(&record.location, abi::stack_pointer(), keep));
                    for (slot, field_type) in &union_boxes {
                        self.emit_shallow_block_free(field_type, *slot)?;
                    }
                    let register = self.allocate_register();
                    self.emit(abi::load_u64(register, abi::stack_pointer(), keep));
                    record.location = Operand::from(register);
                }
                Ok(record)
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
            // bug-781: the null function value (`lower_default_value_inner`).
            ParameterType::Func(..) => true,
            ParameterType::Res(inner) => self.default_value_materializable(inner, visited),
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
}
