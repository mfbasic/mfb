// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

mod access;
mod construct;
mod payload_copy;
mod records;
mod type_predicates;
pub(crate) use type_predicates::*;

/// The payload size, in bytes, of a list element type whose payloads are
/// fixed-width — and which therefore may be addressed as `dataBase + i * size`.
///
/// A `Some` here is a promise that bug-365's ordering invariant holds: for such
/// a list `entry[i].valueOffset == i * size` after **every** operation, so
/// walking the data region linearly visits elements in index order.
/// `lower_list_insert_collection`, `lower_list_prepend_in_place` and
/// `lower_collection_set` are what maintain it; every other list operation
/// either preserves order or rebuilds in order. The per-operation table lives in
/// `src/docs/spec/memory/05_collections.md`, *Payload Order*.
///
/// `None` covers the variable-width types — `String`, records, unions, nested
/// collections — which keep the offset-stable scheme (plan-01 §4.1) and must be
/// read through `entry[i].valueOffset`. A linear stride is not even expressible
/// for them, which is why no reader got those wrong.
///
/// Deliberately excludes function values and pointer payloads. Both are 8-byte
/// fixed and would qualify on every ownership question plan-57-E asked — the
/// block is freed wholesale, transfer never walks entries for them, and a
/// function value is a bare pointer with reference semantics. Widening was
/// nonetheless declined, on coverage rather than analysis: `List OF FUNC` is
/// exercised by one fixture through one operation (`collections::get`), so the
/// suite passing proves nothing about the other twenty. See plan-57-E §4.2 for
/// the criterion that would reopen it.
///
/// Must agree with `CodeBuilder::collection_payload_alignment` for every arm;
/// `fixed_width_agrees_with_payload_alignment` asserts that so the two cannot
/// drift.
pub(crate) fn list_element_is_fixed_width(element_type: &ParameterType) -> Option<usize> {
    match element_type {
        ParameterType::Nothing | ParameterType::Boolean | ParameterType::Byte => Some(1),
        // `Scalar` is a bare nominal, not a variant (it is a Unicode scalar
        // value, `Named("Scalar")`), so it is matched by name rather than shape.
        type_ if type_.is_named("Scalar") => Some(4),
        ParameterType::Integer
        | ParameterType::Float
        | ParameterType::Fixed
        | ParameterType::Money => Some(8),
        _ => None,
    }
}

/// Append the data-region base computation for `collection` into `out`:
/// `collection + COLLECTION_HEADER_SIZE + capacity * COLLECTION_ENTRY_SIZE`.
///
/// **Capacity, never count.** An append-built list has spare capacity, and the
/// `LookupEntry[capacity]` array precedes the data region, so a count-based base
/// reads the spare entry slots as payload bytes. That trap is documented at
/// several call sites individually; this is the one place it has to be right.
///
/// The free-function form exists because ~14 sites compute this inside
/// standalone `CodeFunction` emitters that have no `CodeBuilder`
/// (`os.rs`, `fs_helpers_*`, `net/io.rs`, `tls/*`, `audio/*`, `crypto*`). That
/// structural split is why a single helper never absorbed them.
/// [`CodeBuilder::emit_collection_data_pointer`] delegates here.
///
/// Every register is a parameter so a site can keep its own register choice and
/// stay byte-identical when it is converted.
pub(crate) fn push_collection_data_pointer_into(
    out: &mut Vec<CodeInstruction>,
    dst: impl Into<abi::Reg>,
    collection: impl Into<abi::Reg>,
    scratch_capacity: impl Into<abi::Reg>,
    scratch_entry_size: impl Into<abi::Reg>,
) {
    let dst = dst.into();
    let collection = collection.into();
    let scratch_capacity = scratch_capacity.into();
    let scratch_entry_size = scratch_entry_size.into();
    out.push(abi::move_register(scratch_capacity, collection));
    out.push(abi::add_immediate(dst, collection, COLLECTION_HEADER_SIZE));
    out.push(abi::load_u64(
        scratch_capacity,
        scratch_capacity,
        COLLECTION_OFFSET_CAPACITY,
    ));
    out.push(abi::move_immediate(
        scratch_entry_size,
        "Integer",
        &COLLECTION_ENTRY_SIZE.to_string(),
    ));
    out.push(abi::multiply_registers(
        scratch_capacity,
        scratch_capacity,
        scratch_entry_size,
    ));
    out.push(abi::add_registers(dst, dst, scratch_capacity));
}

/// Append the data-region base computation in the form the standalone runtime
/// emitters use: `dst = collection + HEADER + capacity * ENTRY_SIZE`, computed
/// as a product first and the header folded in afterwards.
///
/// A second shape rather than a second copy of the rule.
/// [`push_collection_data_pointer_into`] computes the same address in the order
/// `CodeBuilder` emits it; these two orders are what exist in the tree, and
/// forcing one site into the other's order would change its emitted bytes for
/// no benefit. Both are edited together by plan-57-D, which is the point —
/// fourteen open-coded copies become two.
///
/// `scratch_product` may alias `scratch_entry_size` and/or `dst`; the sites in
/// `audio/` do exactly that.
pub(crate) fn push_collection_data_base_from_capacity(
    out: &mut Vec<CodeInstruction>,
    dst: impl Into<abi::Reg>,
    collection: impl Into<abi::Reg>,
    scratch_capacity: impl Into<abi::Reg>,
    scratch_entry_size: impl Into<abi::Reg>,
    scratch_product: impl Into<abi::Reg>,
) {
    let dst = dst.into();
    let collection = collection.into();
    let scratch_capacity = scratch_capacity.into();
    let scratch_entry_size = scratch_entry_size.into();
    let scratch_product = scratch_product.into();
    out.push(abi::load_u64(
        scratch_capacity,
        collection,
        COLLECTION_OFFSET_CAPACITY,
    ));
    // Every caller of this helper addresses a `List OF Byte` (net write/sendTo,
    // both TLS backends, both audio backends), so the stride is the byte-list
    // stride: zero once kind-2 drops the entry array, which collapses the data
    // base to `collection + HEADER` (plan-57-D). The multiply is left in place
    // rather than special-cased — `capacity * 0` is already the right answer, and
    // keeping one shape means the flag-off encoding stays byte-identical.
    out.push(abi::move_immediate(
        scratch_entry_size,
        "Integer",
        &byte_list_entry_stride().to_string(),
    ));
    out.push(abi::multiply_registers(
        scratch_product,
        scratch_capacity,
        scratch_entry_size,
    ));
    out.push(abi::add_immediate(
        scratch_product,
        scratch_product,
        COLLECTION_HEADER_SIZE,
    ));
    out.push(abi::add_registers(dst, collection, scratch_product));
}

/// The lookup-entry stride for a list of `element_type`, in bytes.
///
/// `COLLECTION_ENTRY_SIZE` for a kind-0 block, and **zero** for a kind-2 one —
/// a fixed-width list has no `LookupEntry` array at all (plan-57-D).
///
/// Zero is the load-bearing choice. Every layout formula in the tree is written
/// as `HEADER + capacity * <stride>` (+ `dataCapacity` for the block size), so a
/// stride of zero collapses each of them to the kind-2 layout without the
/// formula changing shape:
///
/// | | kind 0 | kind 2 |
/// |---|---|---|
/// | data base | `block + 40 + cap*40` | `block + 40` |
/// | block size | `40 + cap*40 + dataCap` | `40 + dataCap` |
///
/// So the allocation size, the free size and the data base cannot disagree about
/// the representation — they all read the same stride. That mattered enough to
/// design around: `emit_flat_block_size` computing a size the allocator did not
/// allocate is bug-02, and it corrupts the arena free list rather than producing
/// a wrong value.
pub(crate) fn list_entry_stride(element_type: &ParameterType) -> usize {
    if list_element_is_fixed_width(element_type).is_some() {
        0
    } else {
        COLLECTION_ENTRY_SIZE
    }
}

/// The `kind` byte for a list of `element_type`.
pub(crate) fn list_block_kind(element_type: &ParameterType) -> usize {
    if list_element_is_fixed_width(element_type).is_some() {
        COLLECTION_KIND_LIST_FIXED
    } else {
        COLLECTION_KIND_LIST
    }
}

/// The payload size of `element_type` when it uses the entry-free
/// representation, or `None` when it keeps a lookup table.
///
/// Now identical to [`list_element_is_fixed_width`], which it delegates to. The
/// two names are kept apart because they answer different questions and only
/// coincide today: this one is about the **representation** — "may element `i` be
/// addressed at `i * payloadSize`, with no entry to indirect through" — while
/// `list_element_is_fixed_width` is about the **type**. A `Map OF Scalar TO T`
/// has a fixed-width key and still keeps its entries, so the caller that wants a
/// stride must ask this question, not that one. Selecting a stride from the
/// payload type rather than the block kind was one of the two mistakes that
/// produced plan-57-D's corruption bugs.
pub(crate) fn kind2_payload_size(element_type: &ParameterType) -> Option<usize> {
    list_element_is_fixed_width(element_type)
}

/// The lookup-entry stride for a `List OF Byte`, for the runtime helpers that
/// build or read one and know their element type statically. Zero once the
/// entry-free representation is live (plan-57-D).
pub(crate) fn byte_list_entry_stride() -> usize {
    list_entry_stride(&ParameterType::Byte)
}

/// The type a list literal should actually be laid out as, or `None` to keep the
/// declared one.
///
/// `expression_type` falls back to `List OF Unknown` whenever it cannot type the
/// first element — a call to a builtin like `toByte` is enough. That placeholder
/// must not reach a layout decision: `lower_collection_values` sizes the entry
/// array from the first slot's ACTUAL value type, while the layout, the header,
/// and the scope-drop free size key off the declared type. Left unrefined the two
/// disagree, and the block is allocated entry-free then freed as `capacity * 40`
/// bytes past its end — bug-02's failure mode, reached from `[toByte(1)]`.
///
/// Only list literals with at least one element can be refined; an empty `[]` has
/// nothing to learn from, and a map keeps its entries regardless.
fn refined_list_literal_type(
    declared: &ParameterType,
    first_element_type: Option<&ParameterType>,
) -> Option<ParameterType> {
    // plan-111-C Phase 4 (plan-106-E Correction 4's site): built structurally.
    // This was `format!("List OF {element}")` over two rendered spellings — the
    // last production type construction outside `ParameterType::name` and the
    // wire codec — and the caller then parsed the result straight back.
    let element = first_element_type?;
    match declared {
        ParameterType::ListOf(declared_element)
            if matches!(**declared_element, ParameterType::Unknown) =>
        {
            Some(ParameterType::list_of(element.clone()))
        }
        _ => None,
    }
}

/// Allocate a `List OF Byte` of `count_off` elements: size the block, write the
/// header, and fill the lookup table with the identity mapping
/// (`valueOffset = i`, `valueLength = 1`). The payload bytes are left
/// uninitialized for the caller to fill.
///
/// Lives here rather than in `audio/` (plan-58-B): `link_thunk`'s `OUT CBuffer`
/// staging needs the same block, and a LINK thunk reaching into the audio module
/// for it would be a dependency that means nothing. The body is unchanged by the
/// move, so every audio thunk stays byte-identical.
///
/// One copy, shared by both audio backends (plan-57-B). It existed verbatim in
/// `alsa.rs` and `macos.rs` — the two differed only in label names and
/// comments. A third near-variant that also copies from a source buffer lives at
/// `crypto_ec::emit_build_byte_list`.
///
/// Sharing it is what makes plan-57-D a small edit rather than a sweep: this is
/// one of the places that must stop writing a lookup table once a fixed-width
/// list no longer has one.
///
/// A free function rather than a `CodeBuilder` method because both callers are
/// standalone `CodeFunction` emitters with no builder in scope (plan-57-A §Open
/// Decisions).
pub(crate) fn emit_alloc_byte_list(
    symbol: &str,
    tag: &str,
    count_off: usize,
    list_off: usize,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let entry_loop = format!("{symbol}_{tag}_bl_entry");
    let entry_done = format!("{symbol}_{tag}_bl_entry_done");
    // Minted from the one per-compilation id source every caller's minter draws
    // on (bug-721), so none of these can alias a register the caller holds. The
    // callers share nothing with this body but frame slots, and the signature
    // stays as the five call sites know it. `vN` is the register once spelled `%vN`.
    let mut vregs = Vregs::new();
    let [v9, v10, v11, v12, v13, v14, v15] = std::array::from_fn(|_| vregs.next());
    instructions.extend([
        abi::load_u64(v10, abi::stack_pointer(), count_off),
        abi::move_immediate(v11, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(v12, v10, v11),
        abi::add_immediate(v12, v12, COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), v12, v10), // + count payload bytes
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register(v15, abi::mfb_return(1)),
        abi::store_u64(v15, abi::stack_pointer(), list_off),
        abi::move_immediate(v9, "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8(v9, v15, COLLECTION_OFFSET_KIND),
        abi::move_immediate(v9, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(v9, v15, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(v9, "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8(v9, v15, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(v9, "Byte", "1"),
        abi::store_u8(v9, v15, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64(v10, abi::stack_pointer(), count_off),
        abi::store_u64(v10, v15, COLLECTION_OFFSET_COUNT),
        abi::store_u64(v10, v15, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(v10, v15, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(v10, v15, COLLECTION_OFFSET_DATA_CAPACITY),
        // entry array: entry[i] = { USED, value_offset=i, value_length=1 }
    ]);
    // kind 2 has no entry array to fill (plan-57-D), so the ENTIRE loop is
    // skipped — not just its body.
    //
    // plan-57-D guarded only the body, which left `label; cmp i,count;
    // bge done; i++; b loop` behind: a no-op loop that still ran `count` times at
    // RUNTIME. Every audio capture allocation paid it, and it scales with the
    // buffer — a 3-minute stereo 48 kHz read burned ~34 million iterations doing
    // nothing. Correct output, silently linear waste, which is why nothing caught
    // it. The header stores above already set count/capacity/dataLength/
    // dataCapacity, so under kind 2 there is nothing left for the loop to do.
    if byte_list_entry_stride() != 0 {
        instructions.extend([
            abi::add_immediate(v11, v15, COLLECTION_HEADER_SIZE), // entry cursor
            abi::move_immediate(v13, "Integer", "0"),             // i
            abi::label(&entry_loop),
            abi::compare_registers(v13, v10),
            abi::branch_ge(&entry_done),
            abi::move_immediate(v14, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8(v14, v11, COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64(abi::ZERO, v11, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64(abi::ZERO, v11, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64(v13, v11, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate(v14, "Integer", "1"),
            abi::store_u64(v14, v11, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
            abi::add_immediate(v11, v11, byte_list_entry_stride()),
            abi::add_immediate(v13, v13, 1),
            abi::branch(&entry_loop),
            abi::label(&entry_done),
        ]);
    }
}

/// The `kind` byte for a `List OF Byte`, for those same runtime helpers.
///
/// Nothing reads this byte at runtime — every consumer dispatches statically on
/// the source type — so stamping the wrong kind is invisible today. It is
/// written truthfully anyway: a block whose header claims a lookup table it does
/// not have is precisely the sort of latent disagreement that produced four
/// separate corruption bugs in this sub-plan, and the next person to reach for
/// the kind byte should be able to trust it.
pub(crate) fn byte_list_block_kind() -> usize {
    list_block_kind(&ParameterType::Byte)
}

#[cfg(test)]
mod tests;
