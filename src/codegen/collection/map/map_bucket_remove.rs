//! bug-732: remove entries from a `Map`/`Set` without rebuilding its hash index.
//!
//! A map's lookup entries are insertion-ordered, and its hash index (the
//! `Buckets` region, `2 * capacity` slots of `entryIndex + 1`, linear probing,
//! FNV-1a over the key bytes) addresses them by index. Removing an entry used to
//! shift the later entries down and mark the whole index not-ready, so the next
//! probe rebuilt it in O(count): a `set`/`removeKey` loop was O(n²). These two
//! runtime helpers keep the index current instead, at a cost proportional to the
//! entries removed and moved:
//!
//! * [`lower_map_bucket_delete_helper`] clears one entry's bucket and closes the
//!   gap by **backward-shift deletion** — the standard linear-probing delete that
//!   needs no tombstone: each later slot of the probe run moves back into the hole
//!   unless its key's home lies cyclically in `(hole, slot]`, where moving it
//!   would put it before its home.
//! * [`lower_map_compact_from_helper`] drops the entries a caller marked
//!   [`COLLECTION_ENTRY_FLAG_REMOVED`] from index `from` onward, sliding the
//!   survivors down in order, and renumbers each moved survivor's bucket.
//!
//! bug-733 adds a third helper, [`lower_map_reclaim_helper`], for the dead key and
//! value bytes those removals (and a value that outgrows its slot) leave in the
//! data region: every in-place map grow calls it first, and when the dead bytes
//! outnumber the live ones it packs the live payloads to the front of the data
//! region, in place and without allocating, so a map whose size stays constant
//! under churn stops growing.
//!
//! A caller removes entry `i` by: freeing what its value owns, deleting its bucket
//! (while every entry is still where the index says), marking it, and — once all
//! the removals are marked — compacting from the smallest marked index. Both
//! helpers are no-ops on the index when `bucketsReady` is 0; it is then rebuilt on
//! the next probe as before. Neither allocates or calls.

use crate::codegen::engine::types::{CodeFunction, CodeInstruction};
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;

/// `dataBase` (`%v11`), `bucketBase` (`%v12`), `bucketCount` (`%v10`) and the
/// FNV prime (`%v8`) of the map in `%v20`.
fn index_geometry() -> Vec<CodeInstruction> {
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    vec![
        abi::load_u64("%v14", "%v20", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("%v16", "Integer", &entry_size),
        abi::multiply_registers("%v11", "%v14", "%v16"),
        abi::add_registers("%v11", "%v11", "%v20"),
        abi::add_immediate("%v11", "%v11", COLLECTION_HEADER_SIZE),
        abi::load_u64("%v15", "%v20", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_registers("%v12", "%v11", "%v15"),
        abi::shift_left_immediate("%v10", "%v14", 1),
        abi::move_immediate("%v8", "Integer", FNV1A_PRIME),
    ]
}

/// The home slot (`%v4`) of entry `index`'s key: `FNV-1a(key) mod bucketCount`,
/// exactly as `_mfb_rt_map_build_buckets` computes it. Clobbers `%v3`, `%v5`,
/// `%v6`, `%v14`–`%v17`.
fn home_slot(labels: &str, index: &str) -> Vec<CodeInstruction> {
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let hloop = format!("{labels}_hloop");
    let hdone = format!("{labels}_hdone");
    vec![
        abi::move_immediate("%v16", "Integer", &entry_size),
        abi::multiply_registers("%v14", index, "%v16"),
        abi::add_registers("%v14", "%v14", "%v20"),
        abi::add_immediate("%v14", "%v14", COLLECTION_HEADER_SIZE),
        abi::load_u64("%v15", "%v14", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::add_registers("%v15", "%v11", "%v15"),
        abi::load_u64("%v17", "%v14", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::move_immediate("%v16", "Integer", FNV1A_BASIS),
        abi::move_register("%v5", "%v15"),
        abi::move_register("%v6", "%v17"),
        abi::label(&hloop),
        abi::compare_immediate("%v6", "0"),
        abi::branch_eq(&hdone),
        abi::load_u8("%v3", "%v5", 0),
        abi::exclusive_or_registers("%v16", "%v16", "%v3"),
        abi::multiply_registers("%v16", "%v16", "%v8"),
        abi::add_immediate("%v5", "%v5", 1),
        abi::subtract_immediate("%v6", "%v6", 1),
        abi::branch(&hloop),
        abi::label(&hdone),
        abi::unsigned_divide_registers("%v4", "%v16", "%v10"),
        abi::multiply_subtract_registers("%v4", "%v4", "%v10", "%v16"),
    ]
}

/// `slot = slot + 1`, wrapping to 0 at `bucketCount` (`%v10`).
fn next_slot(labels: &str, slot: &str) -> Vec<CodeInstruction> {
    let nowrap = format!("{labels}_nowrap");
    vec![
        abi::add_immediate(slot, slot, 1),
        abi::compare_registers(slot, "%v10"),
        abi::branch_lo(&nowrap),
        abi::move_immediate(slot, "Integer", "0"),
        abi::label(&nowrap),
    ]
}

/// The address (`%v7`) and content (`%v6`) of bucket `slot`.
fn load_bucket(slot: &str) -> Vec<CodeInstruction> {
    vec![
        abi::shift_left_immediate("%v7", slot, 3),
        abi::add_registers("%v7", "%v12", "%v7"),
        abi::load_u64("%v6", "%v7", 0),
    ]
}

fn helper(symbol: &str, name: &str, mut instructions: Vec<CodeInstruction>) -> CodeFunction {
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    CodeFunction {
        name: name.to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations: Vec::new(),
    }
}

/// `_mfb_rt_map_bucket_delete(map, index)`: remove entry `index`'s bucket from the
/// built hash index of the `Map`/`Set` in `x0`, keeping every other key findable.
/// Entry `index` must still be in place (its key is hashed to find the bucket).
///
/// Should the bucket not be found — the index disagreeing with the entries, which
/// no path produces — the index is marked not-ready, so the next probe rebuilds it
/// from the entries rather than trusting it.
pub(crate) fn lower_map_bucket_delete_helper() -> CodeFunction {
    let symbol = MAP_BUCKET_DELETE_SYMBOL;
    let done = format!("{symbol}_done");
    let find = format!("{symbol}_find");
    let found = format!("{symbol}_found");
    let lost = format!("{symbol}_lost");
    let shift = format!("{symbol}_shift");
    let wrapped = format!("{symbol}_wrapped");
    let stay = format!("{symbol}_stay");
    let fill = format!("{symbol}_fill");
    let mut body = vec![
        abi::label("entry"),
        // Park the arguments in vregs before any division (x86 homes them in the
        // registers `div` clobbers).
        abi::move_register("%v20", abi::c_arg(0)),
        abi::move_register("%v21", abi::c_arg(1)),
        abi::load_u8("%v9", "%v20", COLLECTION_OFFSET_BUCKETS_READY),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&done),
    ];
    body.extend(index_geometry());
    body.extend(home_slot(&format!("{symbol}_own"), "%v21"));
    body.push(abi::add_immediate("%v22", "%v21", 1)); // the bucket value to find
    body.push(abi::label(&find));
    body.extend(load_bucket("%v4"));
    body.extend([
        abi::compare_immediate("%v6", "0"),
        abi::branch_eq(&lost),
        abi::compare_registers("%v6", "%v22"),
        abi::branch_eq(&found),
    ]);
    body.extend(next_slot(&format!("{symbol}_find"), "%v4"));
    body.push(abi::branch(&find));
    body.extend([
        abi::label(&lost),
        abi::move_immediate("%v9", "Byte", "0"),
        abi::store_u8("%v9", "%v20", COLLECTION_OFFSET_BUCKETS_READY),
        abi::branch(&done),
        // The hole is `%v23`; `%v24` walks the rest of the probe run.
        abi::label(&found),
        abi::move_immediate("%v9", "Integer", "0"),
        abi::store_u64("%v9", "%v7", 0),
        abi::move_register("%v23", "%v4"),
        abi::move_register("%v24", "%v4"),
        abi::label(&shift),
    ]);
    body.extend(next_slot(&format!("{symbol}_shift"), "%v24"));
    body.extend(load_bucket("%v24"));
    body.extend([
        abi::compare_immediate("%v6", "0"),
        abi::branch_eq(&done),
        abi::move_register("%v25", "%v6"),
        abi::subtract_immediate("%v26", "%v6", 1),
    ]);
    body.extend(home_slot(&format!("{symbol}_run"), "%v26"));
    // `%v4` is this slot's home `k`, the hole is `i` (`%v23`), the slot is `j`
    // (`%v24`). It stays when `k` lies cyclically in `(i, j]`.
    body.extend([
        abi::compare_registers("%v23", "%v24"),
        abi::branch_hi(&wrapped),
        // i < j: stays iff i < k && k <= j.
        abi::compare_registers("%v4", "%v23"),
        abi::branch_ls(&fill),
        abi::compare_registers("%v4", "%v24"),
        abi::branch_ls(&stay),
        abi::branch(&fill),
        // i > j (the run wrapped): stays iff k > i || k <= j.
        abi::label(&wrapped),
        abi::compare_registers("%v4", "%v23"),
        abi::branch_hi(&stay),
        abi::compare_registers("%v4", "%v24"),
        abi::branch_ls(&stay),
        // Move slot j back into the hole; j becomes the hole.
        abi::label(&fill),
        abi::shift_left_immediate("%v7", "%v23", 3),
        abi::add_registers("%v7", "%v12", "%v7"),
        abi::store_u64("%v25", "%v7", 0),
        abi::shift_left_immediate("%v7", "%v24", 3),
        abi::add_registers("%v7", "%v12", "%v7"),
        abi::move_immediate("%v9", "Integer", "0"),
        abi::store_u64("%v9", "%v7", 0),
        abi::move_register("%v23", "%v24"),
        abi::label(&stay),
        abi::branch(&shift),
        abi::label(&done),
        abi::return_(),
    ]);
    helper(symbol, "runtime.mapBucketDelete", body)
}

/// `_mfb_rt_map_compact_from(map, from)`: drop every entry of the `Map`/`Set` in
/// `x0` at or after index `from` whose flags byte is
/// [`COLLECTION_ENTRY_FLAG_REMOVED`], sliding the survivors down in their order,
/// and store the new `count`. The entries before `from` are not touched. When the
/// hash index is built, each moved survivor's bucket is renumbered to its new
/// index; the removed entries' buckets must already be gone
/// ([`lower_map_bucket_delete_helper`]), which keeps every bucket value unique
/// while the renumbering runs. A removed entry's key and value bytes stay in the
/// data region as dead slack, exactly as they always have.
pub(crate) fn lower_map_compact_from_helper() -> CodeFunction {
    let symbol = MAP_COMPACT_FROM_SYMBOL;
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let top = format!("{symbol}_loop");
    let skip = format!("{symbol}_skip");
    let keep = format!("{symbol}_keep");
    let find = format!("{symbol}_find");
    let found = format!("{symbol}_found");
    let lost = format!("{symbol}_lost");
    let end = format!("{symbol}_end");
    let mut body = vec![
        abi::label("entry"),
        abi::move_register("%v20", abi::c_arg(0)),
        abi::move_register("%v23", abi::c_arg(1)), // i: the entry read
    ];
    body.extend(index_geometry());
    body.extend([
        abi::move_register("%v24", "%v23"), // w: where the next survivor goes
        abi::load_u64("%v27", "%v20", COLLECTION_OFFSET_COUNT),
        abi::load_u8("%v28", "%v20", COLLECTION_OFFSET_BUCKETS_READY),
        abi::label(&top),
        abi::compare_registers("%v23", "%v27"),
        abi::branch_ge(&end),
        abi::move_immediate("%v16", "Integer", &entry_size),
        abi::multiply_registers("%v13", "%v23", "%v16"),
        abi::add_registers("%v13", "%v13", "%v20"),
        abi::add_immediate("%v13", "%v13", COLLECTION_HEADER_SIZE),
        abi::load_u8("%v9", "%v13", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::compare_immediate("%v9", &COLLECTION_ENTRY_FLAG_REMOVED.to_string()),
        abi::branch_eq(&skip),
        abi::compare_registers("%v23", "%v24"),
        abi::branch_eq(&keep),
        // entry[w] = entry[i]: five words.
        abi::multiply_registers("%v19", "%v24", "%v16"),
        abi::add_registers("%v19", "%v19", "%v20"),
        abi::add_immediate("%v19", "%v19", COLLECTION_HEADER_SIZE),
    ]);
    for word in 0..COLLECTION_ENTRY_SIZE / 8 {
        body.push(abi::load_u64("%v9", "%v13", word * 8));
        body.push(abi::store_u64("%v9", "%v19", word * 8));
    }
    body.extend([
        abi::compare_immediate("%v28", "0"),
        abi::branch_eq(&keep),
        // Renumber: the bucket holding i + 1 now names w.
        abi::add_immediate("%v22", "%v23", 1),
    ]);
    body.extend(home_slot(&format!("{symbol}_moved"), "%v24"));
    body.push(abi::label(&find));
    body.extend(load_bucket("%v4"));
    body.extend([
        abi::compare_immediate("%v6", "0"),
        abi::branch_eq(&lost),
        abi::compare_registers("%v6", "%v22"),
        abi::branch_eq(&found),
    ]);
    body.extend(next_slot(&format!("{symbol}_find"), "%v4"));
    body.push(abi::branch(&find));
    body.extend([
        // The index disagrees with the entries: stop maintaining it and let the
        // next probe rebuild it.
        abi::label(&lost),
        abi::move_immediate("%v28", "Byte", "0"),
        abi::store_u8("%v28", "%v20", COLLECTION_OFFSET_BUCKETS_READY),
        abi::branch(&keep),
        abi::label(&found),
        abi::add_immediate("%v9", "%v24", 1),
        abi::store_u64("%v9", "%v7", 0),
        abi::label(&keep),
        abi::add_immediate("%v24", "%v24", 1),
        abi::label(&skip),
        abi::add_immediate("%v23", "%v23", 1),
        abi::branch(&top),
        abi::label(&end),
        abi::store_u64("%v24", "%v20", COLLECTION_OFFSET_COUNT),
        abi::return_(),
    ]);
    helper(symbol, "runtime.mapCompactFrom", body)
}

/// `cursor = align(cursor, alignment)` for a runtime power-of-two `alignment`:
/// `(cursor + alignment - 1) & -alignment`. Clobbers `%v3`.
fn align_up(cursor: &str, alignment: &str) -> Vec<CodeInstruction> {
    vec![
        abi::add_registers(cursor, cursor, alignment),
        abi::subtract_immediate(cursor, cursor, 1),
        abi::move_immediate("%v3", "Integer", "0"),
        abi::subtract_registers("%v3", "%v3", alignment),
        abi::and_registers(cursor, cursor, "%v3"),
    ]
}

/// `dst = map + HEADER + index * ENTRY` (the map is in `%v20`). Clobbers `%v3`.
fn entry_address(dst: &str, index: &str) -> Vec<CodeInstruction> {
    vec![
        abi::move_immediate("%v3", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(dst, index, "%v3"),
        abi::add_registers(dst, dst, "%v20"),
        abi::add_immediate(dst, dst, COLLECTION_HEADER_SIZE),
    ]
}

/// The value offset (`dst`) of the entry whose index is in heap slot `slot` of the
/// bucket region (`%v12`). Clobbers `%v3`.
fn heap_key(dst: &str, slot: &str) -> Vec<CodeInstruction> {
    let mut out = vec![
        abi::shift_left_immediate(dst, slot, 3),
        abi::add_registers(dst, dst, "%v12"),
        abi::load_u64(dst, dst, 0),
    ];
    out.extend(entry_address(dst, dst));
    out.push(abi::load_u64(
        dst,
        dst,
        COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
    ));
    out
}

/// Swap heap slots `a` and `b` of the bucket region (`%v12`). Clobbers `%v3`–`%v6`.
fn heap_swap(a: &str, b: &str) -> Vec<CodeInstruction> {
    vec![
        abi::shift_left_immediate("%v4", a, 3),
        abi::add_registers("%v4", "%v4", "%v12"),
        abi::shift_left_immediate("%v5", b, 3),
        abi::add_registers("%v5", "%v5", "%v12"),
        abi::load_u64("%v6", "%v4", 0),
        abi::load_u64("%v3", "%v5", 0),
        abi::store_u64("%v3", "%v4", 0),
        abi::store_u64("%v6", "%v5", 0),
    ]
}

/// Sift heap slot `%v40` down within the first `%v41` slots (a max-heap on the
/// entries' value offsets). Clobbers `%v3`–`%v6`, `%v42`–`%v45`.
fn sift_down(labels: &str) -> Vec<CodeInstruction> {
    let top = format!("{labels}_top");
    let right_ok = format!("{labels}_right_ok");
    let done = format!("{labels}_done");
    let mut out = vec![
        abi::label(&top),
        // child = 2 * root + 1
        abi::shift_left_immediate("%v42", "%v40", 1),
        abi::add_immediate("%v42", "%v42", 1),
        abi::compare_registers("%v42", "%v41"),
        abi::branch_ge(&done),
        abi::add_immediate("%v43", "%v42", 1),
        abi::compare_registers("%v43", "%v41"),
        abi::branch_ge(&right_ok),
    ];
    out.extend(heap_key("%v44", "%v42"));
    out.extend(heap_key("%v45", "%v43"));
    out.extend([
        abi::compare_registers("%v45", "%v44"),
        abi::branch_ls(&right_ok),
        abi::move_register("%v42", "%v43"),
        abi::label(&right_ok),
    ]);
    out.extend(heap_key("%v44", "%v40"));
    out.extend(heap_key("%v45", "%v42"));
    out.extend([
        abi::compare_registers("%v44", "%v45"),
        abi::branch_ge(&done),
    ]);
    out.extend(heap_swap("%v40", "%v42"));
    out.extend([
        abi::move_register("%v40", "%v42"),
        abi::branch(&top),
        abi::label(&done),
    ]);
    out
}

/// Move one payload down to the aligned `cursor` (`%v29`) and repoint its entry
/// field: the entry is at `entry`, its offset/length fields at `off_field` /
/// `len_field`, its alignment in `alignment`. The destination never passes the
/// source (payloads are visited in increasing source offset), so a forward byte
/// copy is safe on the overlap. A zero-length payload moves nothing and takes the
/// unaligned cursor. Clobbers `%v3`, `%v6`–`%v8`.
fn move_unit(
    labels: &str,
    entry: &str,
    off_field: usize,
    len_field: usize,
    alignment: &str,
) -> Vec<CodeInstruction> {
    let empty = format!("{labels}_empty");
    let copy = format!("{labels}_copy");
    let done = format!("{labels}_done");
    let mut out = vec![
        abi::load_u64("%v7", entry, len_field),
        abi::compare_immediate("%v7", "0"),
        abi::branch_eq(&empty),
    ];
    out.extend(align_up("%v29", alignment));
    out.extend([
        abi::load_u64("%v6", entry, off_field),
        abi::store_u64("%v29", entry, off_field),
        abi::add_registers("%v6", "%v28", "%v6"),  // source
        abi::add_registers("%v8", "%v28", "%v29"), // destination
        abi::add_registers("%v29", "%v29", "%v7"),
        abi::label(&copy),
        abi::compare_immediate("%v7", "0"),
        abi::branch_eq(&done),
        abi::load_u8("%v3", "%v6", 0),
        abi::store_u8("%v3", "%v8", 0),
        abi::add_immediate("%v6", "%v6", 1),
        abi::add_immediate("%v8", "%v8", 1),
        abi::subtract_immediate("%v7", "%v7", 1),
        abi::branch(&copy),
        abi::label(&empty),
        abi::store_u64("%v29", entry, off_field),
        abi::label(&done),
    ]);
    out
}

/// Entry reserved byte used as a transient "value out of sequence" mark while the
/// reclaim runs; cleared before it returns.
const OUT_OF_SEQUENCE_MARK: usize = COLLECTION_ENTRY_OFFSET_FLAGS + 1;

/// `_mfb_rt_map_reclaim(map, keyAlign, valueAlign) -> 0 | 1`: reclaim the dead key
/// and value bytes of the `Map`/`Set` in `x0` (bug-733), in place, allocating
/// nothing.
///
/// It totals the live bytes — every entry's key then value, each aligned as the
/// entry writer aligns them — and returns `0` (nothing done) unless the dead bytes
/// (`dataLength - live`) outnumber them. Otherwise it packs the live payloads to
/// the front of the data region and returns `1`; the block, its capacity, its data
/// base and its bucket region do not move.
///
/// Packing in place needs the payloads visited in increasing source offset, so a
/// move never overwrites a payload not yet moved. Keys are always in entry order
/// (a key is written once, at the data tail, and entries stay in insertion order);
/// a value is too unless an in-place `set` relocated it to the tail because it
/// outgrew its slot. So the walk marks each value that does not lie between its
/// own key and the next entry's key as out of sequence. Those are heap-sorted by
/// offset in the bucket region (at most `count <= capacity` of its `2 * capacity`
/// slots — the index is then marked not-ready and rebuilt by the next probe), and
/// merged with the in-sequence keys and values. Were the keys ever found out of
/// order, the reclaim does nothing and returns `0`: the grow then copies, as
/// before, rather than risk overwriting a payload.
///
/// Callers run it at a grow, whose entry copy already costs O(count), and retry
/// their room check after a `1`. A repack leaves no dead bytes, and at least as
/// many bytes must die again before the next one, so its O(count + live) cost —
/// plus O(r log r) for `r` relocated values — is paid for by the removals and
/// relocations that made the dead bytes. Every verbatim grow therefore starts
/// with at most as many dead bytes as live ones.
pub(crate) fn lower_map_reclaim_helper() -> CodeFunction {
    let symbol = MAP_RECLAIM_SYMBOL;
    let l = |name: &str| format!("{symbol}_{name}");
    let mut b = vec![
        abi::label("entry"),
        abi::move_register("%v20", abi::c_arg(0)), // map
        abi::move_register("%v21", abi::c_arg(1)), // key alignment
        abi::move_register("%v22", abi::c_arg(2)), // value alignment
        abi::load_u64("%v23", "%v20", COLLECTION_OFFSET_COUNT),
        abi::load_u64("%v14", "%v20", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("%v16", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v28", "%v14", "%v16"),
        abi::add_registers("%v28", "%v28", "%v20"),
        abi::add_immediate("%v28", "%v28", COLLECTION_HEADER_SIZE), // data base
        abi::load_u64("%v15", "%v20", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_registers("%v12", "%v28", "%v15"), // bucket base
        // --- Walk: live bytes (%v24), key order, out-of-sequence values (%v30). ---
        abi::move_immediate("%v24", "Integer", "0"),
        abi::move_immediate("%v30", "Integer", "0"),
        abi::move_immediate("%v31", "Integer", "0"), // end of the previous key
        abi::move_immediate("%v25", "Integer", "0"),
        abi::label(&l("walk")),
        abi::compare_registers("%v25", "%v23"),
        abi::branch_ge(&l("walked")),
    ];
    b.extend(entry_address("%v26", "%v25"));
    b.extend(align_up("%v24", "%v21"));
    b.extend([
        abi::load_u64("%v5", "%v26", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::add_registers("%v24", "%v24", "%v5"),
    ]);
    b.extend(align_up("%v24", "%v22"));
    b.extend([
        abi::load_u64("%v5", "%v26", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_registers("%v24", "%v24", "%v5"),
        // Keys in entry order, or give up.
        abi::load_u64("%v6", "%v26", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::compare_registers("%v6", "%v31"),
        abi::branch_lo(&l("none")),
        abi::load_u64("%v5", "%v26", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::add_registers("%v31", "%v6", "%v5"),
        // In sequence: keyEnd <= valueOffset < next key's offset (no next: any).
        abi::move_immediate("%v7", "Integer", "0"),
        abi::load_u64("%v8", "%v26", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::compare_registers("%v8", "%v31"),
        abi::branch_lo(&l("out")),
        abi::add_immediate("%v9", "%v25", 1),
        abi::compare_registers("%v9", "%v23"),
        abi::branch_ge(&l("mark")),
        abi::load_u64(
            "%v9",
            "%v26",
            COLLECTION_ENTRY_SIZE + COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ),
        abi::compare_registers("%v8", "%v9"),
        abi::branch_lo(&l("mark")),
        abi::label(&l("out")),
        abi::move_immediate("%v7", "Integer", "1"),
        abi::add_immediate("%v30", "%v30", 1),
        abi::label(&l("mark")),
        abi::store_u8("%v7", "%v26", OUT_OF_SEQUENCE_MARK),
        abi::add_immediate("%v25", "%v25", 1),
        abi::branch(&l("walk")),
        abi::label(&l("walked")),
        // dead = dataLength - live; nothing to do unless dead > live.
        abi::load_u64("%v5", "%v20", COLLECTION_OFFSET_DATA_LENGTH),
        abi::subtract_registers("%v5", "%v5", "%v24"),
        abi::compare_registers("%v5", "%v24"),
        abi::branch_ls(&l("none")),
        abi::compare_immediate("%v30", "0"),
        abi::branch_eq(&l("sorted")),
        // --- Gather the out-of-sequence entries into the bucket region. ---
        abi::move_immediate("%v25", "Integer", "0"),
        abi::move_immediate("%v27", "Integer", "0"), // slots filled
        abi::label(&l("gather")),
        abi::compare_registers("%v25", "%v23"),
        abi::branch_ge(&l("gathered")),
    ]);
    b.extend(entry_address("%v26", "%v25"));
    b.extend([
        abi::load_u8("%v7", "%v26", OUT_OF_SEQUENCE_MARK),
        abi::compare_immediate("%v7", "0"),
        abi::branch_eq(&l("gather_next")),
        abi::shift_left_immediate("%v8", "%v27", 3),
        abi::add_registers("%v8", "%v8", "%v12"),
        abi::store_u64("%v25", "%v8", 0),
        abi::add_immediate("%v27", "%v27", 1),
        abi::label(&l("gather_next")),
        abi::add_immediate("%v25", "%v25", 1),
        abi::branch(&l("gather")),
        abi::label(&l("gathered")),
        // The bucket region now holds the sort, not the index.
        abi::move_immediate("%v7", "Byte", "0"),
        abi::store_u8("%v7", "%v20", COLLECTION_OFFSET_BUCKETS_READY),
        // --- Heap sort slots [0, r) by value offset. ---
        // Build: root = r / 2 down to 0.
        abi::shift_right_immediate("%v46", "%v30", 1),
        abi::label(&l("build")),
        abi::compare_immediate("%v46", "0"),
        abi::branch_eq(&l("built")),
        abi::subtract_immediate("%v46", "%v46", 1),
        abi::move_register("%v40", "%v46"),
        abi::move_register("%v41", "%v30"),
    ]);
    b.extend(sift_down(&l("build_sift")));
    b.extend([
        abi::branch(&l("build")),
        abi::label(&l("built")),
        // Sort: end = r - 1 down to 1: swap slot 0 with slot end, sift in [0, end).
        abi::move_register("%v46", "%v30"),
        abi::label(&l("sort")),
        abi::subtract_immediate("%v46", "%v46", 1),
        abi::compare_immediate("%v46", "0"),
        abi::branch_le(&l("sorted")),
        abi::move_immediate("%v47", "Integer", "0"),
    ]);
    b.extend(heap_swap("%v47", "%v46"));
    b.extend([
        abi::move_immediate("%v40", "Integer", "0"),
        abi::move_register("%v41", "%v46"),
    ]);
    b.extend(sift_down(&l("sort_sift")));
    b.extend([
        abi::branch(&l("sort")),
        abi::label(&l("sorted")),
        // --- Merge: in-sequence units in entry order (%v25 entry, %v32 = 0 key /
        // 1 value) with the sorted out-of-sequence values (%v33 slot). ---
        abi::move_immediate("%v29", "Integer", "0"), // cursor
        abi::move_immediate("%v25", "Integer", "0"),
        abi::move_immediate("%v32", "Integer", "0"),
        abi::move_immediate("%v33", "Integer", "0"),
        abi::label(&l("merge")),
        // Next in-sequence unit: its entry in %v26, its offset in %v34, or none.
        abi::label(&l("next_seq")),
        abi::compare_registers("%v25", "%v23"),
        abi::branch_ge(&l("seq_done")),
    ]);
    b.extend(entry_address("%v26", "%v25"));
    b.extend([
        abi::compare_immediate("%v32", "0"),
        abi::branch_ne(&l("seq_value")),
        abi::load_u64("%v34", "%v26", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::branch(&l("have_seq")),
        abi::label(&l("seq_value")),
        abi::load_u8("%v7", "%v26", OUT_OF_SEQUENCE_MARK),
        abi::compare_immediate("%v7", "0"),
        abi::branch_eq(&l("seq_value_in")),
        // This value is merged from the sorted slots; go on to the next key.
        abi::move_immediate("%v32", "Integer", "0"),
        abi::add_immediate("%v25", "%v25", 1),
        abi::branch(&l("next_seq")),
        abi::label(&l("seq_value_in")),
        abi::load_u64("%v34", "%v26", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::branch(&l("have_seq")),
        abi::label(&l("seq_done")),
        // No in-sequence unit left: take a sorted one, or finish.
        abi::compare_registers("%v33", "%v30"),
        abi::branch_ge(&l("merged")),
        abi::branch(&l("take_sorted")),
        abi::label(&l("have_seq")),
        abi::compare_registers("%v33", "%v30"),
        abi::branch_ge(&l("take_seq")),
    ]);
    b.extend(heap_key("%v35", "%v33"));
    b.extend([
        abi::compare_registers("%v35", "%v34"),
        abi::branch_lo(&l("take_sorted")),
        abi::label(&l("take_seq")),
        abi::compare_immediate("%v32", "0"),
        abi::branch_ne(&l("take_seq_value")),
    ]);
    b.extend(move_unit(
        &l("key"),
        "%v26",
        COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        "%v21",
    ));
    b.extend([
        abi::move_immediate("%v32", "Integer", "1"),
        abi::branch(&l("merge")),
        abi::label(&l("take_seq_value")),
    ]);
    b.extend(move_unit(
        &l("value"),
        "%v26",
        COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        "%v22",
    ));
    b.extend([
        abi::move_immediate("%v32", "Integer", "0"),
        abi::add_immediate("%v25", "%v25", 1),
        abi::branch(&l("merge")),
        abi::label(&l("take_sorted")),
        abi::shift_left_immediate("%v36", "%v33", 3),
        abi::add_registers("%v36", "%v36", "%v12"),
        abi::load_u64("%v36", "%v36", 0),
    ]);
    b.extend(entry_address("%v36", "%v36"));
    b.extend(move_unit(
        &l("sorted_value"),
        "%v36",
        COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        "%v22",
    ));
    b.extend([
        abi::add_immediate("%v33", "%v33", 1),
        abi::branch(&l("merge")),
        abi::label(&l("merged")),
        abi::store_u64("%v29", "%v20", COLLECTION_OFFSET_DATA_LENGTH),
        abi::move_immediate("%v37", "Integer", "1"),
        abi::branch(&l("clear")),
        abi::label(&l("none")),
        abi::move_immediate("%v37", "Integer", "0"),
        // --- Clear the transient marks (the walk may have stopped early). ---
        abi::label(&l("clear")),
        abi::move_immediate("%v25", "Integer", "0"),
        abi::move_immediate("%v7", "Byte", "0"),
        abi::label(&l("clear_loop")),
        abi::compare_registers("%v25", "%v23"),
        abi::branch_ge(&l("done")),
    ]);
    b.extend(entry_address("%v26", "%v25"));
    b.extend([
        abi::store_u8("%v7", "%v26", OUT_OF_SEQUENCE_MARK),
        abi::add_immediate("%v25", "%v25", 1),
        abi::branch(&l("clear_loop")),
        abi::label(&l("done")),
        abi::move_register(abi::mfb_return(0), "%v37"),
        abi::return_(),
    ]);
    helper(symbol, "runtime.mapReclaim", b)
}
