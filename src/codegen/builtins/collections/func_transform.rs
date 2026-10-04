//! `collections::transform` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::list_entry_stride;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::{typed_callable_return_type, typed_list_element_type};
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::operators::{BinaryOp, UnaryOp};
use crate::target::shared::abi;
use crate::target::shared::nir::{source_file_id, NirOp, NirSourceLoc, NirValue};
use crate::types::ParameterType;
const INTO_TRANSFORM: &str =
    "Map every element of a list through a function and collect the results";
const DESC_TRANSFORM: &str = r#"`collections::transform` walks `value` from the first element to the last,
calls `f` once per element with that element as its only argument, and appends
each returned value to a new list. The result therefore has exactly as many
elements as `value`, in the same order.

The element type of the result is `f`'s success type `U`, so mapping a
`List OF Integer` through a `FUNC(Integer) AS String` yields a `List OF String`.
`U` may differ from `T` or equal it.

`f` must be a callable *value* — a declared `FUNC`, a `LAMBDA`, or a `FUNC` value
such as a function parameter of a matching type. An overloaded built-in such as `toString` is not a callable value and
cannot be passed here; wrap it in a one-line `FUNC` of your own instead. The
single-argument `general` predicates (`isEven`, `isOdd`, and friends) *are*
ordinary callables and can be passed directly where their type fits.

`f` must produce a value: a callback whose success type is `Nothing` — such as a
`FUNC … AS Nothing` — is a compile-time error, because there would be nothing to collect. Use
`collections::forEach` to run a callback purely for its side effects.

`value` is not modified; the result is a new list with exactly one value per
source element, in the same order.

An empty `value` calls `f` zero times and yields an empty `List OF U`.

`transform` raises no domain error of its own. It can fail only
because a failing `f` propagates: when the callback fails,
the loop stops immediately at that element, later elements are never visited, no
result list is produced, and the callback's own error is passed through
unchanged.

An inline `TRAP` on a `transform` call captures that propagated callback error
at the call site rather than letting it auto-propagate. `ErrOutOfMemory` means the program ran out of memory."#;

const EX: &str = r#"Map integers to strings, changing the element type:

```
IMPORT collections
IMPORT io

FUNC label(value AS Integer) AS String
  RETURN "n=" & toString(value)
END FUNC

FUNC main AS Integer
  LET labels AS List OF String = collections::transform([1, 2, 3], label)
  io::print(collections::get(labels, 0))
  RETURN 0
END FUNC
```

Map with a `LAMBDA`, keeping the element type:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET tripled AS List OF Integer = collections::transform([1, 2, 3], LAMBDA(value AS Integer) -> value * 3)
  io::print(toString(collections::get(tripled, 2)))
  RETURN 0
END FUNC
```

An empty input yields an empty result:

```
IMPORT collections
IMPORT io

FUNC double(value AS Integer) AS Integer
  RETURN value * 2
END FUNC

FUNC main AS Integer
  LET empty AS List OF Integer = []
  LET mapped AS List OF Integer = collections::transform(empty, double)
  io::print(toString(len(mapped)))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "transform",
        intro: INTO_TRANSFORM,
        desc: DESC_TRANSFORM,
        example: EX,
        expected_arguments: Some("List OF T, FUNC(T) AS U"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The list to map over. Not modified — you get a new list back.",
                    aliases: &["collection"],
                    ty: ParameterType::list_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "f",
                    desc: "Called once per element, in order; what it returns becomes the corresponding element of the result.",
                    aliases: &["transform"],
                    ty: ParameterType::func(vec![ParameterType::var("T")], ParameterType::var("U")),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::list_of(ParameterType::var("U")),
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_inline(lower_transform),
        }],
    });
}

/// `collections::transform(List OF T, FUNC(T) AS U) AS List OF U`: map each
/// element through `f`, appending the results to a pre-sized output list.
pub(crate) fn lower_transform(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let scratch9 = builder.temporary_vreg();
    let scratch17 = builder.temporary_vreg();
    let collection = args[0].clone();
    let Some(element_type) = typed_list_element_type(&collection.type_).cloned() else {
        return Err(format!(
            "native collection transform does not accept {}",
            collection.type_
        ));
    };
    let collection_slot = builder.allocate_stack_object("transform_collection", 8);
    builder.emit(abi::store_u64(
        &collection.location,
        abi::stack_pointer(),
        collection_slot,
    ));
    let action = args[1].clone();
    let output_type = typed_callable_return_type(&action.type_)
        .cloned()
        .ok_or_else(|| {
            format!(
                "native collection transform action must be a function, got {}",
                action.type_
            )
        })?;
    builder.require_direct_callable("transform", &action)?;
    let action_slot = builder.allocate_stack_object("transform_action", 8);
    builder.emit(abi::store_u64(
        &action.location,
        abi::stack_pointer(),
        action_slot,
    ));
    let output_list_type = ParameterType::list_of(output_type.clone());
    // Pre-size the output to the source's working set so the per-element
    // append never regrows the entry table (transform emits exactly
    // count(source) entries) — plan-25-B B2.
    let output = builder.lower_reserved_list(&output_list_type, collection_slot)?;
    let output_slot = builder.allocate_stack_object("transform_output", 8);
    let cursor_slot = builder.allocate_stack_object("transform_cursor", 8);
    let remaining_slot = builder.allocate_stack_object("transform_remaining", 8);
    builder.emit(abi::store_u64(
        &output.location,
        abi::stack_pointer(),
        output_slot,
    ));
    builder.initialize_collection_loop_slots(
        collection_slot,
        cursor_slot,
        remaining_slot,
        &element_type,
    );

    let inline = inline_record_callback(builder, &action, &element_type, &output_type);
    let loop_label = builder.label("transform_call_loop");
    let ok_label = builder.label("transform_call_ok");
    let done = builder.label("transform_call_done");
    builder.emit(abi::label(&loop_label));
    builder.emit(abi::load_u64(
        &scratch9,
        abi::stack_pointer(),
        remaining_slot,
    ));
    builder.emit(abi::compare_immediate(&scratch9, "0"));
    builder.emit(abi::branch_eq(&done));
    let item = builder.load_collection_loop_item(collection_slot, cursor_slot, &element_type)?;
    // bug-307: stash before the callback (calls clobber caller-saved registers).
    let free_slot = builder.allocate_stack_object("transform_item_free", 8);
    builder.emit(abi::store_u64(&item, abi::stack_pointer(), free_slot));
    if let Some(inline) = &inline {
        // bug-766 E: the callback's body is evaluated here, into the accumulator.
        emit_inline_record_callback(
            builder,
            inline,
            free_slot,
            output_slot,
            &output_list_type,
            &element_type,
        )?;
        builder.advance_collection_loop(cursor_slot, remaining_slot, &loop_label, &element_type);
        builder.emit(abi::label(&done));
        let result = builder.allocate_register();
        builder.emit(abi::load_u64(&result, abi::stack_pointer(), output_slot));
        // The evaluated body byte-copied each source element into the output, so
        // the output's edges still point at the source's children: give it its own.
        return builder.own_collection_payload_edges(ValueResult {
            origin: None,
            type_: output_list_type.clone(),
            location: Operand::from(result),
            text: format!("transform({}, {})", collection.type_, action.text),
        });
    }
    builder.emit(abi::move_register(&abi::argument_register(0)?, &item));
    builder.emit(abi::load_u64(&scratch17, abi::stack_pointer(), action_slot));
    builder.emit_direct_callable_branch(&scratch17);
    builder.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
    builder.emit(abi::branch_eq(&ok_label));
    // A failing callback: free the partial output list (a private, uniquely-
    // owned buffer) before routing the raw error to the inline-TRAP capture
    // point (plan-26-B); non-trapped, this is the same auto-propagating return.
    builder.emit_callback_failure_exit(
        &[(output_slot, output_list_type.clone())],
        Some((free_slot, &element_type)),
    )?;
    builder.emit(abi::label(&ok_label));

    let item_slot = builder.allocate_stack_object("transform_item", 8);
    builder.emit(abi::store_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        item_slot,
    ));
    // bug-307: only AFTER the callback's result is safely in its slot. The free
    // is a call and would otherwise destroy RESULT_VALUE_REGISTER before it was
    // stored. The appended value is that result, a separate allocation, so the
    // source item is not retained and is dead here.
    builder.free_collection_loop_item(free_slot, &element_type)?;
    // The output accumulator is a private, uniquely-owned buffer, so append
    // each transformed item in place with geometric headroom (plan-01 §4.2)
    // — amortized O(1) instead of the O(n) splice the singleton+insert did.
    builder.lower_list_append_in_place(output_slot, item_slot, &output_list_type, &output_type)?;
    // bug-569: the `FunctionRef` ABI owns the callback's RESULT as well as its
    // argument, and this is where that ownership ends. The append above copied
    // the result's payload bytes into the output buffer, so the returned block
    // has no remaining reader — and nothing freed it, so every `String` callback
    // leaked one block per element per call (56.8 -> 112.6 MB at 50k/100k passes
    // over an 8-element list). Only a `String` result is a standalone block;
    // every fixed-width `U` materializes nothing and this is a no-op for it.
    //
    // Guarded by a runtime pointer-identity check against the loop item, which
    // is `collections::reduce`'s model (`gen_memory.rs` compares the reducer's
    // result against both the item and the superseded accumulator before freeing
    // either). bug-562 makes a `String`-returning callback hand back a
    // solely-owned block on every return site, so the unguarded free would be
    // sound today; the guard is what keeps THIS free correct without depending
    // on that, for the one alias that has ever existed here — a callback that
    // returns its own bare parameter, i.e. the very block `free_collection_loop_item`
    // released above. Skipping the free on identity leaves exactly one.
    //
    // Emitted only when the item is itself a block: for a fixed-width element
    // the item register holds a scalar, which cannot compare equal to an arena
    // pointer, and the comparison would be pure noise in the loop.
    //
    // bug-677: the same holds for every flat block result — a record, a data
    // union, a collection — which the append byte-copies just the same, and which
    // leaked one block per element until `free_callback_result` covered it. A
    // non-`String` block element is an alias into the source's packed data, so the
    // identity guard there protects the SOURCE list from a callback that handed its
    // argument back.
    if builder.callback_result_is_block(&output_type) {
        if element_type == output_type {
            let kept = builder.label("transform_result_kept");
            let carried = builder.temporary_vreg();
            let produced = builder.temporary_vreg();
            builder.emit(abi::load_u64(&carried, abi::stack_pointer(), free_slot));
            builder.emit(abi::load_u64(&produced, abi::stack_pointer(), item_slot));
            builder.emit(abi::compare_registers(&carried, &produced));
            builder.emit(abi::branch_eq(&kept));
            builder.free_callback_result(item_slot, &output_type)?;
            builder.emit(abi::label(&kept));
        } else {
            builder.free_callback_result(item_slot, &output_type)?;
        }
    } else if builder.owns_graph(&output_type) {
        // A graph-owning result (a recursive value, a function value, or one holding
        // either) is the callback's fresh value: the callee copies anything it was
        // lent before returning it. The append took it over — a pointer payload
        // whole, an inline record or union payload by its bytes, whose children now
        // belong to the output — so only an inline payload's own block is left to
        // free. The output therefore owns its edges, and the dispatch must not copy
        // them again (`member_result_owns_its_graph`): that orphaned every callback
        // result's graph (bug-781 follow-up).
        if matches!(
            builder.payload_edge_shape(&output_type),
            Some(
                crate::codegen::memory::arena::builder_arena_transfer::PayloadEdgeShape::InlineRecord
                    | crate::codegen::memory::arena::builder_arena_transfer::PayloadEdgeShape::InlineUnion
            )
        ) {
            builder.emit_shallow_block_free(&output_type, item_slot)?;
        }
    }
    builder.advance_collection_loop(cursor_slot, remaining_slot, &loop_label, &element_type);
    builder.emit(abi::label(&done));
    let result = builder.allocate_register();
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), output_slot));
    Ok(ValueResult {
        origin: None,
        type_: output_list_type.clone(),
        location: Operand::from(result),
        text: format!("transform({}, {})", collection.type_, action.text),
    })
}

/// bug-766 E: a `transform` callback whose body the loop evaluates itself.
///
/// The callback is a literal `LAMBDA` or a direct `FUNC` reference whose whole
/// body is `RETURN param` or `RETURN WITH param { f := e, … }`, every `f` a scalar
/// field and every `e` a scalar expression over `param`'s scalar fields,
/// constants, globals and by-value captures — no call, no `String`, no block.
/// Called, such a callback builds its result as a fresh arena block that the
/// loop byte-copies into the accumulator and frees (`free_callback_result`): one
/// allocation per element. Evaluated here, the element is appended to the
/// accumulator straight from the source (the copy `WITH` makes) and the new
/// field values are stored over the appended copy's slots — the source element
/// is never written, and nothing is allocated per element.
struct InlineRecordCallback {
    /// The fresh builder local `param` is renamed to; bound to the loop item.
    local: String,
    /// `(field index, new value)` in source order, the value already rewritten
    /// into this frame (`param` → `local`, each capture → its captured value).
    updates: Vec<(usize, NirValue)>,
}

/// The [`InlineRecordCallback`] for `action`, or `None` when the call path must
/// run it. `element_type` must equal `output_type` (the callback returns its
/// parameter, updated or not) and be a record laid out with lookup entries.
fn inline_record_callback(
    builder: &CodeBuilder,
    action: &ValueResult,
    element_type: &ParameterType,
    output_type: &ParameterType,
) -> Option<InlineRecordCallback> {
    if element_type != output_type || list_entry_stride(element_type) == 0 {
        return None;
    }
    let fields = builder.type_model.record_fields.get(element_type)?.clone();
    let (name, captures): (&str, &[NirValue]) = match action.origin.as_ref()? {
        NirValue::FunctionRef { name, .. } => (name, &[]),
        NirValue::Closure { name, captures, .. } => (name, captures),
        _ => return None,
    };
    let function = *builder.functions.get(name)?;
    if function.isolated || function.params.len() != 1 {
        return None;
    }
    let param = &function.params[0];
    if param.type_ != *element_type {
        return None;
    }
    // A capturing `LAMBDA`'s body opens by binding each capture to a local
    // (`Bind k = Capture 0`); that prologue reads the environment and nothing
    // else, so it is dropped here and each such local read as its capture.
    let [prologue @ .., NirOp::Return { value: Some(body) }] = function.body.as_slice() else {
        return None;
    };
    let mut bound = Vec::with_capacity(prologue.len());
    for op in prologue {
        let NirOp::Bind {
            mutable: false,
            name,
            value: Some(value @ NirValue::Capture { by_ref: false, .. }),
            ..
        } = op
        else {
            return None;
        };
        bound.push((name.clone(), value.clone()));
    }
    // An error the body raises names the file the body was written in: a `FUNC`
    // from another file stamps its nodes with that file (as leaf inlining does).
    let file = if function.file == builder.current_file {
        0
    } else {
        source_file_id(&function.file)
    };
    // `$` is not an identifier character, so no program local shares the name;
    // no update value can hold a second `transform`, so one name serves every site.
    let local = "$transform_inline_item".to_string();
    let rewrite = InlineRewrite {
        builder,
        param: &param.name,
        local: &local,
        fields: &fields,
        captures,
        bound: &bound,
        file,
    };
    let updates = match body {
        NirValue::Local(name) if *name == param.name => Vec::new(),
        NirValue::WithUpdate {
            type_,
            target,
            updates,
        } if type_ == element_type
            && matches!(target.as_ref(), NirValue::Local(name) if *name == param.name) =>
        {
            let mut rewritten = Vec::with_capacity(updates.len());
            for update in updates {
                let index = fields
                    .iter()
                    .position(|(field, _)| *field == update.field)?;
                if !rewrite.scalar_field(&fields[index].1) {
                    return None;
                }
                rewritten.push((index, rewrite.value(&update.value)?));
            }
            rewritten
        }
        _ => return None,
    };
    Some(InlineRecordCallback { local, updates })
}

/// Rewrites a callback body's value into the calling frame, refusing (`None`)
/// every node whose lowering could allocate, call, or fail other than through an
/// inline raise (which `raw_result_capture` intercepts).
struct InlineRewrite<'r, 'b> {
    builder: &'r CodeBuilder<'b>,
    param: &'r str,
    local: &'r str,
    fields: &'r [(String, ParameterType)],
    captures: &'r [NirValue],
    /// The body's capture-prologue locals, each with the `Capture` it binds.
    bound: &'r [(String, NirValue)],
    file: u32,
}

impl InlineRewrite<'_, '_> {
    fn scalar_type(type_: &ParameterType) -> bool {
        matches!(
            type_,
            ParameterType::Integer
                | ParameterType::Float
                | ParameterType::Fixed
                | ParameterType::Money
                | ParameterType::Boolean
                | ParameterType::Byte
        )
    }

    /// A field stored in its own slot as a scalar value.
    fn scalar_field(&self, type_: &ParameterType) -> bool {
        Self::scalar_type(type_)
            && !self.builder.record_field_is_inlined(type_)
            && !self.builder.record_field_is_pointer(type_)
    }

    fn loc(&self, loc: NirSourceLoc) -> NirSourceLoc {
        if loc.file == 0 {
            NirSourceLoc {
                file: self.file,
                ..loc
            }
        } else {
            loc
        }
    }

    fn value(&self, value: &NirValue) -> Option<NirValue> {
        match value {
            NirValue::Const { type_, .. } | NirValue::Global { type_, .. }
                if Self::scalar_type(type_) =>
            {
                Some(value.clone())
            }
            NirValue::Capture {
                index,
                type_,
                by_ref: false,
            } if Self::scalar_type(type_) => match self.captures.get(*index)? {
                // The captured value, read where the closure read it: the calling
                // frame, which nothing between the capture and this loop writes.
                captured @ (NirValue::Local(_)
                | NirValue::Const { .. }
                | NirValue::Global { .. }
                | NirValue::Capture { by_ref: false, .. }) => Some(captured.clone()),
                _ => None,
            },
            NirValue::Local(name) if name != self.param => {
                let (_, capture) = self.bound.iter().find(|(local, _)| local == name)?;
                self.value(capture)
            }
            NirValue::MemberAccess { target, member } => match target.as_ref() {
                NirValue::Local(name) if name == self.param => {
                    let (_, type_) = self.fields.iter().find(|(field, _)| field == member)?;
                    self.scalar_field(type_).then(|| NirValue::MemberAccess {
                        target: Box::new(NirValue::Local(self.local.to_string())),
                        member: member.clone(),
                    })
                }
                _ => None,
            },
            NirValue::Binary {
                op,
                left,
                right,
                loc,
            } if *op != BinaryOp::Concat => Some(NirValue::Binary {
                op: *op,
                left: Box::new(self.value(left)?),
                right: Box::new(self.value(right)?),
                loc: self.loc(*loc),
            }),
            NirValue::Unary { op, operand, loc } if *op != UnaryOp::SizeOf => {
                Some(NirValue::Unary {
                    op: *op,
                    operand: Box::new(self.value(operand)?),
                    loc: self.loc(*loc),
                })
            }
            _ => None,
        }
    }
}

/// One element of the [`InlineRecordCallback`] loop: evaluate every new field
/// value against the loop item (source order, before anything is written — as
/// `WITH` does), append the item to the accumulator, then store the values over
/// the appended copy's field slots.
///
/// A value that raises joins the member's callback-failure exit exactly as a
/// failing call would: the partial accumulator is freed and the error routed to
/// the inline `TRAP` or the function's exit. The item is an alias into the
/// source list (a record element owns no fresh block), so it has nothing to free.
fn emit_inline_record_callback(
    builder: &mut CodeBuilder,
    inline: &InlineRecordCallback,
    item_slot: usize,
    output_slot: usize,
    output_list_type: &ParameterType,
    element_type: &ParameterType,
) -> Result<(), String> {
    let mut stores = Vec::with_capacity(inline.updates.len());
    if !inline.updates.is_empty() {
        let failed = builder.label("transform_inline_failed");
        let evaluated = builder.label("transform_inline_evaluated");
        let shadowed = builder.locals.insert(
            inline.local.clone(),
            LocalValue {
                type_: element_type.clone(),
                stack_offset: item_slot,
                constant: None,
                by_ref: false,
            },
        );
        let previous_capture = builder.raw_result_capture.replace(failed.clone());
        let lowered = evaluate_inline_updates(builder, &inline.updates, &mut stores);
        builder.raw_result_capture = previous_capture;
        match shadowed {
            Some(previous) => builder.locals.insert(inline.local.clone(), previous),
            None => builder.locals.remove(&inline.local),
        };
        lowered?;
        builder.emit(abi::branch(&evaluated));
        builder.emit(abi::label(&failed));
        builder.emit_callback_failure_exit(&[(output_slot, output_list_type.clone())], None)?;
        builder.emit(abi::label(&evaluated));
    }
    builder.lower_list_append_in_place(output_slot, item_slot, output_list_type, element_type)?;
    if stores.is_empty() {
        return Ok(());
    }
    // The appended copy is the accumulator's last element: its lookup entry
    // holds the payload's offset into the data region. The append may have moved
    // the buffer, so everything is read back from `output_slot`.
    let buffer = builder.allocate_register();
    let index = builder.allocate_register();
    let stride = builder.allocate_register();
    let entry = builder.allocate_register();
    let offset = builder.allocate_register();
    let element = builder.allocate_register();
    builder.emit(abi::load_u64(&buffer, abi::stack_pointer(), output_slot));
    builder.emit(abi::load_u64(&index, &buffer, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::subtract_immediate(&index, &index, 1));
    builder.emit(abi::move_immediate(
        &stride,
        "Integer",
        &list_entry_stride(element_type).to_string(),
    ));
    builder.emit(abi::multiply_registers(&entry, &index, &stride));
    builder.emit(abi::add_registers(&entry, &entry, &buffer));
    builder.emit(abi::load_u64(
        &offset,
        &entry,
        COLLECTION_HEADER_SIZE + COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
    ));
    builder.emit_collection_data_pointer_for(&element, &buffer, element_type);
    builder.emit(abi::add_registers(&element, &element, &offset));
    for (field, slot) in stores {
        let value = builder.allocate_register();
        builder.emit(abi::load_u64(&value, abi::stack_pointer(), slot));
        builder.emit(abi::store_u64(&value, &element, 8 * field));
    }
    Ok(())
}

/// Lower each update value (observed like a `WITH` field, plan-17) and spill it.
fn evaluate_inline_updates(
    builder: &mut CodeBuilder,
    updates: &[(usize, NirValue)],
    stores: &mut Vec<(usize, usize)>,
) -> Result<(), String> {
    for (field, value) in updates {
        let lowered = builder.lower_value(value)?;
        builder.observe_float(value, &lowered)?;
        let lowered = builder.materialize_value(lowered)?;
        let slot = builder.allocate_stack_object("transform_inline_field", 8);
        builder.emit(abi::store_u64(
            &lowered.location,
            abi::stack_pointer(),
            slot,
        ));
        stores.push((*field, slot));
    }
    Ok(())
}
