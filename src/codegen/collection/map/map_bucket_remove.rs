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
