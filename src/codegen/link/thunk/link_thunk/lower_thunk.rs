//! plan-176-O: lowering one `LINK` call thunk (`lower_link_thunk`). The link support blob,
//! `ThunkContext` and `lower_link_initializer` live in `codegen/link/thunk/link_thunk/mod.rs`.

use super::*;

/// Emit one marshaling thunk for a `LINK` function (plan-linker.md §12.2/§12.3).
pub(super) fn lower_link_thunk(
    function: &IrLinkFunction,
    link_cstructs: &[crate::ir::IrCStruct],
    record_fields: &HashMap<
        crate::types::ParameterType,
        Vec<(String, crate::types::ParameterType)>,
    >,
    ctx: ThunkContext,
    record_native_resources: &HashSet<crate::types::ParameterType>,
    is_close_op: bool,
) -> Result<CodeFunction, String> {
    let ThunkContext {
        index,
        globals_base,
        free_slot,
        max_buffer_bytes,
    } = ctx;
    let symbol = link_thunk_symbol(&function.alias, &function.name);
    let n_params = function.params.len();
    let m_slots = function.abi_slots.len();
    // Only SCALAR OUT slots occupy the 8-byte OUT region; a struct slot gets a
    // sized buffer in the struct region instead (plan-50-E). Counting structs here
    // would inflate the frame and, worse, desynchronize `expr_offsets` from the
    // staging loop's sequence.
    // plan-113: a CSTRUCT-named slot is a `ParameterType::Named`, never a `C` —
    // that namespace is open, so it stays a nominal and is matched by name here.
    let is_struct_ctype = |ctype: &crate::types::ParameterType| {
        link_cstructs
            .iter()
            .any(|c| c.alias == function.alias && ctype.is_named(&c.name))
    };
    let n_out = function
        .abi_slots
        .iter()
        .filter(|slot| slot.direction.writes_back() && !is_struct_ctype(&slot.ctype))
        .count();

    const STATUS_OFF: usize = 8;
    const CRET_OFF: usize = 16;
    // Scratch slot 24 is reserved for string-return marshaling (RET_OFF).
    let param_base = 32;
    let cslot_base = param_base + n_params * 8;
    let out_base = cslot_base + m_slots * 8;
    // One extra slot past the OUT buffers holds the floating-point return bits
    // (`d0`) when the native return is `CDouble`.
    let cretd_off = out_base + n_out * 8;

    // plan-50-E: a struct slot needs a sized, aligned buffer for the C struct
    // itself; the cslot holds its ADDRESS. Layouts are recomputed here from the
    // CSTRUCT's field ctypes — never transported — so a crafted package cannot
    // dictate an offset.
    let target = "";
    let cstruct_of = |ctype: &crate::types::ParameterType| -> Option<&crate::ir::IrCStruct> {
        link_cstructs
            .iter()
            .find(|c| c.alias == function.alias && ctype.is_named(&c.name))
    };
    // (slot index) -> (buffer offset, layout, the CSTRUCT)
    let mut struct_slots: Vec<(usize, usize, crate::ir::CLayout, &crate::ir::IrCStruct)> =
        Vec::new();
    let mut struct_cursor = cretd_off + 8;
    for (slot_idx, slot) in function.abi_slots.iter().enumerate() {
        let Some(decl) = cstruct_of(&slot.ctype) else {
            continue;
        };
        let fields: Vec<(String, crate::types::ParameterType)> = decl
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.ctype.clone()))
            .collect();
        let layout = crate::ir::compute_c_layout(&fields, target)?;
        struct_cursor = align(struct_cursor, layout.align);
        struct_slots.push((slot_idx, struct_cursor, layout.clone(), decl));
        struct_cursor += layout.size;
    }
    // plan-50-F: a record with `String` fields does NOT hold String
    // pointers — it INLINES each String's block into a trailing data region and
    // stores the block-relative OFFSET in the field slot. See
    // `record_field_is_inlined` / `emit_build_inlined_record`: every record
    // inlines its `String`s — plan-132 removed the last pointer-string records. So
    // the whole record must be built in one
    // allocation whose size depends on every field's strlen — which means every
    // length is measured BEFORE the record is allocated, and each needs a slot.
    //
    // Per CString field: [char* , len]. Then a cursor and the running total.
    let cstr_area = align(struct_cursor, 8);
    let n_cstr = struct_slots
        .iter()
        .map(|(_, _, _, decl)| {
            decl.fields
                .iter()
                .filter(|f| f.ctype.is_c_abi(CAbiType::Str))
                .count()
        })
        .max()
        .unwrap_or(0);
    let cursor_off = cstr_area + n_cstr * 16;
    let total_off = cursor_off + 8;
    // plan-53-A: two scratch slots for building a stateful native resource's
    // resource record after the call — one parks the native handle, one the record
    // pointer, across the `arena_alloc` that clobbers all caller-saved registers.
    let rec_handle_off = total_off + 8;
    let rec_ptr_off = rec_handle_off + 8;
    // plan-58-B: one scratch word per `OUT CBuffer` slot holding its byte
    // capacity `N`. It must be a FRAME word, not a register: the byte-list
    // allocation between computing `N` and using it destroys every caller-saved
    // register, and `emit_alloc_byte_list` reads the count from a frame offset
    // anyway. The block POINTER lives in the slot's ordinary `out_base` word.
    let cbuffer_slots: Vec<usize> = function
        .abi_slots
        .iter()
        .enumerate()
        .filter(|(_, slot)| slot.ctype.is_c_abi(CAbiType::Buffer))
        .map(|(idx, _)| idx)
        .collect();
    let cbuffer_size_base = rec_ptr_off + 8;
    // sec-02: a parallel per-CBuffer word holding the *physical* allocation size
    // (logical SIZE + BUFFER_GUARD_BYTES). `size_base` keeps the logical SIZE (for
    // the size gate and the post-call LENGTH clamp); `phys_base` is what
    // `emit_alloc_byte_list` actually reserves so the guard region exists.
    let cbuffer_phys_base = cbuffer_size_base + cbuffer_slots.len() * 8;
    let frame = align(cbuffer_phys_base + cbuffer_slots.len() * 8 + 24, 16);

    // plan-50-H: the wrapper's result is whatever `RETURN <expr>` names. A bare
    // `RETURN <slot>` (an `IrLinkExpr::Var`) selects that slot's value; anything
    // else is a computed expression. Both magic-name tests — `slot.name ==
    // "return"` and `abi_return_name == "return"` — are gone.
    let result_var: Option<&str> = match &function.result {
        Some(IrLinkExpr::Var(name)) => Some(name.as_str()),
        _ => None,
    };

    // §12.3/§12.4 boundary validations that this signature needs.
    // The C return is the result exactly when `RETURN` names it.
    let returns_value = result_var == Some(function.abi_return_name.as_str());
    // plan-50-E: a BIND IN field range-checks the same way a CInt32 argument does
    // (a 64-bit Integer that does not fit the C field fails with ErrOverflow
    // rather than truncating), so it needs the same `range_fail` block. Without
    // this the thunk branches to a label that is never emitted.
    let bind_in_needs_range = function.bind_in.iter().any(|bind| {
        let Some(slot) = function.abi_slots.iter().find(|s| s.name == bind.slot) else {
            return false;
        };
        let Some(decl) = link_cstructs
            .iter()
            .find(|c| c.alias == function.alias && slot.ctype.is_named(&c.name))
        else {
            return false;
        };
        bind.fields.iter().any(|field| {
            decl.fields
                .iter()
                .find(|f| f.name == field.name)
                .is_some_and(|f| f.ctype.c_abi().and_then(narrow_signed_bits).is_some())
        })
    });
    let needs_range = bind_in_needs_range
        || function.abi_slots.iter().any(|slot| {
            !slot.direction.writes_back()
                && slot.ctype.is_c_abi(CAbiType::Int32)
                && function.params.iter().any(|(name, _)| name == &slot.name)
        });
    // plan-50-F: a `CString` struct field is copied out with the same helper, so
    // it needs the same `encoding_fail` block. Without this the thunk branches to
    // a label that is never emitted.
    let struct_has_cstring_field = function.abi_slots.iter().any(|slot| {
        link_cstructs
            .iter()
            .find(|c| c.alias == function.alias && slot.ctype.is_named(&c.name))
            .is_some_and(|c| c.fields.iter().any(|f| f.ctype.is_c_abi(CAbiType::Str)))
    });
    let needs_encoding = struct_has_cstring_field
        || (returns_value
            && function.abi_return_ctype.is_c_abi(CAbiType::Ptr)
            && matches!(function.return_type, crate::types::ParameterType::String));
    // `needs_float` gates the `d0`-stash below (a `double` ABI *return* arrives
    // in `d0`, not `x0`), so it must stay narrow to the direct-return case.
    let returns_cfloat = returns_value && function.abi_return_ctype.is_c_abi(CAbiType::Float);
    let needs_float =
        returns_cfloat || returns_value && function.abi_return_ctype.is_c_abi(CAbiType::Double);
    // bug-409: two other marshalling paths branch to the same `nan_fail` /
    // `inf_fail` labels without the ABI return being a `CDouble`, so gating the
    // labels on `needs_float` alone left a dangling branch — a hard build failure
    // ("branch target label does not resolve"). Mirror `struct_has_cstring_field`:
    // emit the labels whenever any path can reach them.
    //
    // (1) `marshal_struct_out` emits the finiteness gate for every `CDouble`
    //     record field of the returned CSTRUCT (`RETURN structOut`, ABI return a
    //     status). Checked against the result struct's declared fields, the same
    //     way `struct_has_cstring_field` reads declared fields.
    let struct_result_has_cdouble_field = result_var
        .and_then(|name| {
            struct_slots
                .iter()
                .find(|(idx, ..)| function.abi_slots[*idx].name == name)
        })
        .is_some_and(|(_, _, _, decl)| {
            decl.fields
                .iter()
                .any(|f| f.ctype.is_c_abi(CAbiType::Double) || f.ctype.is_c_abi(CAbiType::Float))
        });
    // (2) a `CDouble` OUT-slot result (bug-238) runs the same finiteness gate in
    //     the OUT-slot marshalling arm (`RETURN dblOut`, ABI return a status).
    let out_result_is_cdouble = function.abi_slots.iter().any(|slot| {
        result_var == Some(slot.name.as_str())
            && slot.direction.writes_back()
            && (slot.ctype.is_c_abi(CAbiType::Double) || slot.ctype.is_c_abi(CAbiType::Float))
    });
    let needs_float_labels =
        needs_float || struct_result_has_cdouble_field || out_result_is_cdouble;
    let needs_float_narrow = function_narrows_cfloat(function, link_cstructs);

    let alloc_fail = format!("{symbol}_alloc_fail");
    let call_fail = format!("{symbol}_call_fail");
    let range_fail = format!("{symbol}_range_fail");
    let encoding_fail = format!("{symbol}_encoding_fail");
    let nan_fail = format!("{symbol}_nan_fail");
    let inf_fail = format!("{symbol}_inf_fail");
    // bug-788: narrowing a finite Float to a C `float` can overflow binary32.
    let float_overflow_fail = format!("{symbol}_float_overflow_fail");
    // plan-58-B: the runtime size gate for `BUFFER … SIZE`.
    let buffer_size_fail = format!("{symbol}_buffer_size_fail");
    // sec-02: the post-call canary-mismatch trap target (only emitted when the
    // function has an OUT CBuffer).
    let buffer_overrun = format!("{symbol}_buffer_overrun");
    let needs_buffer_size = !cbuffer_slots.is_empty();
    let done = format!("{symbol}_done");

    // Map wrapper-parameter name -> declared order (its incoming register).
    let param_index: HashMap<&str, usize> = function
        .params
        .iter()
        .enumerate()
        .map(|(idx, (name, _))| (name.as_str(), idx))
        .collect();
    // Map const-pin slot name -> immediate.
    let const_for: HashMap<&str, i64> = function
        .consts
        .iter()
        .map(|(name, value)| (name.as_str(), *value))
        .collect();

    // Vreg-allocated (plan-00-G Phase 2): the C-ABI marshaling slots are an
    // explicit `sp`-relative local region; x9/x10/x16 scratch become vregs the
    // allocator places (incoming wrapper args stay in their ABI x0-x7).
    let mut vregs = Vregs::new();
    let scratch = ThunkScratch::mint(&mut vregs);
    let ThunkScratch {
        t0,
        t1,
        t2,
        t3,
        t4,
        callee,
        ..
    } = &scratch;
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // Save incoming wrapper arguments before any clobbering call.
    for index in 0..n_params {
        instructions.push(abi::store_u64(
            &abi::argument_register(index)?,
            abi::stack_pointer(),
            param_base + index * 8,
        ));
    }

    // plan-59-B: the closed/moved guard, emitted HERE and nowhere else.
    //
    // Placement is the whole correctness argument. It must come after the
    // parameter spill above (the records are only reachable from frame words) and
    // before *any allocating* marshalling — the CBuffer staging below and
    // `emit_copy_string_to_cstring` in the main slot loop both allocate, and a
    // guard placed after either would leak the allocation on the error branch.
    // This point is the unique one satisfying both.
    //
    // One `!= 0` test catches closed AND moved: a moved record is flagged
    // `moved|closed` (= 3), so the non-zero test rejects both and the two are
    // separated only at the point of reporting (`error_constants.rs:721-745`,
    // mirroring `fs_helpers_io.rs:1431-1445`). Nothing here is per-binding: one
    // emission covers every `RES`-taking LINK function.
    let mut resource_guard_params: Vec<usize> = Vec::new();
    for (pidx, (_, type_)) in function.params.iter().enumerate() {
        if record_native_resources.contains(&type_.without_state()) {
            resource_guard_params.push(pidx);
        }
    }
    let resource_closed = format!("{symbol}_resource_closed");
    for pidx in &resource_guard_params {
        instructions.extend([
            abi::load_u64(t0, abi::stack_pointer(), param_base + pidx * 8),
            abi::load_u64(t0, t0, FILE_OFFSET_CLOSED),
            abi::compare_immediate(t0, "0"),
            abi::branch_ne(&resource_closed),
        ]);
    }

    // One label counter shared across the CBuffer SIZE expressions, the
    // SUCCESS_ON gate and the RESULT expression: a per-block counter restarted at
    // 0 in each produced two identically named labels (`{symbol}_cmp0_end`) when
    // two of them emitted a comparison/NOT — a duplicate the encoder rejects
    // outright (bug-79). Declared here, above the first emitter, rather than at
    // the SUCCESS_ON gate where it used to live.
    let mut counter = 0usize;

    // plan-58-B: stage every `OUT CBuffer` BEFORE the main slot loop.
    //
    // Every other slot kind stages into FRAME storage sized at compile time. A
    // CBuffer cannot: its size is a runtime value, and its storage must outlive
    // the call because it becomes the returned MFBASIC value. So it is an arena
    // block, and allocating one destroys every caller-saved register
    // (`_mfb_arena_alloc` has no survivor set — `.ai/compiler.md`).
    //
    // Doing it in a separate pass is what makes that safe: at this point the only
    // live state is in frame words (the wrapper's parameters, spilled on entry),
    // so there is nothing in a register for the allocation to destroy. Running it
    // inside the main loop would clobber slots already staged into registers by
    // earlier iterations. `tests/rt-behavior/native/cbuffer_read` pins this with
    // scalar slots staged on BOTH sides of the buffer.
    //
    // The offsets used here must agree with the main loop's `out_seq` sequence and
    // with `expr_offsets`, or every expression variable after the buffer resolves
    // to the wrong slot. All three walk `writes_back() && !is_struct_ctype`.
    let cbuffer_out_off = |target_idx: usize| -> usize {
        let mut seq = 0usize;
        for (idx, slot) in function.abi_slots.iter().enumerate() {
            if is_struct_ctype(&slot.ctype) {
                continue;
            }
            if !slot.direction.writes_back() {
                continue;
            }
            if idx == target_idx {
                return out_base + seq * 8;
            }
            seq += 1;
        }
        unreachable!("a CBuffer slot is always an OUT non-struct slot")
    };
    for (buf_seq, &slot_idx) in cbuffer_slots.iter().enumerate() {
        let slot = &function.abi_slots[slot_idx];
        let size_off = cbuffer_size_base + buf_seq * 8;
        let phys_off = cbuffer_phys_base + buf_seq * 8;
        let out_off = cbuffer_out_off(slot_idx);
        let cslot_off = cslot_base + slot_idx * 8;

        // `check_buffer_slots` guarantees exactly one clause per CBuffer slot, so
        // this cannot be absent in a well-formed function. A decoded `.mfp` does
        // not carry BUFFER clauses yet (plan-58-C), and `ir::verify` rejects such
        // a package through rule 2 — but that runs on the project, not here, so
        // fail loudly rather than allocating a zero-length buffer.
        let Some(buffer) = function.buffers.iter().find(|b| b.slot == slot.name) else {
            return Err(format!(
                "LINK function '{}.{}' CBuffer slot '{}' has no BUFFER SIZE clause",
                function.alias, function.name, slot.name
            ));
        };

        // A SIZE expression may read only wrapper parameters and CONST pins
        // (plan-58-A rule 9, tightened): those are the only values that exist
        // before the call. Parameters are already spilled to `param_base`; pin
        // immediates are materialized into their cslot words here, which the main
        // loop then writes again — harmless, and it keeps the pin's value in the
        // one place an expression can read it from.
        let mut size_offsets: HashMap<&str, usize> = HashMap::new();
        for (name, &pidx) in &param_index {
            size_offsets.insert(name, param_base + pidx * 8);
        }
        // Iterate the declared pin order (`function.consts` is a Vec), NOT
        // `const_for` (a HashMap): the body EMITS an instruction pair per pin, so
        // a hash-order walk leaked per-process iteration order into the `.ncode`
        // bytes whenever a single CBuffer function carried two or more matching
        // pins (bug-388, the bug-01 class). Each pin writes its own distinct
        // cslot word, so declaration order is semantically neutral and canonical.
        // `const_for` stays for the O(1) `.get()` lookup further below.
        for (pin_name, value) in &function.consts {
            let pin_name = pin_name.as_str();
            let Some(pin_idx) = function
                .abi_slots
                .iter()
                .position(|s| s.name.as_str() == pin_name)
            else {
                continue;
            };
            let pin_off = cslot_base + pin_idx * 8;
            instructions.extend([
                abi::move_immediate(t0, "Integer", &(*value as u64).to_string()),
                abi::store_u64(t0, abi::stack_pointer(), pin_off),
            ]);
            size_offsets.entry(pin_name).or_insert(pin_off);
        }

        let size_reg = emit_link_expr(
            &buffer.size,
            &size_offsets,
            &mut vregs,
            &symbol,
            &mut counter,
            &mut instructions,
        );
        instructions.push(abi::store_u64(&size_reg, abi::stack_pointer(), size_off));

        // Gate the size BEFORE allocating. A negative `N` would compute a nonsense
        // block size, and an unbounded one is a whole-arena request driven
        // straight from a wrapper parameter. Signed compares on both ends: an
        // unsigned compare would let a negative `N` read as enormous and pass the
        // lower bound.
        instructions.extend([
            abi::compare_immediate(&size_reg, "0"),
            abi::branch_lt(&buffer_size_fail),
            abi::move_immediate(t0, "Integer", &max_buffer_bytes.to_string()),
            abi::compare_registers(&size_reg, t0),
            abi::branch_gt(&buffer_size_fail),
        ]);

        // sec-02: reserve BUFFER_GUARD_BYTES past the logical SIZE. `phys_off`
        // (= SIZE + guard) is what `emit_alloc_byte_list` allocates and stores as
        // the block's capacity/dataCapacity; since `arena_free` sizes a byte list
        // from dataCapacity, the whole physical block (guard included) is
        // reclaimed on drop — no leak. `size_off` still holds the logical SIZE, so
        // the size gate above and the post-call LENGTH clamp are unaffected, and
        // the returned list view is still bounded to [0, SIZE].
        instructions.extend([
            abi::load_u64(t0, abi::stack_pointer(), size_off),
            abi::add_immediate(t0, t0, BUFFER_GUARD_BYTES),
            abi::store_u64(t0, abi::stack_pointer(), phys_off),
        ]);

        // Allocate the block and spill its pointer to the OUT word. The helper
        // writes the header (count/capacity/dataLength/dataCapacity all = phys)
        // and branches to `alloc_fail` on failure, so a wrapper with no LENGTH
        // clause needs no post-call work and the list is well-formed even if the
        // callee writes nothing.
        emit_alloc_byte_list(
            &symbol,
            &format!("cbuf{buf_seq}"),
            phys_off,
            out_off,
            &alloc_fail,
            &mut instructions,
            &mut relocations,
        );

        // The C function gets `dataBase`, NOT the block pointer. Two different
        // pointers 40 bytes apart: hand over the block and the callee overwrites
        // the header, which a short write corrupts only partially and therefore
        // plausibly. Reload the block from its frame word first — the allocation
        // destroyed every register.
        instructions.extend([
            abi::load_u64(t0, abi::stack_pointer(), out_off),
            abi::add_immediate(t0, t0, COLLECTION_HEADER_SIZE),
            abi::store_u64(t0, abi::stack_pointer(), cslot_off),
        ]);

        // sec-02: stamp the canary into the guard region
        // [dataBase + SIZE, dataBase + SIZE + BUFFER_GUARD_BYTES) — the last
        // BUFFER_GUARD_BYTES of the physical block. `t0` still holds dataBase. A
        // correct callee writes at most SIZE bytes, leaving the canary intact; an
        // overrun clobbers it (checked after the call).
        instructions.extend([
            abi::load_u64(t1, abi::stack_pointer(), size_off),
            abi::add_registers(t0, t0, t1), // canary base = dataBase + SIZE
            abi::move_immediate(t2, "Integer", BUFFER_CANARY_0),
            abi::store_u64(t2, t0, 0),
            abi::move_immediate(t2, "Integer", BUFFER_CANARY_1),
            abi::store_u64(t2, t0, 8),
        ]);
    }

    // Compute each C argument into its scratch slot. OUT buffers are addressed,
    // const pins are pinned, and ordinary params are marshaled per ABI type.
    let mut out_seq = 0usize;
    let mut result_out_off: Option<usize> = None;
    // The OUT-slot result's ctype drives the same marshalling the direct-return
    // path applies (bug-238); without it a `CInt32` OUT surfaced -1 as 4294967295.
    let mut result_out_ctype: Option<CAbiType> = None;
    for (slot_idx, slot) in function.abi_slots.iter().enumerate() {
        let cslot_off = cslot_base + slot_idx * 8;
        // plan-58-B: a CBuffer was fully staged by the pass above — block
        // allocated, pointer in its OUT word, `dataBase` in its cslot. Skip it
        // here, but ONLY after advancing `out_seq`: the sequence must match
        // `expr_offsets` and the staging pass, or every expression variable after
        // the buffer resolves to the wrong slot.
        //
        // Falling through instead would be silently destructive — the generic
        // scalar-OUT arm below overwrites the cslot with `&out_word` and ZEROES
        // the out word that now holds the block pointer, so the two stagings
        // clobber each other and the callee receives a pointer to frame memory.
        if slot.ctype.is_c_abi(CAbiType::Buffer) {
            if result_var == Some(slot.name.as_str()) {
                result_out_off = Some(out_base + out_seq * 8);
                result_out_ctype = slot.ctype.c_abi();
            }
            out_seq += 1;
            continue;
        }
        // plan-50-E: a struct slot passes the ADDRESS of a zeroed buffer, exactly
        // as an OUT scalar does — only sized.
        if let Some((_, buf_off, layout, decl)) =
            struct_slots.iter().find(|(idx, ..)| *idx == slot_idx)
        {
            // Zero the WHOLE buffer first. Mandatory, not hygiene: libsndfile
            // requires a zeroed SF_INFO for a non-RAW read, and an unzeroed buffer
            // leaks this thunk's stack into the C library. The tail uses narrower
            // stores so a struct whose size is not a multiple of 8 cannot write
            // past its own buffer into the next one.
            let mut z = 0usize;
            while z + 8 <= layout.size {
                instructions.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), buf_off + z));
                z += 8;
            }
            while z + 4 <= layout.size {
                instructions.push(abi::store_u32(abi::ZERO, abi::stack_pointer(), buf_off + z));
                z += 4;
            }
            while z + 2 <= layout.size {
                instructions.push(abi::store_u16(abi::ZERO, abi::stack_pointer(), buf_off + z));
                z += 2;
            }
            while z < layout.size {
                instructions.push(abi::store_u8(abi::ZERO, abi::stack_pointer(), buf_off + z));
                z += 1;
            }
            // Then the bound input fields; everything else stays zero.
            marshal_struct_in(
                function,
                decl,
                layout,
                *buf_off,
                &slot.name,
                &symbol,
                param_base,
                &param_index,
                &range_fail,
                &float_overflow_fail,
                &alloc_fail,
                &scratch,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::add_immediate(t0, abi::stack_pointer(), *buf_off),
                abi::store_u64(t0, abi::stack_pointer(), cslot_off),
            ]);
            continue;
        }
        if slot.direction.writes_back() {
            let out_off = out_base + out_seq * 8;
            out_seq += 1;
            instructions.extend([
                abi::store_u64(abi::ZERO, abi::stack_pointer(), out_off),
                abi::add_immediate(t0, abi::stack_pointer(), out_off),
                abi::store_u64(t0, abi::stack_pointer(), cslot_off),
            ]);
            if result_var == Some(slot.name.as_str()) {
                result_out_off = Some(out_off);
                result_out_ctype = slot.ctype.c_abi();
            }
        } else if let Some(value) = const_for.get(slot.name.as_str()) {
            // §12.3: a `CInt32` slot is a signed 32-bit C argument. A param feeding
            // this slot is range-checked at runtime (`range_fail` → `ErrOverflow`);
            // a `CONST` pin is known at compile time, so an out-of-range value is
            // rejected here rather than silently truncated to its low 32 bits
            // (bug-66).
            if slot.ctype.is_c_abi(CAbiType::Int32)
                && (*value < i64::from(i32::MIN) || *value > i64::from(i32::MAX))
            {
                return Err(format!(
                    "LINK function '{}.{}' CONST pin '{} = {}' does not fit the signed 32-bit \
                     range of its CInt32 ABI slot",
                    function.alias, function.name, slot.name, value
                ));
            }
            instructions.extend([
                abi::move_immediate(
                    t0,
                    "Integer",
                    &c_literal_bits(slot.ctype.c_abi(), *value).to_string(),
                ),
                abi::store_u64(t0, abi::stack_pointer(), cslot_off),
            ]);
        } else if let Some(&pidx) = param_index.get(slot.name.as_str()) {
            let param_off = param_base + pidx * 8;
            if slot.ctype.is_c_abi(CAbiType::Str) {
                emit_copy_string_to_cstring(
                    &symbol,
                    param_off,
                    cslot_off,
                    &alloc_fail,
                    &scratch,
                    &mut instructions,
                    &mut relocations,
                );
            } else if slot.ctype.is_c_abi(CAbiType::Int32) {
                // §12.3: the 64-bit MFBASIC Integer must fit signed 32-bit; an
                // out-of-range value fails rather than silently truncating.
                instructions.extend([
                    abi::load_u64(t0, abi::stack_pointer(), param_off),
                    abi::shift_left_immediate(t1, t0, 32),
                    abi::arithmetic_shift_right_immediate(t1, t1, 32),
                    abi::compare_registers(t0, t1),
                    abi::branch_ne(&range_fail),
                    abi::store_u64(t0, abi::stack_pointer(), cslot_off),
                ]);
            } else if slot.ctype.is_c_abi(CAbiType::Float) {
                // bug-788: a C `float` is binary32; the cslot holds its NaN-boxed
                // bits, ready for an FP argument register.
                instructions.push(abi::load_u64(t0, abi::stack_pointer(), param_off));
                emit_narrow_to_cfloat(t0, t1, t2, &float_overflow_fail, &mut instructions);
                instructions.push(abi::store_u64(t0, abi::stack_pointer(), cslot_off));
            } else if slot.ctype.is_c_abi(CAbiType::Ptr)
                && function
                    .params
                    .get(pidx)
                    .is_some_and(|(_, t)| record_native_resources.contains(&t.without_state()))
            {
                // plan-59-A: a param whose resource TYPE is a native resource is a
                // RECORD pointer, but the native symbol wants the handle it wraps.
                // Load FD@0. Record-ness is per-TYPE and no longer depends on
                // STATE: this fires for a BARE `db AS Db` param (e.g.
                // `close`/`exec`) just as it does for a stateful `SoundFile` —
                // without it the record pointer, not the handle, reaches the C
                // library.
                instructions.extend([
                    abi::load_u64(t0, abi::stack_pointer(), param_off),
                    abi::load_u64(t0, t0, FILE_OFFSET_FD),
                    abi::store_u64(t0, abi::stack_pointer(), cslot_off),
                ]);
            } else {
                instructions.extend([
                    abi::load_u64(t0, abi::stack_pointer(), param_off),
                    abi::store_u64(t0, abi::stack_pointer(), cslot_off),
                ]);
            }
        } else {
            return Err(format!(
                "LINK function '{}.{}' ABI slot '{}' has no source (param, const, or OUT)",
                function.alias, function.name, slot.name
            ));
        }
    }

    // Load the C arguments into their AAPCS64 registers, then call through the
    // resolved pointer.
    // bug-296: a LINK thunk calls a real C function through `blr`, so its
    // arguments follow the target's EXTERNAL C ABI, not the compiler's internal
    // 8-register model. On SysV x86-64 only six integer arguments are passed in
    // registers; the backend's `CALL_ARGS` extends the list with rax/rbp for
    // arguments 7 and 8, which is sound for the compiler's own calls but hands an
    // external callee two registers it never reads -- it takes those from the
    // stack, so it saw garbage, silently and with no diagnostic. Stack-argument
    // staging for external calls is the complete fix; until it exists, refuse the
    // call rather than emit one that is wrong.
    let external_int_registers = crate::codegen::engine::mir::active_backend()
        .register_model()
        .external_int_argument_registers();
    // An OUT slot passes the ADDRESS of its word, a pointer, whatever its ctype:
    // only a by-value floating slot travels in an FP register.
    let is_fp_slot = |slot: &crate::ir::IrAbiSlot| {
        !slot.direction.writes_back()
            && (slot.ctype.is_c_abi(CAbiType::Double) || slot.ctype.is_c_abi(CAbiType::Float))
    };
    // bug-788: Win64 assigns argument registers by POSITION — slot i goes in the
    // i-th integer register or the i-th xmm register, whichever its type takes —
    // where SysV, AAPCS64 and RISC-V count the two banks separately. Win64's four
    // register positions are shared by both banks, so every slot counts against
    // them.
    let positional = crate::codegen::engine::mir::active_backend()
        .register_model()
        .external_arguments_are_positional();
    let int_slot_count = if positional {
        function.abi_slots.len()
    } else {
        function
            .abi_slots
            .iter()
            .filter(|slot| !is_fp_slot(slot))
            .count()
    };
    if int_slot_count > external_int_registers {
        return Err(format!(
            "native function `{}` declares {int_slot_count} integer ABI slots, but this \
             target passes only {external_int_registers} integer arguments in registers and \
             stack arguments are not yet staged for native calls; reduce the slot count or \
             build for a target with more argument registers",
            function.name
        ));
    }
    let mut int_idx = 0usize;
    let mut flt_idx = 0usize;
    for (slot_idx, slot) in function.abi_slots.iter().enumerate() {
        let cslot_off = cslot_base + slot_idx * 8;
        if positional {
            int_idx = slot_idx;
            flt_idx = slot_idx;
        }
        // A `CFloat` cslot already holds NaN-boxed binary32 bits, so moving all
        // 64 bits leaves the single exactly where the callee reads it.
        if is_fp_slot(slot) {
            instructions.extend([
                abi::load_u64(t0, abi::stack_pointer(), cslot_off),
                abi::float_move_d_from_x(abi::fp_argument_register(flt_idx)?, t0),
            ]);
            flt_idx += 1;
        } else {
            instructions.push(abi::load_u64(
                &abi::argument_register(int_idx)?,
                abi::stack_pointer(),
                cslot_off,
            ));
            int_idx += 1;
        }
    }
    instructions.extend([
        abi::load_u64(
            callee,
            ARENA_STATE_REGISTER,
            slot_offset(globals_base, index),
        ),
        abi::branch_link_register(callee),
        // bug-431: the native `LINK` function is a real C call through `blr`, so
        // its integer/pointer result comes back in the C-ABI return register
        // (`rax` on x86-64, `x0`/`a0` elsewhere) — NOT the aligned MFB result
        // bank. On AArch64/RISC-V those coincide (`x0`/`a0`), which is why macOS
        // and Linux-aarch64 worked; on x86-64 the MFB result bank is `rdi` (SysV)
        // / `rcx` (Win64), so capturing `return_register()` read a caller-saved
        // register the callee had clobbered and every native return surfaced as
        // garbage (typically 0). Capture `c_return(0)` so the value is read from
        // where the C ABI actually leaves it.
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), CRET_OFF),
    ]);

    // sec-02: verify every OUT CBuffer's guard canary survived the call, BEFORE
    // the SUCCESS_ON gate or any result marshaling — so a callee that overran its
    // buffer cannot surface a "successful" result built from smashed arena
    // memory. A clobbered canary means the callee wrote past the declared SIZE;
    // trap with ErrNativeBufferOverrun. Uses integer scratch only (`t0`-`t2`), so
    // a float return still parked in d0 is untouched.
    for (buf_seq, &slot_idx) in cbuffer_slots.iter().enumerate() {
        let size_off = cbuffer_size_base + buf_seq * 8;
        let out_off = cbuffer_out_off(slot_idx);
        instructions.extend([
            abi::load_u64(t0, abi::stack_pointer(), out_off), // block
            abi::add_immediate(t0, t0, COLLECTION_HEADER_SIZE), // dataBase
            abi::load_u64(t1, abi::stack_pointer(), size_off), // SIZE
            abi::add_registers(t0, t0, t1),                   // canary base
            abi::load_u64(t1, t0, 0),
            abi::move_immediate(t2, "Integer", BUFFER_CANARY_0),
            abi::compare_registers(t1, t2),
            abi::branch_ne(&buffer_overrun),
            abi::load_u64(t1, t0, 8),
            abi::move_immediate(t2, "Integer", BUFFER_CANARY_1),
            abi::compare_registers(t1, t2),
            abi::branch_ne(&buffer_overrun),
        ]);
    }

    // plan-59-B: a registered `CLOSE BY` op sets `RESOURCE_CLOSED_BIT` here —
    // after the native call has returned, and BEFORE branching on its status.
    //
    // The ordering is bug-63's rule, mirrored from `fs_helpers_io.rs:1413-1420`:
    // a close that FAILS has still released the underlying handle, so leaving the
    // flag at 0 would let a later call sail past the guard and hand the C library
    // a dead pointer — or, for an fd-like handle, one the OS has since recycled.
    // The flag records "this handle was released", not "the release succeeded".
    //
    // Set unconditionally rather than OR-ed with the moved bit: a resource that
    // was transferred away is already refused by the guard above, so control never
    // reaches here for one.
    if is_close_op {
        for pidx in &resource_guard_params {
            instructions.extend([
                abi::load_u64(t0, abi::stack_pointer(), param_base + pidx * 8),
                abi::load_u64(t1, t0, FILE_OFFSET_CLOSED),
                abi::move_immediate(t2, "Integer", &(1u64 << RESOURCE_CLOSED_BIT).to_string()),
                abi::or_registers(t1, t1, t2),
                abi::store_u64(t1, t0, FILE_OFFSET_CLOSED),
            ]);
        }
    }
    if returns_cfloat {
        // bug-788: a `float` return arrives in `s0`; widen it in place first.
        instructions.push(abi::float_convert_d_from_s(
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[0],
        ));
    }
    if needs_float {
        // A `double` return arrives in `d0`, not `x0`; stash its bits.
        instructions.extend([
            abi::float_move_x_from_d(t0, abi::FP_SCRATCH[0]),
            abi::store_u64(t0, abi::stack_pointer(), cretd_off),
        ]);
    }

    // Derive the status value (sign-extending a 32-bit native return).
    instructions.push(abi::load_u64(t0, abi::stack_pointer(), CRET_OFF));
    if function.abi_return_ctype.is_c_abi(CAbiType::Int32) {
        instructions.extend([
            abi::shift_left_immediate(t0, t0, 32),
            abi::arithmetic_shift_right_immediate(t0, t0, 32),
        ]);
    }
    instructions.push(abi::store_u64(t0, abi::stack_pointer(), STATUS_OFF));

    // plan-50-I: every name a `Var` may hold, mapped to the frame slot holding
    // its value. The ABI return maps to STATUS_OFF, NOT CRET_OFF — STATUS_OFF
    // holds the sign-extended value, which is what `SUCCESS_ON status = -1` must
    // compare against.
    let mut expr_offsets: HashMap<&str, usize> = HashMap::new();
    expr_offsets.insert(function.abi_return_name.as_str(), STATUS_OFF);
    {
        let mut seq = 0usize;
        for (slot_idx, slot) in function.abi_slots.iter().enumerate() {
            // A struct slot holds an address, not a value an expression can read;
            // it is skipped so the OUT sequence matches the staging loop exactly.
            if is_struct_ctype(&slot.ctype) {
                continue;
            }
            let off = if slot.direction.writes_back() {
                let o = out_base + seq * 8;
                seq += 1;
                o
            } else {
                cslot_base + slot_idx * 8
            };
            // The ABI return name wins a collision: it is the older meaning and
            // `SUCCESS_ON status = 0` must keep resolving to the status.
            expr_offsets.entry(slot.name.as_str()).or_insert(off);
        }
    }

    // SUCCESS_ON gate: a failing status produces an Error result.
    if let Some(success) = &function.success_on {
        let value = emit_link_expr(
            success,
            &expr_offsets,
            &mut vregs,
            &symbol,
            &mut counter,
            &mut instructions,
        );
        instructions.extend([
            abi::compare_immediate(&value, "0"),
            abi::branch_eq(&call_fail),
        ]);
    }

    // Produce the wrapper result value in RESULT_VALUE_REGISTER (x1).
    // plan-50-E: a bare `RETURN <struct-slot>` builds the mapped record from the
    // slot's post-call buffer. Checked first — a struct slot is also `writes_back`,
    // so it would otherwise be mistaken for a scalar OUT.
    if let Some((_, buf_off, layout, decl)) = result_var.and_then(|name| {
        struct_slots
            .iter()
            .find(|(idx, ..)| function.abi_slots[*idx].name == name)
    }) {
        marshal_struct_out(
            function,
            decl,
            layout,
            *buf_off,
            record_fields,
            &symbol,
            cstr_area,
            cursor_off,
            total_off,
            &alloc_fail,
            &encoding_fail,
            &nan_fail,
            &inf_fail,
            &scratch,
            &mut instructions,
            &mut relocations,
        )?;
    } else if let Some(out_off) = result_out_off {
        // bug-238: an OUT-slot result carries the same C value shapes as a direct
        // return, so apply the same ctype-driven marshalling instead of a bare
        // 8-byte load — otherwise a `CInt32` OUT writing -1 surfaced as
        // 4294967295 (zero-extended) and a `CDouble` OUT bypassed the finiteness
        // rejection an MFBASIC `Float` requires.
        match result_out_ctype {
            Some(CAbiType::Int32) => {
                instructions.extend([
                    abi::load_u64(t0, abi::stack_pointer(), out_off),
                    abi::sign_extend_word(RESULT_VALUE_REGISTER, t0),
                ]);
            }
            Some(CAbiType::Bool) => {
                let set = format!("{symbol}_out_bool_true");
                let end = format!("{symbol}_out_bool_end");
                instructions.extend([
                    abi::load_u64(t0, abi::stack_pointer(), out_off),
                    abi::compare_immediate(t0, "0"),
                    abi::branch_ne(&set),
                    abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                    abi::branch(&end),
                    abi::label(&set),
                    abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
                    abi::label(&end),
                ]);
            }
            Some(CAbiType::Byte) => {
                instructions.extend([
                    abi::load_u64(t0, abi::stack_pointer(), out_off),
                    abi::move_immediate(t1, "Integer", "255"),
                    abi::and_registers(RESULT_VALUE_REGISTER, t0, t1),
                ]);
            }
            Some(ctype @ (CAbiType::Double | CAbiType::Float)) => {
                // Mirrors the direct-return finiteness gate: a non-finite double has
                // all exponent bits set; the mantissa distinguishes Inf from NaN.
                // bug-788: a C `float` OUT word holds 4 bytes; widen it first.
                let finite = format!("{symbol}_out_float_finite");
                if ctype == CAbiType::Float {
                    instructions.push(abi::load_u32(t0, abi::stack_pointer(), out_off));
                    emit_widen_from_cfloat(t0, t1, &mut instructions);
                } else {
                    instructions.push(abi::load_u64(t0, abi::stack_pointer(), out_off));
                }
                instructions.extend([
                    abi::move_immediate(t1, "Integer", F64_POSITIVE_INF_BITS),
                    abi::and_registers(t2, t0, t1),
                    abi::compare_registers(t2, t1),
                    abi::branch_ne(&finite),
                    abi::move_immediate(t3, "Integer", F64_MANTISSA_MASK),
                    abi::and_registers(t4, t0, t3),
                    abi::compare_immediate(t4, "0"),
                    abi::branch_eq(&inf_fail),
                    abi::branch(&nan_fail),
                    abi::label(&finite),
                    abi::move_register(RESULT_VALUE_REGISTER, t0),
                ]);
            }
            // plan-58-B: the OUT word holds the byte-list BLOCK pointer, which is
            // exactly the wrapper's `List OF Byte` result — so this is a plain
            // load, and the `_` default below would coincidentally do the same
            // thing.
            //
            // The arm is written out anyway. Relying on a silent default is how
            // bug-238 happened: that same `_` is why a `CInt32` OUT surfaced `-1`
            // as `4294967295`. An arm that agrees with the default today still
            // states the intent, and stops a future edit to the default from
            // silently changing what a CBuffer returns.
            Some(CAbiType::Buffer) => {
                // Truncate to what the callee actually wrote, then hand back the
                // block pointer — which IS the wrapper's `List OF Byte`.
                //
                // `LENGTH` is mandatory on a CBuffer (plan-58-B rule 10), so this
                // arm always has one to evaluate. Its expression is evaluated
                // HERE, after the call, which is why it may read the ABI return
                // and OUT slots that `BUFFER … SIZE` may not.
                let length = function
                    .result_length
                    .as_ref()
                    .expect("check_buffer_slots requires LENGTH on a returned CBuffer");
                let k = emit_link_expr(
                    length,
                    &expr_offsets,
                    &mut vregs,
                    &symbol,
                    &mut counter,
                    &mut instructions,
                );
                // Clamp to [0, N]. Not defensive padding: `pread`/`read` return
                // -1 on error and `sf_read_short` returns -1 or 0 at EOF. An
                // unclamped negative stored to `count` is a huge UNSIGNED value,
                // and every later collection read then walks off the block. An
                // over-capacity value does the same more slowly.
                // No counter suffix: a thunk has ONE `RETURN`, and rule 6 requires
                // a CBuffer to be the slot it names, so at most one CBuffer per
                // thunk reaches this arm and these labels cannot collide.
                let neg = format!("{symbol}_cbuf_len_neg");
                let capped = format!("{symbol}_cbuf_len_capped");
                let size_off = cbuffer_size_base
                    + cbuffer_slots
                        .iter()
                        .position(|&idx| {
                            function.abi_slots[idx].name.as_str() == result_var.unwrap_or_default()
                        })
                        .expect("the returned CBuffer is in cbuffer_slots")
                        * 8;
                instructions.extend([
                    abi::compare_immediate(&k, "0"),
                    abi::branch_lt(&neg),
                    // Reload N: the call clobbered everything.
                    abi::load_u64(t0, abi::stack_pointer(), size_off),
                    abi::compare_registers(&k, t0),
                    abi::branch_le(&capped),
                    abi::move_register(&k, t0),
                    abi::branch(&capped),
                    abi::label(&neg),
                    abi::move_immediate(&k, "Integer", "0"),
                    abi::label(&capped),
                ]);
                // `capacity`/`dataCapacity` deliberately stay at N. That is what
                // makes `arena_free` reclaim the whole block: `emit_flat_block_size`
                // sizes from capacity, so lowering it to k would leak the tail.
                // `05_collections.md:173-198` sanctions `capacity > count` as
                // headroom, and a value copy is shrink-to-fit.
                instructions.extend([
                    abi::load_u64(t1, abi::stack_pointer(), out_off),
                    abi::store_u64(&k, t1, COLLECTION_OFFSET_COUNT),
                    abi::store_u64(&k, t1, COLLECTION_OFFSET_DATA_LENGTH),
                    abi::move_register(RESULT_VALUE_REGISTER, t1),
                ]);
            }
            // Everything else is a plain 8-byte load of the OUT word. Enumerated
            // rather than left to a `_` (plan-113): that silent default is
            // exactly how bug-238 shipped — a `CInt32` OUT fell through it and
            // surfaced `-1` as `4294967295`. `None` is a CSTRUCT-named slot,
            // whose ctype is a `Named` and whose result is the buffer address.
            Some(
                CAbiType::Ptr
                | CAbiType::Str
                | CAbiType::Int8
                | CAbiType::Int16
                | CAbiType::Int64
                | CAbiType::UInt8
                | CAbiType::UInt16
                | CAbiType::UInt32
                | CAbiType::UInt64
                | CAbiType::Void,
            )
            | None => {
                instructions.push(abi::load_u64(
                    RESULT_VALUE_REGISTER,
                    abi::stack_pointer(),
                    out_off,
                ));
            }
        }
    } else if result_var == Some(function.abi_return_name.as_str()) {
        emit_return_passthrough(
            function,
            ReturnMarshal {
                cret_off: CRET_OFF,
                cretd_off,
                status_off: STATUS_OFF,
                alloc_fail: &alloc_fail,
                encoding_fail: &encoding_fail,
                nan_fail: &nan_fail,
                inf_fail: &inf_fail,
            },
            &symbol,
            &scratch,
            &mut instructions,
            &mut relocations,
        )?;
    } else if let Some(result) = &function.result {
        let value = emit_link_expr(
            result,
            &expr_offsets,
            &mut vregs,
            &symbol,
            &mut counter,
            &mut instructions,
        );
        instructions.push(abi::move_register(RESULT_VALUE_REGISTER, &value));
    } else {
        instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
    }

    // plan-59-A: a native func that produces `AS RES T` — with or without a
    // `STATE S` — hands back a resource RECORD, not the bare handle. Wrapping the
    // stateless case too is what gives it a `closed` flag at offset 16, which it
    // had nowhere to store while the handle itself was the value.
    // RESULT_VALUE_REGISTER currently holds
    // the native handle; wrap it in a resource record so the value the
    // caller binds is a pointer to the canonical plan-80 record {tag@0, FD@8,
    // CLOSED@16, STATE@24, buffers…} — the exact shape a built-in `fs.File STATE S`
    // uses, so `.state`, drop-reclamation (plan-52-B), and the closed guard all
    // work unchanged. STATE@24 is left NULL:
    // the caller's `RES x AS T STATE S = …` bind runs `emit_resource_state_init`,
    // which default-allocates the `S` record exactly as it does for a built-in
    // resource (or `BIND STATE` populates it first — plan-53-B). Without this the
    // handle IS the value and `.state` writes at offset 16 of the native handle's
    // own memory — memory corruption (the defect this fixes).
    if function.return_resource {
        instructions.extend([
            // Park the handle; alloc clobbers every caller-saved register.
            abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), rec_handle_off),
            abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
            abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        ]);
        emit_alloc(&symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            // record pointer (RET[1]) → scratch; reload handle; store handle@FD.
            abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), rec_ptr_off),
            abi::load_u64(t0, abi::stack_pointer(), rec_handle_off),
            abi::load_u64(t1, abi::stack_pointer(), rec_ptr_off),
            abi::store_u64(t0, t1, FILE_OFFSET_FD),
            // Canonical plan-80 header: tag@0 marks this a Native resource.
            abi::move_immediate(t0, "Integer", RESOURCE_TAG_NATIVE),
            abi::store_u64(t0, t1, RESOURCE_OFFSET_TAG),
            // Zero CLOSED (open) and the File I/O buffer words a native resource
            // never uses (they must be zero, not the arena-alloc's poison —
            // plan-52-B). STATE@24 is handled below.
            abi::store_u64(abi::ZERO, t1, FILE_OFFSET_CLOSED),
            // bug-755: a fresh record is unowned until a binding claims it.
            abi::store_u64(abi::ZERO, t1, RESOURCE_OFFSET_OWNER),
            abi::store_u64(abi::ZERO, t1, FILE_OFFSET_BUF_PTR),
            abi::store_u64(abi::ZERO, t1, FILE_OFFSET_BUF_FILLED),
            abi::store_u64(abi::ZERO, t1, FILE_OFFSET_BUF_ENABLED),
            abi::store_u64(abi::ZERO, t1, FILE_OFFSET_READ_PTR),
            abi::store_u64(abi::ZERO, t1, FILE_OFFSET_READ_POS),
            abi::store_u64(abi::ZERO, t1, FILE_OFFSET_READ_FILL),
            abi::store_u64(abi::ZERO, t1, FILE_OFFSET_READ_AT_EOF),
        ]);
        // STATE@24: `BIND STATE <res> = <out-struct>` (plan-53-B) marshals the OUT
        // struct the native call filled into an `S` record and stores its pointer;
        // otherwise leave it null so the caller's bind default-inits it (a built-in
        // `fs.File STATE S` works the same way — the producer never inits STATE).
        if let Some(struct_slot_name) = function.bind_state.as_deref() {
            let Some((_, buf_off, layout, decl)) = struct_slots
                .iter()
                .find(|(idx, ..)| function.abi_slots[*idx].name == struct_slot_name)
            else {
                return Err(format!(
                    "LINK function '{}.{}' BIND STATE names '{struct_slot_name}', which is not an OUT struct slot",
                    function.alias, function.name
                ));
            };
            // marshal_struct_out arena-allocates the `S` record from the post-call
            // buffer and leaves its pointer in RESULT_VALUE_REGISTER. It clobbers
            // scratch, so the record pointer is reloaded from its slot afterward.
            marshal_struct_out(
                function,
                decl,
                layout,
                *buf_off,
                record_fields,
                &symbol,
                cstr_area,
                cursor_off,
                total_off,
                &alloc_fail,
                &encoding_fail,
                &nan_fail,
                &inf_fail,
                &scratch,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64(t1, abi::stack_pointer(), rec_ptr_off),
                abi::store_u64(RESULT_VALUE_REGISTER, t1, FILE_OFFSET_STATE),
            ]);
        } else {
            instructions.extend([
                abi::load_u64(t1, abi::stack_pointer(), rec_ptr_off),
                abi::store_u64(abi::ZERO, t1, FILE_OFFSET_STATE),
            ]);
        }
        instructions.extend([
            // The record pointer is the wrapper result.
            abi::load_u64(t1, abi::stack_pointer(), rec_ptr_off),
            abi::move_register(RESULT_VALUE_REGISTER, t1),
        ]);
    }

    // FREE: release the caller-owned native return now that it is copied into the
    // owned wrapper result (17_native-libraries.md). The original pointer is still at
    // CRET_OFF; the deallocator (a C call) clobbers x0..x18, so the result value
    // is parked in the now-free STATUS slot across the call and reloaded after.
    // A NULL pointer is passed through unchanged — deallocators such as
    // sqlite3_free treat NULL as a no-op.
    if let Some(free_slot) = free_slot {
        // sec-01: the former source checker + ir::verify now reject `FREE` on an `AS RES`
        // producer, so this deallocation can never run for a resource-wrapping
        // thunk (which keeps the handle live in FD@0). Lock the invariant here so
        // any future path that reintroduces the combination trips in debug.
        assert!(
            !function.return_resource,
            "FREE on an AS RES producer must be rejected before codegen (sec-01)"
        );
        instructions.extend([
            abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STATUS_OFF),
            abi::load_u64(&abi::argument_register(0)?, abi::stack_pointer(), CRET_OFF),
            abi::load_u64(
                callee,
                ARENA_STATE_REGISTER,
                slot_offset(globals_base, free_slot),
            ),
            abi::branch_link_register(callee),
            abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STATUS_OFF),
        ]);
    }

    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // call_fail: SUCCESS_ON rejected the native status.
    instructions.push(abi::label(&call_fail));
    raise_error_into(
        &symbol,
        "ErrNativeBindingCallFailed",
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::branch(&done));

    // sec-02: buffer_overrun: a post-call canary check found an OUT CBuffer's
    // guard region clobbered — the callee wrote past its declared SIZE. Only
    // emitted when the function has an OUT CBuffer (otherwise the label is unused
    // and never branched to).
    if !cbuffer_slots.is_empty() {
        instructions.push(abi::label(&buffer_overrun));
        raise_error_into(
            &symbol,
            "ErrNativeBufferOverrun",
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::branch(&done));
    }

    // alloc_fail: a marshaling allocation failed.
    instructions.push(abi::label(&alloc_fail));
    raise_error_into(
        &symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );

    // plan-59-B: the guard's failure epilogue. Only emitted when some param is a
    // record resource, so a LINK function that takes none is byte-identical to
    // before this change.
    //
    // The two bits are split ONLY here, at the point of reporting — the guard
    // itself is a single `!= 0` test. A moved record carries `moved|closed`, so
    // testing the moved bit first is what stops a transferred handle being
    // misdescribed as "already closed" (`fs_helpers_io.rs:1431-1445`).
    if !resource_guard_params.is_empty() {
        let resource_moved = format!("{symbol}_resource_moved");
        instructions.extend([
            abi::branch(&done),
            abi::label(&resource_closed),
            // Re-load: the guard's compare register is long dead by here.
            abi::load_u64(
                t0,
                abi::stack_pointer(),
                param_base + resource_guard_params[0] * 8,
            ),
            abi::load_u64(t0, t0, FILE_OFFSET_CLOSED),
            abi::move_immediate(t1, "Integer", &(1u64 << RESOURCE_MOVED_BIT).to_string()),
            abi::and_registers(t0, t0, t1),
            abi::compare_immediate(t0, "0"),
            abi::branch_ne(&resource_moved),
        ]);
        raise_error_into(
            &symbol,
            "ErrResourceClosed",
            &mut instructions,
            &mut relocations,
        );
        instructions.extend([abi::branch(&done), abi::label(&resource_moved)]);
        raise_error_into(
            &symbol,
            "ErrResourceMoved",
            &mut instructions,
            &mut relocations,
        );
    }

    // Boundary-validation failure epilogues (plan-linker.md §12.3/§12.4), emitted
    // only when the signature can reach them.
    for (needed, label, error_name) in [
        (needs_range, &range_fail, "ErrOverflow"),
        (needs_encoding, &encoding_fail, "ErrEncoding"),
        (needs_float_labels, &nan_fail, "ErrFloatNaN"),
        (needs_float_labels, &inf_fail, "ErrFloatInf"),
        (needs_float_narrow, &float_overflow_fail, "ErrFloatOverflow"),
        (needs_buffer_size, &buffer_size_fail, "ErrInvalidArgument"),
    ] {
        if !needed {
            continue;
        }
        instructions.push(abi::branch(&done));
        instructions.push(abi::label(label));
        raise_error_into(&symbol, error_name, &mut instructions, &mut relocations);
    }

    // bug-371: every path out of the thunk must leave the error-origin register
    // (`RESULT_ERROR_SOURCE_REGISTER`) holding the no-origin sentinel. A thunk
    // has no MFBASIC source location to report, and none of the epilogues above
    // write x3 — but the register is also an argument register, so on every
    // failure it still holds whatever the marshaling staged for the native call.
    // A caller that consumes the loose error (an inline `TRAP`, which reads x3
    // and builds an `ErrorLoc` from it) then read a garbage pointer as a record
    // block: `ErrOutOfMemory` from a nonsense block size, or SIGSEGV. Zeroed at
    // `done` so the OK path is covered too, and so a new epilogue cannot forget.
    instructions.extend([
        abi::label(&done),
        abi::move_immediate(RESULT_ERROR_SOURCE_REGISTER, "Integer", "0"),
        abi::return_(),
    ]);

    let (frame_obj, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], frame);
    Ok(CodeFunction {
        name: format!("linker.{}.{}", function.alias, function.name),
        symbol,
        params: function
            .params
            .iter()
            .enumerate()
            .map(|(idx, (name, type_))| {
                Ok(CodeParam {
                    name: name.clone(),
                    type_: type_.name().into_owned(),
                    // The wrapper's incoming MFB argument register, as a role
                    // token — the thunk body saves from the same bank
                    // (plan-34-D; ≤8 params, enforced by `argument_register`).
                    location: abi::argument_register(idx)?.into(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
        returns: function.return_type.name().into_owned(),
        frame: frame_obj,
        stack_slots,
        instructions,
        relocations,
    })
}
