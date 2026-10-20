//! plan-176-U: the runtime-helper lowering (`lower_runtime_helper`), the arena and branch
//! emitters and the map/string-concat helpers. The builder types and shared imports live in
//! `engine/builder/mod.rs`.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_runtime_helper(
    symbol: &str,
    build_mode: crate::target::NativeBuildMode,
    module_name: &str,
    arena_layout: ArenaLayout,
    uses_rng: bool,
    type_model: &TypeModel,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    string_symbols: &HashMap<String, String>,
) -> Result<CodeFunction, String> {
    let Some(spec) = runtime::spec_for_symbol(symbol) else {
        return Err(format!(
            "native code plan does not emit runtime helper '{symbol}'"
        ));
    };
    // Every runtime helper lowers to the same CodeFunction shape — an empty
    // `params` list and the spec's return type — differing only in the
    // (frame, instructions, relocations, stack_slots) tuple each arm computes.
    // Build that tuple here, then construct the single CodeFunction after the
    // match (the shape the net.*/tls.* inner-match arms already used).
    let (frame, mut instructions, mut relocations, stack_slots) =
        if crate::codegen::registry::is_abi_function_call(spec.call) {
            // Any package's `AbiFunction` member (the unified successor to OsLower) is
            // wrapped once into its shared `_mfb_rt_*` helper by
            // `lower_abi_function_helper`, regardless of package prefix — so this is
            // checked before the per-package prefix arms below.
            lower_abi_function_helper(
                spec.call,
                symbol,
                build_mode,
                module_name,
                type_model,
                platform_imports,
                platform,
                string_symbols,
                arena_layout.term_state_offset,
                arena_layout.presentation_mode_offset,
                arena_layout.mouse_state_offset,
                arena_layout.input_state_offset,
                arena_layout.global_slots,
                uses_rng,
                arena_layout.debug_arena_registry,
            )?
        // (`term.*` members are `Body::abi_function` since the clean-room migration —
        // each member's own per-member body (`func_*.rs::lower_<name>`) calls the shared
        // `gen_shared::lower_term_helper`, which branches app-vs-console off the `AbiCtx`
        // (`build_mode`/`term_state_offset`/`presentation_mode_offset`) and, in app
        // mode, appends the platform's `emit_app_term_helper` sequence — so they route
        // through the `is_abi_function_call` branch above; no `term.` arm here. The
        // app-mode `ErrWrongMode` gate stays in the shared code layer, spliced by that
        // shared helper.)
        } else {
            match spec.call {
                // (`app.*` presentation-mode members are `Body::abi_function` since the
                // clean-room migration — each member's own per-member body reads the
                // arena `presentation_mode_offset` off the `AbiCtx` — so they route
                // through the `is_abi_function_call` branch above; no `app.` arm here.)
                // plan-67-B: internal perf-tracking helpers (injected, never NIR-level).
                "perf.init" | "perf.start" | "perf.end" | "perf.done" => {
                    perf::lower_perf_helper(spec.call, symbol, platform_imports, platform)?
                }
                // (`crypto.*` and `os.*` members are `Body::abi_function` since the
                // clean-room migration — each member owns its per-member body in its
                // `func_*.rs` (`lower_<name>`), so they route through the
                // `is_abi_function_call` branch above; no `crypto.`/`os.` arm here.)
                // (`io.*` members are `Body::abi_function` since plan-101, so they are
                // handled by the `is_abi_function_call` branch above — no `io.` arm here.)
                // (`thread.*` members are `Body::abi_function`/`abi_function_aliased` since
                // the clean-room migration — each member owns its per-member body in its
                // `func_*.rs` (`lower_<name>`), which calls the shared un-finalized
                // `gen_shared`/runtime-thread emitters and branches worker/parent + the
                // resource plane off `AbiCtx::call`; the internal `emit`/`read`/
                // the `*Resource`/`drop` code forms are `os_aliases`. So they
                // route through the `is_abi_function_call` branch above; no `thread.` arm
                // here. `thread.start` reads `AbiCtx::arena_global_slots`/`uses_rng`.)
                // (`net.*` members are `Body::abi_function_aliased` since the clean-room
                // migration — each member's own per-member body (`func_*.rs::lower_<name>`)
                // plus its `connectTcpAddr`/`pollList` code-form aliases — so they route
                // through the `is_abi_function_call` branch above; no `net.` arm here.)
                // (`audio.*` device-I/O members are `Body::abi_function_aliased` since the
                // clean-room migration — each member's own per-member body
                // (`func_*.rs::lower_<name>`) plus its IR-level overload-split code forms
                // (openInputDevice/openOutputDevice/readTimeout/pollTimeout/closeInput/
                // closeOutput) — so they route through the `is_abi_function_call` branch
                // above; no `audio.` arm here.)
                // Every `process.*` member is `Body::abi_function` since the
                // clean-room migration (the shared `per-member body plus
                // its `spawnEnv`/`sendTimeout`/… code-form aliases), so it routes
                // through the `is_abi_function_call` branch above. `process.__drop` is
                // the sole exception — it is not a descriptor member (the scope-drop op
                // synthesized during IR lowering), so it keeps its own family-branching
                // self-finalizing helper and this dedicated arm.
                "process.__drop" => crate::codegen::builtins::process::lower_process_drop_helper(
                    symbol,
                    platform_imports,
                    platform,
                )?,
                // (`tls.*` members are `Body::abi_function`/`abi_function_aliased` since
                // the clean-room migration — each member's own per-member body plus its
                // `pollList`/`closeListener` code-form aliases — so they route through the
                // `is_abi_function_call` branch above; no `tls.` arm here.)
                other => {
                    return Err(format!(
                        "native code plan does not emit runtime call '{other}'"
                    ));
                }
            }
        };
    // plan-62-E / plan-98-A: the console-reading `io::` helpers (`input`,
    // `readLine` and the keystroke readers) are gated in an app build that uses `app::` —
    // in a mode whose window input pipe has no producer, an ungated read would block
    // forever, so they raise the trappable `ErrWrongMode` instead. The requirement is
    // `WindowedMode`, not `Console`: `Canvas` has a window whose key events feed the
    // very same fd-0 pipe, so only `None` — which has no window at all — has nowhere
    // for input to come from. The writing side (`io::print`/`io::write`) is never
    // gated: it degrades to the fd sink in every mode.
    // No-op when the program cannot leave `Console` (`presentation_mode_offset` is
    // `None`). `term::` is gated separately at its own dispatch above, and keeps the
    // `Console`-only requirement — it needs the character grid, not just input.
    if matches!(
        spec.call,
        "io.input"
            | "io.readLine"
            | "io.readRaw"
            | "io.readScalar"
            | "io.readGrapheme"
            | "io.readKey"
    ) {
        app::prepend_wrong_mode_gate(
            &mut instructions,
            &mut relocations,
            symbol,
            arena_layout.presentation_mode_offset,
            app::ModeRequirement::WindowedMode,
        );
    }
    Ok(CodeFunction {
        name: format!("runtime.{}", spec.call),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: spec.abi.returns.to_string(),
        frame,
        stack_slots,
        instructions,
        relocations,
    })
}

/// Whether a runtime helper symbol belongs to the TLS **server** side
/// (`tls::listen`/`tls::accept`/the listener close). Gates the server-only
/// data objects and block trampolines so client-only programs keep their
/// exact pre-server native output (plan-06-tls-server.md §1 non-goals).
pub(crate) fn is_tls_server_symbol(symbol: &str) -> bool {
    matches!(
        symbol,
        "_mfb_rt_tls_tls_listen" | "_mfb_rt_tls_tls_accept" | "_mfb_rt_tls_tls_closeListener"
    )
}

/// Call `_mfb_arena_alloc` (size in `x0`, alignment in `x1`) and branch to
/// `fail` when it fails.
///
/// The free-function twin of `CodeBuilder::emit_arena_alloc_call`, for the
/// modules that emit into plain `Vec`s rather than through the builder. It lived
/// in `tls/mod.rs` as a `pub(crate)` item that the whole of `code/` already
/// resolved through its glob imports, while four sibling modules each defined a
/// byte-identical private copy that shadowed it (bug-322). Those are deleted;
/// this is the one definition.
pub(crate) fn emit_alloc(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    fail: &str,
) {
    instructions.push(abi::branch_link(ARENA_ALLOC_SYMBOL));
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(fail),
    ]);
}

/// `arena_free(x0 = ptr, x1 = size)` — return a compiler-sized allocation to the
/// arena. The caller preloads `x0`/`x1`; one internal-call relocation per (from,to)
/// covers all sites in `symbol`. Companion of [`emit_alloc`].
pub(crate) fn emit_arena_free(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::branch_link(ARENA_FREE_SYMBOL));
    relocations.push(internal_branch(symbol, ARENA_FREE_SYMBOL));
}

pub(crate) fn internal_branch(from: &str, to: &str) -> CodeRelocation {
    CodeRelocation {
        from: from.to_string(),
        to: to.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }
}

pub(crate) fn external_branch(
    from: &str,
    to: &str,
    platform_imports: &HashMap<String, String>,
) -> Result<CodeRelocation, String> {
    let library = platform_imports
        .get(to)
        .ok_or_else(|| format!("thread runtime helper requires {to} import"))?
        .clone();
    Ok(CodeRelocation {
        from: from.to_string(),
        to: to.to_string(),
        kind: RelocIntent::Call,
        binding: "external".to_string(),
        library: Some(library),
    })
}

/// Build the hash buckets for the `Map` whose pointer is in `x0`
/// (plan-02 Phase 6). Zeroes the `2*capacity`-entry bucket array that sits past
/// the data region, then for each lookup entry hashes its key bytes
/// (`dataBase + keyOffset`, `keyLength`; `map_hash::home_slot_of_key_bytes`) and
/// open-addresses `entryIndex + 1` into
/// the first empty bucket, finally setting the header "ready" flag. Preserves
/// `x0`/`x1`/`x2` (so it is safe to call from the probe); uses `x3`–`x17` scratch;
/// makes no calls. A `count == 0` map fills nothing.
pub(crate) fn lower_map_build_buckets_helper() -> CodeFunction {
    let symbol = MAP_BUILD_BUCKETS_SYMBOL;
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let zloop = format!("{symbol}_zloop");
    let zdone = format!("{symbol}_zdone");
    let eloop = format!("{symbol}_eloop");
    let edone = format!("{symbol}_edone");
    let enext = format!("{symbol}_enext");
    let hash = format!("{symbol}_hash");
    let ploop = format!("{symbol}_ploop");
    let place = format!("{symbol}_place");
    let nowrap = format!("{symbol}_nowrap");
    let mut vregs = Vregs::new();
    let v3 = vregs.next();
    let v4 = vregs.next();
    let v5 = vregs.next();
    let v6 = vregs.next();
    let v7 = vregs.next();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    let v16 = vregs.next();
    let v17 = vregs.next();
    let v18 = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        // Capture the map pointer (x0) into a vreg immediately: on x86 an
        // incoming param homes in rax/rdx by role, which the div/msub below
        // clobber — destroying it before the final store. As a vreg the
        // allocator keeps it in a safe (or spilled) location. AArch64 unaffected.
        abi::move_register(&v18, abi::c_arg(0)),
        // dataBase (v11) = map + HEADER + capacity*ENTRY ; bucketBase (v12) += dataCap.
        abi::load_u64(&v9, &v18, COLLECTION_OFFSET_COUNT),
        abi::load_u64(&v14, &v18, COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate(&v16, "Integer", &entry_size),
        abi::multiply_registers(&v11, &v14, &v16),
        abi::add_registers(&v11, &v11, &v18),
        abi::add_immediate(&v11, &v11, COLLECTION_HEADER_SIZE),
        abi::load_u64(&v15, &v18, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_registers(&v12, &v11, &v15),
        abi::shift_left_immediate(&v10, &v14, 1), // bucketCount = 2*capacity
        // Zero the bucket array, two slots a turn (bucketCount = 2*capacity is
        // even): its whole cost when the entries are few, and an index is rebuilt
        // after every grow and every tombstone compaction (bug-770).
        abi::move_register(&v7, &v12),
        abi::shift_left_immediate(&v13, &v10, 3),
        abi::add_registers(&v13, &v12, &v13), // end of the bucket array
        abi::label(&zloop),
        abi::compare_registers(&v7, &v13),
        abi::branch_eq(&zdone), // the array is a whole number of turns
        abi::store_u64(abi::ZERO, &v7, 0),
        abi::store_u64(abi::ZERO, &v7, 8),
        abi::add_immediate(&v7, &v7, 16),
        abi::branch(&zloop),
        abi::label(&zdone),
        // For each entry: hash its key, open-address its index+1. `v14` walks the
        // entries (bug-770: an add per entry, not a multiply).
        abi::move_immediate(&v13, "Integer", "0"),
        abi::add_immediate(&v14, &v18, COLLECTION_HEADER_SIZE),
        abi::subtract_immediate(&v14, &v14, COLLECTION_ENTRY_SIZE),
        abi::label(&eloop),
        abi::add_immediate(&v14, &v14, COLLECTION_ENTRY_SIZE), // entry addr
        abi::compare_registers(&v13, &v9),
        abi::branch_ge(&edone),
        // bug-760 B1: a tombstone has no bucket.
        abi::load_u8(&v3, &v14, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::compare_immediate(&v3, &COLLECTION_ENTRY_FLAG_REMOVED.to_string()),
        abi::branch_eq(&enext),
        abi::load_u64(&v15, &v14, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::add_registers(&v15, &v11, &v15), // keyPtr
        abi::load_u64(&v17, &v14, COLLECTION_ENTRY_OFFSET_KEY_LENGTH), // keyLen
    ];
    // slot (v4) = the key's home slot (bug-770: the one shared definition).
    instructions.extend(home_slot_of_key_bytes(
        &hash,
        &KeyHashRegs {
            key_ptr: v15,
            key_len: v17,
            bucket_count: v10,
            slot: v4,
            hash: v16,
            byte: v3,
            cursor: v5,
            remaining: v6,
        },
    ));
    instructions.extend([
        abi::label(&ploop),
        abi::shift_left_immediate(&v7, &v4, 3),
        abi::add_registers(&v7, &v12, &v7),
        abi::load_u64(&v6, &v7, 0),
        abi::compare_immediate(&v6, "0"),
        abi::branch_eq(&place),
        abi::add_immediate(&v4, &v4, 1),
        abi::compare_registers(&v4, &v10),
        abi::branch_lo(&nowrap),
        abi::move_immediate(&v4, "Integer", "0"),
        abi::label(&nowrap),
        abi::branch(&ploop),
        abi::label(&place),
        abi::add_immediate(&v6, &v13, 1),
        abi::store_u64(&v6, &v7, 0),
        abi::label(&enext),
        abi::add_immediate(&v13, &v13, 1),
        abi::branch(&eloop),
        abi::label(&edone),
        abi::move_immediate(&v6, "Integer", "1"),
        abi::store_u8(&v6, &v18, COLLECTION_OFFSET_BUCKETS_READY),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    CodeFunction {
        name: "runtime.mapBuildBuckets".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations: Vec::new(),
    }
}

/// Incrementally insert lookup entry `x1` of the `Map` in `x0` into its (already
/// built) bucket array (plan-02 Phase 6): hashes that entry's key and stores
/// `x1 + 1` into the first empty bucket. The `2*capacity` load factor guarantees a
/// free slot until the next capacity grow (which marks the index not-ready). Used
/// by the in-place `set` insert so building a map via repeated `set` stays O(n).
/// Makes no calls; preserves `x0`.
pub(crate) fn lower_map_bucket_put_helper() -> CodeFunction {
    let symbol = MAP_BUCKET_PUT_SYMBOL;
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let hash = format!("{symbol}_hash");
    let ploop = format!("{symbol}_ploop");
    let place = format!("{symbol}_place");
    let nowrap = format!("{symbol}_nowrap");
    let mut vregs = Vregs::new();
    let v3 = vregs.next();
    let v4 = vregs.next();
    let v5 = vregs.next();
    let v6 = vregs.next();
    let v7 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    let v16 = vregs.next();
    let v17 = vregs.next();
    let v20 = vregs.next();
    let v21 = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        // Capture map ptr (x0) and entry index (x1) into vregs before the div/msub
        // below clobber their x86 arg-register homes (rax/rdx). AArch64 unaffected.
        abi::move_register(&v20, abi::c_arg(0)),
        abi::move_register(&v21, abi::c_arg(1)),
        abi::load_u64(&v14, &v20, COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate(&v16, "Integer", &entry_size),
        abi::multiply_registers(&v11, &v14, &v16),
        abi::add_registers(&v11, &v11, &v20),
        abi::add_immediate(&v11, &v11, COLLECTION_HEADER_SIZE), // dataBase
        abi::load_u64(&v15, &v20, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_registers(&v12, &v11, &v15),     // bucketBase
        abi::shift_left_immediate(&v10, &v14, 1), // bucketCount
        // entry addr = map + HEADER + index*ENTRY.
        abi::move_immediate(&v16, "Integer", &entry_size),
        abi::multiply_registers(&v14, &v21, &v16),
        abi::add_registers(&v14, &v14, &v20),
        abi::add_immediate(&v14, &v14, COLLECTION_HEADER_SIZE),
        abi::load_u64(&v15, &v14, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::add_registers(&v15, &v11, &v15),
        abi::load_u64(&v17, &v14, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
    ];
    // slot (v4) = the key's home slot (bug-770: the one shared definition).
    instructions.extend(home_slot_of_key_bytes(
        &hash,
        &KeyHashRegs {
            key_ptr: v15,
            key_len: v17,
            bucket_count: v10,
            slot: v4,
            hash: v16,
            byte: v3,
            cursor: v5,
            remaining: v6,
        },
    ));
    instructions.extend([
        abi::label(&ploop),
        abi::shift_left_immediate(&v7, &v4, 3),
        abi::add_registers(&v7, &v12, &v7),
        abi::load_u64(&v6, &v7, 0),
        abi::compare_immediate(&v6, "0"),
        abi::branch_eq(&place),
        abi::add_immediate(&v4, &v4, 1),
        abi::compare_registers(&v4, &v10),
        abi::branch_lo(&nowrap),
        abi::move_immediate(&v4, "Integer", "0"),
        abi::label(&nowrap),
        abi::branch(&ploop),
        abi::label(&place),
        abi::add_immediate(&v6, &v21, 1),
        abi::store_u64(&v6, &v7, 0),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    CodeFunction {
        name: "runtime.mapBucketPut".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations: Vec::new(),
    }
}

/// Probe the `Map` in `x0` for the key whose bytes are `x1` (pointer) / `x2`
/// (length); returns the matching `entryIndex` in `x0`, or `-1` when absent
/// (plan-02 Phase 6). Lazily builds the buckets (calling
/// `_mfb_rt_map_build_buckets`) when the header "ready" flag is 0, so a freshly
/// allocated, copied, or grown map recomputes its index on first lookup. Key
/// equality is byte-wise over `keyLength` bytes — identical to the linear-scan
/// comparison it replaces. Has a one-slot frame to preserve the link register
/// across the build call.
/// `_mfb_rt_string_concat(left, right) -> block | 0` — the shared two-operand
/// `String` concatenation (plan-118-C).
///
/// `binop:Concat` was the single largest expansion category in the module:
/// 2,907,604 builder-emitted instructions over 17,221 sites, 169 each, because
/// every `a & b` inlined the length loads, the arena allocation, the header
/// store, and TWO byte-at-a-time copy loops. All of that is here once; a call
/// site becomes two argument moves, a `bl`, and a null check.
///
/// The copies are word-at-a-time where the alignment permits it. An arena block
/// is 8-aligned and both payloads start at `+8`, so copying the LEFT operand
/// keeps both cursors aligned for as long as 8 bytes remain. The RIGHT operand's
/// destination is only aligned when the left length is a multiple of 8, so that
/// case is tested at run time and falls back to bytes otherwise — rather than
/// issuing an unaligned word access, which a riscv64 target may service by a
/// kernel trap.
///
/// The allocation error stays at the call site: an `ErrOutOfMemory` carries the
/// `ErrorLoc` of the concatenation that failed, and a shared helper has no line
/// or column. So this returns a null pointer and the site raises.
pub(crate) fn lower_string_concat_helper() -> CodeFunction {
    let symbol = STRING_CONCAT_SYMBOL;
    let fail = format!("{symbol}_fail");
    let left_word = format!("{symbol}_left_word");
    let left_byte = format!("{symbol}_left_byte");
    let left_done = format!("{symbol}_left_done");
    let right_word = format!("{symbol}_right_word");
    let right_byte = format!("{symbol}_right_byte");
    let right_done = format!("{symbol}_right_done");
    let mut vregs = Vregs::new();
    let v20 = vregs.next();
    let v21 = vregs.next();
    let v22 = vregs.next();
    let v23 = vregs.next();
    let v24 = vregs.next();
    let v25 = vregs.next();
    let v26 = vregs.next();
    let v27 = vregs.next();
    let v28 = vregs.next();
    let v29 = vregs.next();
    let v30 = vregs.next();
    let v31 = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        // Park both operands before the allocation call: `_mfb_arena_alloc`
        // clobbers the argument registers (and on x86-64 rdx via its own
        // arithmetic), exactly as `lower_map_probe_helper` documents.
        abi::move_register(&v20, abi::c_arg(0)),
        abi::move_register(&v21, abi::c_arg(1)),
        abi::load_u64(&v22, &v20, 0), // left byteLength
        abi::load_u64(&v23, &v21, 0), // right byteLength
        abi::add_registers(&v24, &v22, &v23),
        // `mfb.string.v1` is `u64 byteLength; bytes; NUL`, so 8 + len + 1.
        abi::add_immediate(abi::c_arg(0), &v24, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&fail),
        abi::move_register(&v25, abi::mfb_return(1)), // new block
        abi::store_u64(&v24, &v25, 0),
        abi::add_immediate(&v26, &v25, 8), // write cursor
        abi::add_immediate(&v27, &v20, 8), // left payload
        abi::add_immediate(&v28, &v21, 8), // right payload
        // --- left operand: aligned words, then the tail ---
        abi::move_register(&v29, &v22),
        abi::label(&left_word),
        abi::compare_immediate(&v29, "8"),
        abi::branch_lo(&left_byte),
        abi::load_u64(&v30, &v27, 0),
        abi::store_u64(&v30, &v26, 0),
        abi::add_immediate(&v27, &v27, 8),
        abi::add_immediate(&v26, &v26, 8),
        abi::subtract_immediate(&v29, &v29, 8),
        abi::branch(&left_word),
        abi::label(&left_byte),
        abi::compare_immediate(&v29, "0"),
        abi::branch_eq(&left_done),
        abi::load_u8(&v30, &v27, 0),
        abi::store_u8(&v30, &v26, 0),
        abi::add_immediate(&v27, &v27, 1),
        abi::add_immediate(&v26, &v26, 1),
        abi::subtract_immediate(&v29, &v29, 1),
        abi::branch(&left_byte),
        abi::label(&left_done),
        // --- right operand: words only if the write cursor is still aligned ---
        abi::move_register(&v29, &v23),
        abi::move_immediate(&v31, "Integer", "7"),
        abi::and_registers(&v30, &v22, &v31),
        abi::compare_immediate(&v30, "0"),
        abi::branch_ne(&right_byte),
        abi::label(&right_word),
        abi::compare_immediate(&v29, "8"),
        abi::branch_lo(&right_byte),
        abi::load_u64(&v30, &v28, 0),
        abi::store_u64(&v30, &v26, 0),
        abi::add_immediate(&v28, &v28, 8),
        abi::add_immediate(&v26, &v26, 8),
        abi::subtract_immediate(&v29, &v29, 8),
        abi::branch(&right_word),
        abi::label(&right_byte),
        abi::compare_immediate(&v29, "0"),
        abi::branch_eq(&right_done),
        abi::load_u8(&v30, &v28, 0),
        abi::store_u8(&v30, &v26, 0),
        abi::add_immediate(&v28, &v28, 1),
        abi::add_immediate(&v26, &v26, 1),
        abi::subtract_immediate(&v29, &v29, 1),
        abi::branch(&right_byte),
        abi::label(&right_done),
        abi::move_immediate(&v30, "Integer", "0"),
        abi::store_u8(&v30, &v26, 0),
        abi::move_register(abi::mfb_return(0), &v25),
        abi::return_(),
        abi::label(&fail),
        abi::move_immediate(abi::mfb_return(0), "Integer", "0"),
        abi::return_(),
    ];
    let relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    CodeFunction {
        name: "runtime.stringConcat".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "String".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations,
    }
}

pub(crate) fn lower_map_probe_helper() -> CodeFunction {
    let symbol = MAP_PROBE_SYMBOL;
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let ready = format!("{symbol}_ready");
    let notfound = format!("{symbol}_notfound");
    let hash = format!("{symbol}_hash");
    let ploop = format!("{symbol}_ploop");
    let pnext = format!("{symbol}_pnext");
    let nowrap = format!("{symbol}_nowrap");
    let cloop = format!("{symbol}_cloop");
    let cmatch = format!("{symbol}_cmatch");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let v3 = vregs.next();
    let v4 = vregs.next();
    let v5 = vregs.next();
    let v6 = vregs.next();
    let v7 = vregs.next();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    let v16 = vregs.next();
    let v17 = vregs.next();
    let v20 = vregs.next();
    let v21 = vregs.next();
    let v22 = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        // Capture the params (map ptr / key ptr / key len) into vregs up front.
        // The lazy `bl build_buckets` below clobbers the SysV argument registers
        // on x86 (esp. rdx via its div), so x0/x1/x2 must not be read after it.
        // The build call still receives the map implicitly in rdi (unclobbered
        // until then). AArch64 unaffected.
        abi::move_register(&v20, abi::c_arg(0)),
        abi::move_register(&v21, abi::c_arg(1)),
        abi::move_register(&v22, abi::c_arg(2)),
        // Lazy build if not ready.
        abi::load_u8(&v9, &v20, COLLECTION_OFFSET_BUCKETS_READY),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&ready),
        abi::branch_link(MAP_BUILD_BUCKETS_SYMBOL),
        abi::label(&ready),
        abi::load_u64(&v9, &v20, COLLECTION_OFFSET_COUNT),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&notfound),
        abi::load_u64(&v14, &v20, COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate(&v16, "Integer", &entry_size),
        abi::multiply_registers(&v11, &v14, &v16),
        abi::add_registers(&v11, &v11, &v20),
        abi::add_immediate(&v11, &v11, COLLECTION_HEADER_SIZE), // dataBase
        abi::load_u64(&v15, &v20, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_registers(&v12, &v11, &v15),     // bucketBase
        abi::shift_left_immediate(&v10, &v14, 1), // bucketCount
    ];
    // slot (v4) = the query key's (x1/x2) home slot (bug-770: the one shared
    // definition).
    instructions.extend(home_slot_of_key_bytes(
        &hash,
        &KeyHashRegs {
            key_ptr: v21,
            key_len: v22,
            bucket_count: v10,
            slot: v4,
            hash: v16,
            byte: v3,
            cursor: v5,
            remaining: v6,
        },
    ));
    instructions.extend([
        abi::label(&ploop),
        abi::shift_left_immediate(&v7, &v4, 3),
        abi::add_registers(&v7, &v12, &v7),
        abi::load_u64(&v6, &v7, 0),
        abi::compare_immediate(&v6, "0"),
        abi::branch_eq(&notfound),
        abi::subtract_immediate(&v13, &v6, 1), // candidate idx
        abi::move_immediate(&v16, "Integer", &entry_size),
        abi::multiply_registers(&v15, &v13, &v16),
        abi::add_registers(&v15, &v15, &v20),
        abi::add_immediate(&v15, &v15, COLLECTION_HEADER_SIZE), // entry addr
        abi::load_u64(&v17, &v15, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::compare_registers(&v17, &v22),
        abi::branch_ne(&pnext),
        abi::load_u64(&v16, &v15, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::add_registers(&v16, &v11, &v16), // storedPtr
        abi::move_register(&v5, &v21),        // queryCursor
        abi::move_register(&v6, &v22),        // remaining
        abi::label(&cloop),
        abi::compare_immediate(&v6, "0"),
        abi::branch_eq(&cmatch),
        abi::load_u8(&v3, &v5, 0),
        abi::load_u8(&v17, &v16, 0),
        abi::compare_registers(&v3, &v17),
        abi::branch_ne(&pnext),
        abi::add_immediate(&v5, &v5, 1),
        abi::add_immediate(&v16, &v16, 1),
        abi::subtract_immediate(&v6, &v6, 1),
        abi::branch(&cloop),
        abi::label(&cmatch),
        abi::move_register(abi::mfb_return(0), &v13),
        abi::branch(&done),
        abi::label(&pnext),
        abi::add_immediate(&v4, &v4, 1),
        abi::compare_registers(&v4, &v10),
        abi::branch_lo(&nowrap),
        abi::move_immediate(&v4, "Integer", "0"),
        abi::label(&nowrap),
        abi::branch(&ploop),
        abi::label(&notfound),
        abi::move_immediate(abi::mfb_return(0), "Integer", "0"),
        abi::subtract_immediate(abi::mfb_return(0), abi::mfb_return(0), 1), // -1
        abi::label(&done),
        abi::return_(),
    ]);
    let relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: MAP_BUILD_BUCKETS_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    CodeFunction {
        name: "runtime.mapProbe".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations,
    }
}
