use super::*;

#[cfg(test)]
mod kind2_layout_tests {
    use super::*;

    /// Every element type the entry-free representation applies to, with its
    /// payload width, and every one it must NOT apply to.
    const FIXED_WIDTH: &[(&str, usize)] = &[
        ("Boolean", 1),
        ("Byte", 1),
        ("Scalar", 4),
        ("Integer", 8),
        ("Float", 8),
        ("Fixed", 8),
        ("Money", 8),
    ];
    const VARIABLE_WIDTH: &[&str] = &[
        "String",
        "List OF Integer",
        "List OF List OF Integer",
        "Map OF Integer TO Integer",
        "Unknown",
        "",
    ];

    /// The three functions that select the representation must never disagree.
    ///
    /// They are consulted at different sites — `list_entry_stride` by the
    /// allocator and by `emit_flat_block_size`, `list_block_kind` by the header
    /// writers, `kind2_payload_size` by element access and iteration. A block
    /// allocated under one answer and read or freed under another is bug-02: the
    /// arena free list is corrupted rather than a wrong value returned. This is
    /// the invariant that makes the "entry stride lever" safe.
    #[test]
    fn representation_selectors_agree() {
        let every_type = FIXED_WIDTH
            .iter()
            .map(|(element, _)| *element)
            .chain(VARIABLE_WIDTH.iter().copied());
        for spelling in every_type {
            let element = ParameterType::parse(spelling);
            let entry_free = list_entry_stride(&element) == 0;
            assert_eq!(
                entry_free,
                list_block_kind(&element) == COLLECTION_KIND_LIST_FIXED,
                "stride and kind disagree for {element:?}"
            );
            assert_eq!(
                entry_free,
                kind2_payload_size(&element).is_some(),
                "stride and payload size disagree for {element:?}"
            );
        }
    }

    /// The allocation size and the free size must agree, for the reason they can
    /// actually disagree: they read the stride off DIFFERENT type strings.
    ///
    /// `lower_collection_values` allocates `HEADER + count*stride + dataLen`
    /// using the first slot's ACTUAL element type; `emit_flat_block_size` frees
    /// `HEADER + capacity*stride + dataCapacity` using the element type parsed
    /// out of the collection's DECLARED type. When inference gives up and the
    /// declared type is `List OF Unknown`, those two are different answers —
    /// stride 0 versus stride 40 — and the free runs `capacity * 40` bytes past
    /// the end of the block. That is bug-02's failure mode and it is reachable
    /// from `[toByte(1), toByte(2)]`, so the placeholder is refined before it
    /// reaches either side.
    ///
    /// Asserting `alloc == free` with both sides computed the same way would be a
    /// tautology; this models each side from its own type string.
    #[test]
    fn alloc_size_matches_free_size() {
        for &(element, width) in FIXED_WIDTH {
            for declared in [format!("List OF {element}"), "List OF Unknown".to_string()] {
                let declared_type = ParameterType::declared(&declared);
                let laid_out = refined_list_literal_type(
                    &declared_type,
                    Some(&ParameterType::declared(element)),
                )
                .unwrap_or_else(|| declared_type.clone());
                for count in [0usize, 1, 2, 7, 1000] {
                    // Allocation: stride from the element's own type.
                    let allocated = COLLECTION_HEADER_SIZE
                        + count * list_entry_stride(&ParameterType::declared(element))
                        + count * width;
                    // Free: stride from the type the block was laid out as.
                    let free_element = typed_list_element_type(&laid_out)
                        .cloned()
                        .unwrap_or_else(|| ParameterType::named(""));
                    let freed = COLLECTION_HEADER_SIZE
                        + count * list_entry_stride(&free_element)
                        + count * width;
                    assert_eq!(
                        allocated, freed,
                        "alloc/free disagree for {count} x {element} declared as {declared}"
                    );
                    // And the entry-free size really is the payoff being claimed:
                    // 40 + N*width, not 40 + N*(40 + width).
                    assert_eq!(
                        allocated,
                        COLLECTION_HEADER_SIZE + count * width,
                        "{element} x{count} is not entry-free"
                    );
                }
            }
        }
    }

    /// The refinement fires only where it is needed: a declared element type that
    /// inference actually produced is never second-guessed, and an empty literal
    /// or a map has nothing to refine from.
    #[test]
    fn literal_type_refinement_is_narrow() {
        let refine = |declared: &str, element: Option<&str>| {
            refined_list_literal_type(
                &ParameterType::declared(declared),
                element.map(ParameterType::declared).as_ref(),
            )
            .map(|t| t.name().into_owned())
        };
        assert_eq!(
            refine("List OF Unknown", Some("Byte")).as_deref(),
            Some("List OF Byte")
        );
        assert_eq!(refine("List OF Unknown", None), None);
        assert_eq!(refine("List OF Byte", Some("Byte")), None);
        assert_eq!(refine("List OF String", Some("String")), None);
        assert_eq!(refine("Map OF Integer TO Integer", Some("Integer")), None);
    }

    /// A nested list keeps its lookup table: `List OF List OF Integer` is a list
    /// whose *elements* are variable-length blocks, so only the inner lists go
    /// entry-free. §3 flagged this as a risk concentration.
    #[test]
    fn nested_list_outer_keeps_entries() {
        assert_eq!(
            list_entry_stride(&ParameterType::declared("List OF List OF Integer")),
            COLLECTION_ENTRY_SIZE
        );
        assert_eq!(
            list_entry_stride(&ParameterType::declared("List OF Integer")),
            COLLECTION_ENTRY_SIZE
        );
        assert_eq!(list_entry_stride(&ParameterType::Integer), 0);
        assert_eq!(
            list_block_kind(&ParameterType::declared("List OF Integer")),
            COLLECTION_KIND_LIST
        );
    }

    /// A `RES` element marker is an ownership axis, not part of the value type,
    /// and a resource pointer is never a fixed-width payload.
    #[test]
    fn resource_elements_keep_entries() {
        assert_eq!(
            list_entry_stride(&ParameterType::declared("fs.File")),
            COLLECTION_ENTRY_SIZE
        );
        assert_eq!(
            list_entry_stride(&ParameterType::declared("RES File")),
            COLLECTION_ENTRY_SIZE
        );
    }
}

/// plan-114-B: the two predicates that replaced `type_is_flat`.
///
/// These assert the DIVERGENCE explicitly. The whole point of the split is that
/// "a `memcpy` copies this correctly within one thread" and "this block may be
/// relocated into another thread's arena" are different questions; for every
/// type that existed before a resource could sit in a record they coincide, so
/// only the resource-carrying cases can catch a regression that re-merges them.
#[cfg(test)]
mod flatness_split_tests {
    use super::*;

    /// A model with one record: `Holder { name AS String, handle AS RES fs.File }`.
    /// The source-level ban is still up (letter D lifts it), so the only way to
    /// reach this shape is to build the model by hand.
    fn model_with_res_field_record() -> TypeModel {
        let mut model = TypeModel::empty();
        model.record_fields.insert(
            ParameterType::declared("Holder"),
            vec![
                ("name".to_string(), ParameterType::String),
                ("handle".to_string(), ParameterType::parse("RES fs.File")),
            ],
        );
        model
    }

    /// The shapes that genuinely diverge: a `RES`-marked type used directly, and
    /// a record carrying one as a field. Both are memcpy-copyable (the slot holds
    /// one 8-byte pointer, and copying it aliases the resource, §15.6) and
    /// neither is arena-transferable (that pointer would arrive pointing into the
    /// sender's arena).
    ///
    /// C7: this list is deliberately SHORT, and the collection spellings are
    /// covered by `a_res_collection_is_a_block_of_aliases`.
    #[test]
    fn a_res_field_and_its_record_diverge_between_the_two_predicates() {
        let model = model_with_res_field_record();
        for spelling in ["RES fs.File", "Holder"] {
            let type_ = ParameterType::parse(spelling);
            assert!(
                type_is_memcpy_copyable(&model, &type_),
                "`{spelling}` must be memcpy-copyable: a handle slot is one \
                 8-byte pointer and copying it aliases the resource (§15.6)"
            );
            assert!(
                !type_is_arena_transferable(&model, &type_),
                "`{spelling}` must NOT be arena-transferable: the handle would \
                 arrive pointing into the sender's arena"
            );
        }
    }

    /// A `RES` **collection** diverges exactly as a record with a `RES` field
    /// does: it is a block of aliases, so a `memcpy` copies it correctly within a
    /// thread (each slot still names the one resource, §15.6) and it may not be
    /// relocated into another arena.
    ///
    /// `typed_list_element_type` / `typed_map_type_parts` strip the `RES` marker,
    /// so the walk meets the BARE resource as the payload; the collection arm
    /// answers for it there (`is_resource_alias_payload`), which is what puts a
    /// list of aliases under the ordinary copy-insertion and scope-drop rules
    /// every other flat collection follows.
    #[test]
    fn a_res_collection_is_a_block_of_aliases() {
        let model = model_with_res_field_record();
        for spelling in [
            "List OF RES fs.File",
            "Map OF String TO RES fs.File",
            "List OF List OF RES fs.File",
        ] {
            let type_ = ParameterType::parse(spelling);
            assert!(
                type_is_memcpy_copyable(&model, &type_),
                "`{spelling}` is a block of aliases: copying it copies pointers \
                 and duplicates no resource"
            );
            assert!(!type_is_arena_transferable(&model, &type_), "`{spelling}`");
        }
    }

    /// A bare resource nominal does **not** diverge — it is `false` for both,
    /// and that is deliberate (C6). `Res(inner)` marks a *slot inside* an
    /// enclosing block, which stays flat; a bare nominal is the value's own type,
    /// and the resource record it names is separately allocated with its own
    /// lifetime, so it is not a block anything may `memcpy` or `arena_free` as a
    /// unit.
    ///
    /// This is a regression test with teeth: answering `true` for the memcpy
    /// question here propagates through every structural arm — it made
    /// `Result OF fs.File` "flat", which made `is_freeable_flat_value` claim it
    /// and emit a `pending_temp` free that had never existed. That was a measured
    /// `.ncode` diff in a thread-free fixture, not a theoretical concern.
    #[test]
    fn a_bare_resource_nominal_is_flat_in_neither_mode() {
        let model = TypeModel::empty();
        let file = ParameterType::declared("fs.File");
        assert!(!type_is_memcpy_copyable(&model, &file));
        assert!(!type_is_arena_transferable(&model, &file));

        // And it must not leak through a structural arm into a wrapper type.
        let wrapped = ParameterType::ResultOf(Box::new(file));
        assert!(
            !type_is_memcpy_copyable(&model, &wrapped),
            "`Result OF fs.File` must not become flat — is_freeable_flat_value \
             would claim it and free a block that is not one"
        );
        assert!(!type_is_arena_transferable(&model, &wrapped));
    }

    /// The regression net for the split: every type that existed before a
    /// resource could sit in a record must answer IDENTICALLY to both
    /// predicates. If a refactor makes one of these diverge, the split has
    /// leaked into ordinary types.
    #[test]
    fn every_resource_free_type_answers_both_predicates_the_same() {
        let mut model = TypeModel::empty();
        model.record_fields.insert(
            ParameterType::declared("Plain"),
            vec![
                ("count".to_string(), ParameterType::Integer),
                ("label".to_string(), ParameterType::String),
            ],
        );
        // (spelling, expected answer for BOTH predicates)
        let cases: &[(&str, bool)] = &[
            ("Integer", true),
            ("Boolean", true),
            ("Byte", true),
            ("Float", true),
            ("Money", true),
            ("String", true),
            ("List OF Integer", true),
            ("List OF String", true),
            ("Map OF String TO Integer", true),
            ("List OF List OF Integer", true),
            ("#Result OF Integer", true),
            ("Plain", true),
            // `Error` is a pointer composite, so it is flat in neither mode.
            ("Error", false),
        ];
        for (spelling, expected) in cases {
            let type_ = ParameterType::parse(spelling);
            assert_eq!(
                type_is_memcpy_copyable(&model, &type_),
                *expected,
                "`{spelling}` memcpy-copyable"
            );
            assert_eq!(
                type_is_arena_transferable(&model, &type_),
                *expected,
                "`{spelling}` arena-transferable — must match memcpy-copyable \
                 for every resource-free type"
            );
        }
    }

    /// The cycle guard must survive the rewrite in both modes: a self-referential
    /// record is not a finite flat block and must stay a pointer.
    #[test]
    fn a_cyclic_record_is_flat_in_neither_mode() {
        let mut model = TypeModel::empty();
        model.record_fields.insert(
            ParameterType::declared("Node"),
            vec![("next".to_string(), ParameterType::declared("Node"))],
        );
        let node = ParameterType::declared("Node");
        assert!(!type_is_memcpy_copyable(&model, &node));
        assert!(!type_is_arena_transferable(&model, &node));
    }
}

/// plan-114-B Phase 2: the layout, copy, size and drop of a record carrying a
/// `RES` field — `Holder { name AS String, handle AS RES fs.File }`.
///
/// The source-level ban is still up (letter D lifts it), so no fixture can reach
/// this shape; the model is built by hand. Each test asserts the property at the
/// point codegen actually decides it, and names the emitter that consumes the
/// answer, so a change to either side breaks a test rather than silently
/// diverging.
#[cfg(test)]
mod res_field_record_layout_tests {
    use super::*;

    fn holder_model() -> (TypeModel, ParameterType) {
        let mut model = TypeModel::empty();
        let holder = ParameterType::declared("Holder");
        model.record_fields.insert(
            holder.clone(),
            vec![
                ("name".to_string(), ParameterType::String),
                ("handle".to_string(), ParameterType::parse("RES fs.File")),
            ],
        );
        (model, holder)
    }

    /// (a) The handle occupies a plain 8-byte value slot at `8*index`.
    ///
    /// `record_field_is_pointer` false + `record_field_is_inlined` false is
    /// exactly the classification `emit_build_inlined_record`'s `else` branch
    /// keys on (`memory/marshal/record.rs:163-168`), which emits
    /// `load [sp+field_slot]` / `store -> [record + 8*index]` — i.e. the field's
    /// lowered value, which for a resource is its record pointer.
    #[test]
    fn the_handle_field_is_a_plain_value_slot_not_a_pointer_or_inlined_block() {
        let (model, _) = holder_model();
        let handle = ParameterType::parse("RES fs.File");

        assert!(
            !record_field_is_pointer(&model, &handle),
            "a resource field must not be classified as a pointer composite — \
             that would give it a separate allocation instead of a handle slot"
        );
        assert!(
            !record_field_is_inlined(&model, &handle),
            "a resource field must not be inlined into the data region — its \
             slot holds the handle, not a block-relative offset"
        );
        // Contrast: the String field IS inlined, so the record really does
        // exercise both branches of the write loop.
        assert!(record_field_is_inlined(&model, &ParameterType::String));
    }

    /// (b) A `memcpy` of the block is a correct copy, and the copied handle word
    /// therefore EQUALS the source's — the copy aliases the one resource rather
    /// than duplicating it (§15.6).
    ///
    /// `copy_value_to_current_arena` routes to `copy_flat_block` on
    /// `type_is_arena_transferable`; `is_freeable_flat_value` and the in-thread
    /// layout sites key on `type_is_memcpy_copyable`. The pair below is what
    /// makes an in-thread copy a `memcpy` while a thread transfer refuses.
    #[test]
    fn the_record_copies_by_memcpy_but_may_not_cross_an_arena() {
        let (model, holder) = holder_model();
        assert!(
            type_is_memcpy_copyable(&model, &holder),
            "an in-thread copy of Holder is a memcpy: the handle word is copied \
             verbatim, aliasing the same resource record"
        );
        assert!(
            !type_is_arena_transferable(&model, &holder),
            "Holder must NOT be arena-transferable: the copied handle would \
             point into the sender's arena"
        );
    }

    /// (c) The block sizes to `8 * fields.len()` plus the inlined String's bytes
    /// — the resource field contributes exactly its 8 bytes and no sub-block.
    ///
    /// `emit_record_block_size_to_slot` (`:794`) starts at `8 * fields.len()`
    /// and `continue`s past every non-inlined field, so the fixed term below IS
    /// the resource field's whole contribution.
    #[test]
    fn the_handle_field_contributes_exactly_eight_bytes_and_no_sub_block() {
        let (model, holder) = holder_model();
        let fields = model.record_fields.get(&holder).cloned().unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(8 * fields.len(), 16, "the fixed slot region is 8 per field");

        // Exactly one field walks the inlined-sub-block path; the handle is
        // skipped, so the only variable term is the String's.
        let inlined: Vec<&str> = fields
            .iter()
            .filter(|(_, ft)| record_field_is_inlined(&model, ft))
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(
            inlined,
            vec!["name"],
            "only the String field is inlined; the handle contributes no sub-block"
        );
    }

    /// (d) Scope-drop frees the record block and nothing else.
    ///
    /// `is_freeable_flat_value` gates the generic owned-value `arena_free` on
    /// memcpy-copyability plus "is a record/String/collection/data-union/Result".
    /// Holder qualifies, so ONE `arena_free` reclaims the record block. The
    /// resource record itself is not reachable from that path — it is reclaimed
    /// by the resource's own `ActiveCleanup::Resource`, which is letter C's
    /// routing — so the drop must not touch it.
    #[test]
    fn the_record_block_is_freeable_but_the_resource_record_is_not() {
        let (model, holder) = holder_model();
        assert!(
            type_is_memcpy_copyable(&model, &holder) && model.record_fields.contains_key(&holder),
            "both terms of is_freeable_flat_value hold for Holder, so scope-drop \
             emits one arena_free for its block"
        );
        // The resource itself is NOT a freeable flat value: it is not a record,
        // String, collection, data union or Result, so the generic owned-value
        // path cannot reach it however the record is dropped.
        let handle = ParameterType::parse("RES fs.File");
        assert!(
            !model.record_fields.contains_key(&handle)
                && !typed_is_collection_type(&handle)
                && handle != ParameterType::String,
            "the resource field must not satisfy is_freeable_flat_value's second \
             term, or scope-drop would arena_free the resource record too"
        );
    }

    // ===================================================================
    // bug-546: a USER-declared resource is a resource to the flatness walk
    // ===================================================================

    /// A module declaring `RESOURCE Db CLOSE BY sql::close [THREAD_SENDABLE]`,
    /// as `TypeModel::from_module` builds it: the name in `resource_names`, and
    /// the sendable subset in `sendable_resource_names`.
    fn declared_resource_model(sendable: bool) -> (TypeModel, ParameterType) {
        let mut model = TypeModel::empty();
        let db = ParameterType::declared("Db");
        model.resource_names.insert(db.clone());
        if sendable {
            model.sendable_resource_names.insert(db.clone());
        }
        (model, db)
    }

    /// The defect itself. A user-declared resource used to match no arm of
    /// `flatness_walk` — not the built-in resource arm (registry-only), and not
    /// `record_field_is_pointer` in the `else` — so the walk answered `true` for
    /// BOTH modes. "Memcpy-copyable" routed a bare `Db` bind to
    /// `copy_flat_block`, which is the build failure the bug reports; the
    /// "arena-transferable" half is the silent one, because that predicate is
    /// what permits a thread transfer to relocate a block, and a relocated
    /// resource handle points into the sender's per-thread arena.
    #[test]
    fn a_user_declared_resource_is_neither_memcpy_copyable_nor_arena_transferable() {
        for sendable in [false, true] {
            let (model, db) = declared_resource_model(sendable);
            assert!(
                is_resource_nominal(&model, &db),
                "a declared RESOURCE must be a resource nominal to codegen"
            );
            assert!(
                !type_is_memcpy_copyable(&model, &db),
                "a resource record is separately allocated with its own close op, \
                 so it is not a copyable flat block (sendable={sendable})"
            );
            assert!(
                !type_is_arena_transferable(&model, &db),
                "and it may never be relocated wholesale into another arena \
                 (sendable={sendable})"
            );
        }
    }

    /// The `THREAD_SENDABLE` opt-in is what picks the arm in
    /// `emit_thread_copy_real`: the deep copy into the receiver's arena, or the
    /// move-only pointer carry. Codegen must not widen the declaration — the
    /// frontend rejects transferring a resource whose author did not opt in
    /// (`2-203-0063 TYPE_THREAD_NOT_SENDABLE`), and this predicate is the only
    /// thing keeping the backend agreeing with it.
    #[test]
    fn only_a_thread_sendable_declaration_reaches_the_deep_copy_arm() {
        let (plain, db) = declared_resource_model(false);
        assert!(
            !is_sendable_resource_nominal(&plain, &db),
            "a RESOURCE declared without THREAD_SENDABLE is not sendable"
        );
        let (sendable, db) = declared_resource_model(true);
        assert!(
            is_sendable_resource_nominal(&sendable, &db),
            "a RESOURCE declared THREAD_SENDABLE is"
        );
    }

    /// bug-479: a thread handle is flat in NEITHER mode, and used to be flat in
    /// BOTH by the same accident as a user-declared resource — no arm matched it,
    /// and `record_field_is_pointer` in the `else` has no arm either.
    ///
    /// The value is a pointer to a heap control block (plan-156-A) shared with a
    /// running worker, so "memcpy-copyable" is wrong (it sent an
    /// inline-TRAP'd `thread::start` result to `copy_flat_block`, the reported
    /// build failure) and "arena-transferable" is wrong AND silent — that
    /// predicate gates relocating a block into another thread's arena, and the
    /// copy would duplicate the block while the worker kept using the original.
    ///
    /// `ir::verify` has never treated a thread view as copyable data — both
    /// views are resources there (`is_resource_base`); this is codegen agreeing
    /// with it.
    #[test]
    fn a_thread_handle_is_flat_in_neither_mode() {
        let model = TypeModel::empty();
        for spelling in [
            "Thread OF String TO Integer",
            "ThreadWorker OF String TO Integer",
            "Thread OF String RES fs.File TO Integer",
        ] {
            let type_ = ParameterType::parse(spelling);
            assert!(
                matches!(type_, ParameterType::ThreadHandle { .. }),
                "`{spelling}` must parse to a ThreadHandle for this test to mean \
                 anything"
            );
            assert!(
                !type_is_memcpy_copyable(&model, &type_),
                "`{spelling}` is a handle to a block full of queue pointers, not a \
                 copyable flat block"
            );
            assert!(
                !type_is_arena_transferable(&model, &type_),
                "`{spelling}` may never be relocated into another arena"
            );
        }
    }

    /// And the propagation: a `Result OF Thread …` is what an inline `TRAP` on
    /// `thread::start` actually binds, so the payload's answer has to reach it.
    #[test]
    fn a_result_wrapping_a_thread_handle_is_flat_in_neither_mode() {
        let model = TypeModel::empty();
        let type_ = ParameterType::parse("#Result OF Thread OF String TO Integer");
        assert!(
            !type_is_memcpy_copyable(&model, &type_) && !type_is_arena_transferable(&model, &type_),
            "the ResultOf arm walks its payload, so a thread payload makes the \
             whole Result non-flat"
        );
    }

    /// The positive pin. Widening the two predicates must not change the answer
    /// for anything that already had one — the built-in resources on both sides
    /// of the sendable split, and the ordinary flat types whose classification
    /// the whole record/collection layout rests on.
    #[test]
    fn widening_the_resource_predicates_moves_no_pre_existing_answer() {
        let (model, _) = declared_resource_model(true);

        // Built-in resources: `fs.File` is sendable, `process.Process` is not
        // (their registry `sendable` bits), and neither is flat in either mode.
        for builtin in ["fs.File", "tcp.Socket", "process.Process"] {
            let type_ = ParameterType::parse(builtin);
            assert!(
                is_resource_nominal(&model, &type_),
                "{builtin} is still a resource nominal"
            );
            assert!(
                !type_is_memcpy_copyable(&model, &type_)
                    && !type_is_arena_transferable(&model, &type_),
                "{builtin}'s flatness must be unchanged"
            );
        }
        assert!(
            is_sendable_resource_nominal(&model, &ParameterType::parse("fs.File")),
            "fs.File keeps the registry's sendable bit"
        );
        assert!(
            !is_sendable_resource_nominal(&model, &ParameterType::parse("process.Process")),
            "process.Process keeps the registry's unsendable bit"
        );

        // Ordinary values are untouched: the walk never reaches a resource arm
        // for them, and a declared resource in the model must not change that.
        for flat in [
            "String",
            "Integer",
            "List OF String",
            "Map OF String TO Integer",
        ] {
            let type_ = ParameterType::parse(flat);
            assert!(
                !is_resource_nominal(&model, &type_),
                "{flat} is not a resource"
            );
            assert!(
                type_is_memcpy_copyable(&model, &type_)
                    && type_is_arena_transferable(&model, &type_),
                "{flat} stays flat in both modes"
            );
        }
    }
}
