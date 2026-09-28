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

use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::engine::types::{CodeFunction, CodeInstruction};
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;

/// One helper body's registers, minted together (bug-721). A helper body and the
/// fragments it splices in (`index_geometry`, `home_slot`, `load_bucket`, ...)
/// share registers by name — `home_slot` leaves the home slot in `v4` for the
/// body to read — so each helper mints one bank and hands it to every fragment.
/// Field `vN` is the register the body once spelled `%vN`.
macro_rules! register_bank {
    ($($name:ident),* $(,)?) => {
        struct Regs {
            $($name: VirtualRegister,)*
        }

        impl Regs {
            fn mint() -> Self {
                let mut vregs = Vregs::new();
                Regs {
                    $($name: vregs.next(),)*
                }
            }
        }
    };
}

register_bank!(
    v3, v4, v5, v6, v7, v8, v9, v10, v11, v12, v13, v14, v15, v16, v17, v19, v20, v21, v22, v23,
    v24, v25, v26, v27, v28, v29, v30, v31, v32, v33, v34, v35, v36, v37, v40, v41, v42, v43, v44,
    v45, v46, v47,
);

/// `dataBase` (`%v11`), `bucketBase` (`%v12`), `bucketCount` (`%v10`) and the
/// FNV prime (`%v8`) of the map in `%v20`.
fn index_geometry(r: &Regs) -> Vec<CodeInstruction> {
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    vec![
        abi::load_u64(r.v14, r.v20, COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate(r.v16, "Integer", &entry_size),
        abi::multiply_registers(r.v11, r.v14, r.v16),
        abi::add_registers(r.v11, r.v11, r.v20),
        abi::add_immediate(r.v11, r.v11, COLLECTION_HEADER_SIZE),
        abi::load_u64(r.v15, r.v20, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_registers(r.v12, r.v11, r.v15),
        abi::shift_left_immediate(r.v10, r.v14, 1),
        abi::move_immediate(r.v8, "Integer", FNV1A_PRIME),
    ]
}

/// The home slot (`%v4`) of entry `index`'s key: `FNV-1a(key) mod bucketCount`,
/// exactly as `_mfb_rt_map_build_buckets` computes it. Clobbers `%v3`, `%v5`,
/// `%v6`, `%v14`–`%v17`.
fn home_slot(r: &Regs, labels: &str, index: VirtualRegister) -> Vec<CodeInstruction> {
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let hloop = format!("{labels}_hloop");
    let hdone = format!("{labels}_hdone");
    vec![
        abi::move_immediate(r.v16, "Integer", &entry_size),
        abi::multiply_registers(r.v14, index, r.v16),
        abi::add_registers(r.v14, r.v14, r.v20),
        abi::add_immediate(r.v14, r.v14, COLLECTION_HEADER_SIZE),
        abi::load_u64(r.v15, r.v14, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::add_registers(r.v15, r.v11, r.v15),
        abi::load_u64(r.v17, r.v14, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::move_immediate(r.v16, "Integer", FNV1A_BASIS),
        abi::move_register(r.v5, r.v15),
        abi::move_register(r.v6, r.v17),
        abi::label(&hloop),
        abi::compare_immediate(r.v6, "0"),
        abi::branch_eq(&hdone),
        abi::load_u8(r.v3, r.v5, 0),
        abi::exclusive_or_registers(r.v16, r.v16, r.v3),
        abi::multiply_registers(r.v16, r.v16, r.v8),
        abi::add_immediate(r.v5, r.v5, 1),
        abi::subtract_immediate(r.v6, r.v6, 1),
        abi::branch(&hloop),
        abi::label(&hdone),
        abi::unsigned_divide_registers(r.v4, r.v16, r.v10),
        abi::multiply_subtract_registers(r.v4, r.v4, r.v10, r.v16),
    ]
}

/// `slot = slot + 1`, wrapping to 0 at `bucketCount` (`%v10`).
fn next_slot(r: &Regs, labels: &str, slot: VirtualRegister) -> Vec<CodeInstruction> {
    let nowrap = format!("{labels}_nowrap");
    vec![
        abi::add_immediate(slot, slot, 1),
        abi::compare_registers(slot, r.v10),
        abi::branch_lo(&nowrap),
        abi::move_immediate(slot, "Integer", "0"),
        abi::label(&nowrap),
    ]
}

/// The address (`%v7`) and content (`%v6`) of bucket `slot`.
fn load_bucket(r: &Regs, slot: VirtualRegister) -> Vec<CodeInstruction> {
    vec![
        abi::shift_left_immediate(r.v7, slot, 3),
        abi::add_registers(r.v7, r.v12, r.v7),
        abi::load_u64(r.v6, r.v7, 0),
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
    let r = &Regs::mint();
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
        abi::move_register(r.v20, abi::c_arg(0)),
        abi::move_register(r.v21, abi::c_arg(1)),
        abi::load_u8(r.v9, r.v20, COLLECTION_OFFSET_BUCKETS_READY),
        abi::compare_immediate(r.v9, "0"),
        abi::branch_eq(&done),
    ];
    body.extend(index_geometry(&r));
    body.extend(home_slot(r, &format!("{symbol}_own"), r.v21));
    body.push(abi::add_immediate(r.v22, r.v21, 1)); // the bucket value to find
    body.push(abi::label(&find));
    body.extend(load_bucket(r, r.v4));
    body.extend([
        abi::compare_immediate(r.v6, "0"),
        abi::branch_eq(&lost),
        abi::compare_registers(r.v6, r.v22),
        abi::branch_eq(&found),
    ]);
    body.extend(next_slot(r, &format!("{symbol}_find"), r.v4));
    body.push(abi::branch(&find));
    body.extend([
        abi::label(&lost),
        abi::move_immediate(r.v9, "Byte", "0"),
        abi::store_u8(r.v9, r.v20, COLLECTION_OFFSET_BUCKETS_READY),
        abi::branch(&done),
        // The hole is `%v23`; `%v24` walks the rest of the probe run.
        abi::label(&found),
        abi::move_immediate(r.v9, "Integer", "0"),
        abi::store_u64(r.v9, r.v7, 0),
        abi::move_register(r.v23, r.v4),
        abi::move_register(r.v24, r.v4),
        abi::label(&shift),
    ]);
    body.extend(next_slot(r, &format!("{symbol}_shift"), r.v24));
    body.extend(load_bucket(r, r.v24));
    body.extend([
        abi::compare_immediate(r.v6, "0"),
        abi::branch_eq(&done),
        abi::move_register(r.v25, r.v6),
        abi::subtract_immediate(r.v26, r.v6, 1),
    ]);
    body.extend(home_slot(r, &format!("{symbol}_run"), r.v26));
    // `%v4` is this slot's home `k`, the hole is `i` (`%v23`), the slot is `j`
    // (`%v24`). It stays when `k` lies cyclically in `(i, j]`.
    body.extend([
        abi::compare_registers(r.v23, r.v24),
        abi::branch_hi(&wrapped),
        // i < j: stays iff i < k && k <= j.
        abi::compare_registers(r.v4, r.v23),
        abi::branch_ls(&fill),
        abi::compare_registers(r.v4, r.v24),
        abi::branch_ls(&stay),
        abi::branch(&fill),
        // i > j (the run wrapped): stays iff k > i || k <= j.
        abi::label(&wrapped),
        abi::compare_registers(r.v4, r.v23),
        abi::branch_hi(&stay),
        abi::compare_registers(r.v4, r.v24),
        abi::branch_ls(&stay),
        // Move slot j back into the hole; j becomes the hole.
        abi::label(&fill),
        abi::shift_left_immediate(r.v7, r.v23, 3),
        abi::add_registers(r.v7, r.v12, r.v7),
        abi::store_u64(r.v25, r.v7, 0),
        abi::shift_left_immediate(r.v7, r.v24, 3),
        abi::add_registers(r.v7, r.v12, r.v7),
        abi::move_immediate(r.v9, "Integer", "0"),
        abi::store_u64(r.v9, r.v7, 0),
        abi::move_register(r.v23, r.v24),
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
    let r = &Regs::mint();
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
        abi::move_register(r.v20, abi::c_arg(0)),
        abi::move_register(r.v23, abi::c_arg(1)), // i: the entry read
    ];
    body.extend(index_geometry(&r));
    body.extend([
        abi::move_register(r.v24, r.v23), // w: where the next survivor goes
        abi::load_u64(r.v27, r.v20, COLLECTION_OFFSET_COUNT),
        abi::load_u8(r.v28, r.v20, COLLECTION_OFFSET_BUCKETS_READY),
        abi::label(&top),
        abi::compare_registers(r.v23, r.v27),
        abi::branch_ge(&end),
        abi::move_immediate(r.v16, "Integer", &entry_size),
        abi::multiply_registers(r.v13, r.v23, r.v16),
        abi::add_registers(r.v13, r.v13, r.v20),
        abi::add_immediate(r.v13, r.v13, COLLECTION_HEADER_SIZE),
        abi::load_u8(r.v9, r.v13, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::compare_immediate(r.v9, &COLLECTION_ENTRY_FLAG_REMOVED.to_string()),
        abi::branch_eq(&skip),
        abi::compare_registers(r.v23, r.v24),
        abi::branch_eq(&keep),
        // entry[w] = entry[i]: five words.
        abi::multiply_registers(r.v19, r.v24, r.v16),
        abi::add_registers(r.v19, r.v19, r.v20),
        abi::add_immediate(r.v19, r.v19, COLLECTION_HEADER_SIZE),
    ]);
    for word in 0..COLLECTION_ENTRY_SIZE / 8 {
        body.push(abi::load_u64(r.v9, r.v13, word * 8));
        body.push(abi::store_u64(r.v9, r.v19, word * 8));
    }
    body.extend([
        abi::compare_immediate(r.v28, "0"),
        abi::branch_eq(&keep),
        // Renumber: the bucket holding i + 1 now names w.
        abi::add_immediate(r.v22, r.v23, 1),
    ]);
    body.extend(home_slot(r, &format!("{symbol}_moved"), r.v24));
    body.push(abi::label(&find));
    body.extend(load_bucket(r, r.v4));
    body.extend([
        abi::compare_immediate(r.v6, "0"),
        abi::branch_eq(&lost),
        abi::compare_registers(r.v6, r.v22),
        abi::branch_eq(&found),
    ]);
    body.extend(next_slot(r, &format!("{symbol}_find"), r.v4));
    body.push(abi::branch(&find));
    body.extend([
        // The index disagrees with the entries: stop maintaining it and let the
        // next probe rebuild it.
        abi::label(&lost),
        abi::move_immediate(r.v28, "Byte", "0"),
        abi::store_u8(r.v28, r.v20, COLLECTION_OFFSET_BUCKETS_READY),
        abi::branch(&keep),
        abi::label(&found),
        abi::add_immediate(r.v9, r.v24, 1),
        abi::store_u64(r.v9, r.v7, 0),
        abi::label(&keep),
        abi::add_immediate(r.v24, r.v24, 1),
        abi::label(&skip),
        abi::add_immediate(r.v23, r.v23, 1),
        abi::branch(&top),
        abi::label(&end),
        abi::store_u64(r.v24, r.v20, COLLECTION_OFFSET_COUNT),
        abi::return_(),
    ]);
    helper(symbol, "runtime.mapCompactFrom", body)
}

/// bug-760 B1: drop every tombstone ([`COLLECTION_ENTRY_FLAG_REMOVED`] entry) of
/// the map in `%v20`, sliding the live entries down in their order (five words
/// each, no hashing), store the new `count`, and clear the whole
/// [`COLLECTION_OFFSET_MAP_STATE`] word: no tombstones, and the index not ready —
/// every live entry's index changed, so the next probe rebuilds it (one pass,
/// once per compaction). The payload bytes do not move; a tombstone's key and
/// value are dead slack for the reclaim. Clobbers `%v9`, `%v13`, `%v16`, `%v19`,
/// `%v23`, `%v24`, `%v27`.
///
/// `track` is `(address, index)`: when the address register is not 0, the live
/// entry at the index register moves, and its new index is stored at the address
/// — for a caller that holds an entry index across the compaction.
fn compact_tombstones(
    r: &Regs,
    labels: &str,
    track: Option<(VirtualRegister, VirtualRegister)>,
) -> Vec<CodeInstruction> {
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let top = format!("{labels}_loop");
    let keep = format!("{labels}_keep");
    let skip = format!("{labels}_skip");
    let end = format!("{labels}_end");
    let untracked = format!("{labels}_untracked");
    let mut body = vec![
        abi::load_u64(r.v27, r.v20, COLLECTION_OFFSET_COUNT),
        abi::move_immediate(r.v23, "Integer", "0"), // i: the entry read
        abi::move_immediate(r.v24, "Integer", "0"), // w: where the next live one goes
        abi::label(&top),
        abi::compare_registers(r.v23, r.v27),
        abi::branch_ge(&end),
        abi::move_immediate(r.v16, "Integer", &entry_size),
        abi::multiply_registers(r.v13, r.v23, r.v16),
        abi::add_registers(r.v13, r.v13, r.v20),
        abi::add_immediate(r.v13, r.v13, COLLECTION_HEADER_SIZE),
        abi::load_u8(r.v9, r.v13, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::compare_immediate(r.v9, &COLLECTION_ENTRY_FLAG_REMOVED.to_string()),
        abi::branch_eq(&skip),
        abi::compare_registers(r.v23, r.v24),
        abi::branch_eq(&keep),
        abi::multiply_registers(r.v19, r.v24, r.v16),
        abi::add_registers(r.v19, r.v19, r.v20),
        abi::add_immediate(r.v19, r.v19, COLLECTION_HEADER_SIZE),
    ];
    for word in 0..COLLECTION_ENTRY_SIZE / 8 {
        body.push(abi::load_u64(r.v9, r.v13, word * 8));
        body.push(abi::store_u64(r.v9, r.v19, word * 8));
    }
    body.push(abi::label(&keep));
    if let Some((address, index)) = track {
        body.extend([
            abi::compare_immediate(address, "0"),
            abi::branch_eq(&untracked),
            abi::compare_registers(r.v23, index),
            abi::branch_ne(&untracked),
            abi::store_u64(r.v24, address, 0),
            abi::label(&untracked),
        ]);
    }
    body.extend([
        abi::add_immediate(r.v24, r.v24, 1),
        abi::label(&skip),
        abi::add_immediate(r.v23, r.v23, 1),
        abi::branch(&top),
        abi::label(&end),
        abi::store_u64(r.v24, r.v20, COLLECTION_OFFSET_COUNT),
        abi::move_immediate(r.v9, "Integer", "0"),
        abi::store_u32(r.v9, r.v20, COLLECTION_OFFSET_MAP_STATE),
    ]);
    body
}

/// `_mfb_rt_map_settle(map) -> map` (bug-760 B1): make the `Map`/`Set` in `x0`
/// hold no tombstones, and return the (unmoved) block pointer. A no-op — three instructions — when its dead count is 0, which is
/// every map but one an in-place removal left tombstones in; otherwise one
/// [`compact_tombstones`] pass. The block does not move and nothing is
/// allocated, so a caller's pointer to it stays valid.
///
/// This is what keeps tombstones invisible: every read of a `Map`/`Set` binding
/// other than a tombstone-aware one (`len`, a hash lookup, an in-place update)
/// calls it first (`CodeBuilder::emit_map_settle`), so no other reader — copy,
/// iteration, equality, `toString`, a callee, a thread — ever sees a map whose
/// dead count is not 0. Its cost is the entries, paid only after removals left
/// tombstones, and then by a reader that walks the entries anyway.
pub(crate) fn lower_map_settle_helper() -> CodeFunction {
    let symbol = MAP_SETTLE_SYMBOL;
    let r = &Regs::mint();
    let done = format!("{symbol}_done");
    let mut body = vec![
        abi::label("entry"),
        abi::move_register(r.v20, abi::c_arg(0)),
        abi::load_u32(r.v9, r.v20, COLLECTION_OFFSET_MAP_STATE),
        abi::shift_right_immediate(r.v9, r.v9, MAP_DEAD_ENTRIES_SHIFT),
        abi::compare_immediate(r.v9, "0"),
        abi::branch_eq(&done),
    ];
    body.extend(compact_tombstones(r, &format!("{symbol}_compact"), None));
    body.extend([
        abi::label(&done),
        abi::move_register(abi::mfb_return(0), r.v20),
        abi::return_(),
    ]);
    helper(symbol, "runtime.mapSettle", body)
}

/// `_mfb_rt_map_retire(map, index)` (bug-760 B1): finish removing entry `index`
/// of the `Map`/`Set` in `x0`, whose bucket the caller has already deleted
/// (`_mfb_rt_map_bucket_delete`) and whose flag it has set to `REMOVED`.
///
/// * The last entry is truncated: `count` drops past it and past any tombstones
///   directly before it (each was counted once when it was made, so the walk is
///   paid for). Removing from the back therefore leaves no tombstone at all —
///   bug-732's back-removal cost.
/// * Any other entry stays as a tombstone: the dead count goes up by one and
///   nothing moves, so removing the oldest key of a FIFO costs O(1).
/// * Once the tombstones outnumber the live entries (`2 * dead > count`), or the
///   24-bit field is full, [`compact_tombstones`] drops them all. At least as many
///   removals as there are live entries separate two compactions, so each
///   removal pays O(1) amortized for them — and the dead entries (and their dead
///   bytes, which the reclaim then counts) never exceed the live ones.
pub(crate) fn lower_map_retire_helper() -> CodeFunction {
    let symbol = MAP_RETIRE_SYMBOL;
    let r = &Regs::mint();
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let l = |name: &str| format!("{symbol}_{name}");
    let mut body = vec![
        abi::label("entry"),
        abi::move_register(r.v20, abi::c_arg(0)),
        abi::move_register(r.v21, abi::c_arg(1)),
        abi::load_u64(r.v27, r.v20, COLLECTION_OFFSET_COUNT),
        abi::load_u32(r.v9, r.v20, COLLECTION_OFFSET_MAP_STATE),
        abi::shift_right_immediate(r.v25, r.v9, MAP_DEAD_ENTRIES_SHIFT), // dead
        abi::move_immediate(r.v6, "Integer", "255"),
        abi::and_registers(r.v26, r.v9, r.v6), // the ready byte
        abi::add_immediate(r.v22, r.v21, 1),
        abi::compare_registers(r.v22, r.v27),
        abi::branch_ne(&l("tomb")),
        // --- The last entry: truncate it and the tombstones before it. ---
        abi::move_register(r.v27, r.v21),
        abi::label(&l("trim")),
        abi::compare_immediate(r.v27, "0"),
        abi::branch_eq(&l("store")),
        abi::compare_immediate(r.v25, "0"),
        abi::branch_eq(&l("store")),
        abi::subtract_immediate(r.v22, r.v27, 1),
        abi::move_immediate(r.v16, "Integer", &entry_size),
        abi::multiply_registers(r.v13, r.v22, r.v16),
        abi::add_registers(r.v13, r.v13, r.v20),
        abi::add_immediate(r.v13, r.v13, COLLECTION_HEADER_SIZE),
        abi::load_u8(r.v9, r.v13, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::compare_immediate(r.v9, &COLLECTION_ENTRY_FLAG_REMOVED.to_string()),
        abi::branch_ne(&l("store")),
        abi::move_register(r.v27, r.v22),
        abi::subtract_immediate(r.v25, r.v25, 1),
        abi::branch(&l("trim")),
        abi::label(&l("store")),
        abi::store_u64(r.v27, r.v20, COLLECTION_OFFSET_COUNT),
        abi::branch(&l("state")),
        // --- Any other entry: one more tombstone, compacting when they win. ---
        abi::label(&l("tomb")),
        abi::add_immediate(r.v25, r.v25, 1),
        abi::add_registers(r.v6, r.v25, r.v25),
        abi::compare_registers(r.v6, r.v27),
        abi::branch_hi(&l("compact")),
        abi::move_immediate(r.v6, "Integer", &MAP_DEAD_ENTRIES_MAX.to_string()),
        abi::compare_registers(r.v25, r.v6),
        abi::branch_lo(&l("state")),
        abi::label(&l("compact")),
    ];
    body.extend(compact_tombstones(r, &l("compact"), None));
    body.extend([
        abi::branch(&l("done")),
        abi::label(&l("state")),
        abi::shift_left_immediate(r.v25, r.v25, MAP_DEAD_ENTRIES_SHIFT),
        abi::or_registers(r.v25, r.v25, r.v26),
        abi::store_u32(r.v25, r.v20, COLLECTION_OFFSET_MAP_STATE),
        abi::label(&l("done")),
        abi::return_(),
    ]);
    helper(symbol, "runtime.mapRetire", body)
}

/// `cursor = align(cursor, alignment)` for a runtime power-of-two `alignment`:
/// `(cursor + alignment - 1) & -alignment`. Clobbers `%v3`.
fn align_up(r: &Regs, cursor: VirtualRegister, alignment: VirtualRegister) -> Vec<CodeInstruction> {
    vec![
        abi::add_registers(cursor, cursor, alignment),
        abi::subtract_immediate(cursor, cursor, 1),
        abi::move_immediate(r.v3, "Integer", "0"),
        abi::subtract_registers(r.v3, r.v3, alignment),
        abi::and_registers(cursor, cursor, r.v3),
    ]
}

/// `dst = map + HEADER + index * ENTRY` (the map is in `%v20`). Clobbers `%v3`.
fn entry_address(r: &Regs, dst: VirtualRegister, index: VirtualRegister) -> Vec<CodeInstruction> {
    vec![
        abi::move_immediate(r.v3, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(dst, index, r.v3),
        abi::add_registers(dst, dst, r.v20),
        abi::add_immediate(dst, dst, COLLECTION_HEADER_SIZE),
    ]
}

/// The value offset (`dst`) of the entry whose index is in heap slot `slot` of the
/// bucket region (`%v12`). Clobbers `%v3`.
fn heap_key(r: &Regs, dst: VirtualRegister, slot: VirtualRegister) -> Vec<CodeInstruction> {
    let mut out = vec![
        abi::shift_left_immediate(dst, slot, 3),
        abi::add_registers(dst, dst, r.v12),
        abi::load_u64(dst, dst, 0),
    ];
    out.extend(entry_address(r, dst, dst));
    out.push(abi::load_u64(
        dst,
        dst,
        COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
    ));
    out
}

/// Swap heap slots `a` and `b` of the bucket region (`%v12`). Clobbers `%v3`–`%v6`.
fn heap_swap(r: &Regs, a: VirtualRegister, b: VirtualRegister) -> Vec<CodeInstruction> {
    vec![
        abi::shift_left_immediate(r.v4, a, 3),
        abi::add_registers(r.v4, r.v4, r.v12),
        abi::shift_left_immediate(r.v5, b, 3),
        abi::add_registers(r.v5, r.v5, r.v12),
        abi::load_u64(r.v6, r.v4, 0),
        abi::load_u64(r.v3, r.v5, 0),
        abi::store_u64(r.v3, r.v4, 0),
        abi::store_u64(r.v6, r.v5, 0),
    ]
}

/// Sift heap slot `%v40` down within the first `%v41` slots (a max-heap on the
/// entries' value offsets). Clobbers `%v3`–`%v6`, `%v42`–`%v45`.
fn sift_down(r: &Regs, labels: &str) -> Vec<CodeInstruction> {
    let top = format!("{labels}_top");
    let right_ok = format!("{labels}_right_ok");
    let done = format!("{labels}_done");
    let mut out = vec![
        abi::label(&top),
        // child = 2 * root + 1
        abi::shift_left_immediate(r.v42, r.v40, 1),
        abi::add_immediate(r.v42, r.v42, 1),
        abi::compare_registers(r.v42, r.v41),
        abi::branch_ge(&done),
        abi::add_immediate(r.v43, r.v42, 1),
        abi::compare_registers(r.v43, r.v41),
        abi::branch_ge(&right_ok),
    ];
    out.extend(heap_key(r, r.v44, r.v42));
    out.extend(heap_key(r, r.v45, r.v43));
    out.extend([
        abi::compare_registers(r.v45, r.v44),
        abi::branch_ls(&right_ok),
        abi::move_register(r.v42, r.v43),
        abi::label(&right_ok),
    ]);
    out.extend(heap_key(r, r.v44, r.v40));
    out.extend(heap_key(r, r.v45, r.v42));
    out.extend([abi::compare_registers(r.v44, r.v45), abi::branch_ge(&done)]);
    out.extend(heap_swap(r, r.v40, r.v42));
    out.extend([
        abi::move_register(r.v40, r.v42),
        abi::branch(&top),
        abi::label(&done),
    ]);
    out
}

/// Move one payload down to the aligned `cursor` (`%v29`) and repoint its entry
/// field: the entry is at `entry`, its offset/length fields at `off_field` /
/// `len_field`, its alignment in `alignment`. The destination never passes the
/// source (payloads are visited in increasing source offset), so a forward copy
/// is safe on the overlap — bug-760 B2: eight bytes at a time, then a byte tail
/// (each word is read before any byte at or past it is written, since
/// `destination <= source`), the same unaligned word loop
/// `emit_block_copy_advance` runs over every data region. A payload already at
/// its destination is not copied at all. A zero-length payload moves nothing and
/// takes the unaligned cursor. Clobbers `%v3`, `%v6`–`%v8`.
fn move_unit(
    r: &Regs,
    labels: &str,
    entry: VirtualRegister,
    off_field: usize,
    len_field: usize,
    alignment: VirtualRegister,
) -> Vec<CodeInstruction> {
    let empty = format!("{labels}_empty");
    let words = format!("{labels}_words");
    let copy = format!("{labels}_copy");
    let done = format!("{labels}_done");
    let mut out = vec![
        abi::load_u64(r.v7, entry, len_field),
        abi::compare_immediate(r.v7, "0"),
        abi::branch_eq(&empty),
    ];
    out.extend(align_up(r, r.v29, alignment));
    out.extend([
        abi::load_u64(r.v6, entry, off_field),
        abi::store_u64(r.v29, entry, off_field),
        abi::add_registers(r.v6, r.v28, r.v6),  // source
        abi::add_registers(r.v8, r.v28, r.v29), // destination
        abi::add_registers(r.v29, r.v29, r.v7),
        abi::compare_registers(r.v6, r.v8),
        abi::branch_eq(&done),
        abi::label(&words),
        abi::compare_immediate(r.v7, "8"),
        abi::branch_lo(&copy),
        abi::load_u64(r.v3, r.v6, 0),
        abi::store_u64(r.v3, r.v8, 0),
        abi::add_immediate(r.v6, r.v6, 8),
        abi::add_immediate(r.v8, r.v8, 8),
        abi::subtract_immediate(r.v7, r.v7, 8),
        abi::branch(&words),
        abi::label(&copy),
        abi::compare_immediate(r.v7, "0"),
        abi::branch_eq(&done),
        abi::load_u8(r.v3, r.v6, 0),
        abi::store_u8(r.v3, r.v8, 0),
        abi::add_immediate(r.v6, r.v6, 1),
        abi::add_immediate(r.v8, r.v8, 1),
        abi::subtract_immediate(r.v7, r.v7, 1),
        abi::branch(&copy),
        abi::label(&empty),
        abi::store_u64(r.v29, entry, off_field),
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
/// entry writer aligns them — and returns `0` (nothing done) unless
/// `dataLength > 2 * live`, i.e. the dead bytes outnumber the live ones. The
/// packed length is at most `2 * live` (each pad is shorter than the aligned
/// payload after it), so a repack is never followed by another: every `1` is
/// progress, and a caller's retry terminates. Otherwise it packs the live payloads to
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
///
/// bug-760 B1: it first drops the map's tombstones ([`compact_tombstones`]),
/// returning `1` for that alone — a grow must never copy a tombstone into a block
/// whose fresh header says there are none, and the entries they free may be all
/// the room the caller needed. That moves entries, so a caller holding an entry
/// index across the call (a `set` whose value outgrew its slot) passes the
/// index's address as a fourth argument (`0` for none) and gets it rewritten.
pub(crate) fn lower_map_reclaim_helper() -> CodeFunction {
    let symbol = MAP_RECLAIM_SYMBOL;
    let r = &Regs::mint();
    let l = |name: &str| format!("{symbol}_{name}");
    let mut b = vec![
        abi::label("entry"),
        abi::move_register(r.v20, abi::c_arg(0)), // map
        abi::move_register(r.v21, abi::c_arg(1)), // key alignment
        abi::move_register(r.v22, abi::c_arg(2)), // value alignment
        abi::move_register(r.v46, abi::c_arg(3)), // a tracked entry index's address, or 0
        // bug-760 B1: drop the tombstones first — their entries are room the
        // grow need not buy, and their payloads are dead bytes the walk below
        // must not count as live. Doing so is progress (`1`) on its own.
        abi::move_immediate(r.v37, "Integer", "0"),
        abi::load_u32(r.v9, r.v20, COLLECTION_OFFSET_MAP_STATE),
        abi::shift_right_immediate(r.v9, r.v9, MAP_DEAD_ENTRIES_SHIFT),
        abi::compare_immediate(r.v9, "0"),
        abi::branch_eq(&l("settled")),
        abi::move_immediate(r.v47, "Integer", "0"),
        abi::compare_immediate(r.v46, "0"),
        abi::branch_eq(&l("untracked")),
        abi::load_u64(r.v47, r.v46, 0),
        abi::label(&l("untracked")),
    ];
    b.extend(compact_tombstones(r, &l("settle"), Some((r.v46, r.v47))));
    b.extend([
        abi::move_immediate(r.v37, "Integer", "1"),
        abi::label(&l("settled")),
        abi::load_u64(r.v23, r.v20, COLLECTION_OFFSET_COUNT),
        abi::load_u64(r.v14, r.v20, COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate(r.v16, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(r.v28, r.v14, r.v16),
        abi::add_registers(r.v28, r.v28, r.v20),
        abi::add_immediate(r.v28, r.v28, COLLECTION_HEADER_SIZE), // data base
        abi::load_u64(r.v15, r.v20, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_registers(r.v12, r.v28, r.v15), // bucket base
        // --- Walk: live bytes (%v24), key order, out-of-sequence values (%v30). ---
        abi::move_immediate(r.v24, "Integer", "0"),
        abi::move_immediate(r.v30, "Integer", "0"),
        abi::move_immediate(r.v31, "Integer", "0"), // end of the previous key
        abi::move_immediate(r.v25, "Integer", "0"),
        abi::label(&l("walk")),
        abi::compare_registers(r.v25, r.v23),
        abi::branch_ge(&l("walked")),
    ]);
    b.extend(entry_address(r, r.v26, r.v25));
    b.extend(align_up(r, r.v24, r.v21));
    b.extend([
        abi::load_u64(r.v5, r.v26, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::add_registers(r.v24, r.v24, r.v5),
    ]);
    b.extend(align_up(r, r.v24, r.v22));
    b.extend([
        abi::load_u64(r.v5, r.v26, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_registers(r.v24, r.v24, r.v5),
        // Keys in entry order, or give up.
        abi::load_u64(r.v6, r.v26, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::compare_registers(r.v6, r.v31),
        abi::branch_lo(&l("none")),
        abi::load_u64(r.v5, r.v26, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::add_registers(r.v31, r.v6, r.v5),
        // In sequence: keyEnd <= valueOffset < next key's offset (no next: any).
        abi::move_immediate(r.v7, "Integer", "0"),
        abi::load_u64(r.v8, r.v26, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::compare_registers(r.v8, r.v31),
        abi::branch_lo(&l("out")),
        abi::add_immediate(r.v9, r.v25, 1),
        abi::compare_registers(r.v9, r.v23),
        abi::branch_ge(&l("mark")),
        abi::load_u64(
            r.v9,
            r.v26,
            COLLECTION_ENTRY_SIZE + COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ),
        abi::compare_registers(r.v8, r.v9),
        abi::branch_lo(&l("mark")),
        abi::label(&l("out")),
        abi::move_immediate(r.v7, "Integer", "1"),
        abi::add_immediate(r.v30, r.v30, 1),
        abi::label(&l("mark")),
        abi::store_u8(r.v7, r.v26, OUT_OF_SEQUENCE_MARK),
        abi::add_immediate(r.v25, r.v25, 1),
        abi::branch(&l("walk")),
        abi::label(&l("walked")),
        // Nothing to do unless dataLength > 2 * live (dead > live). Not
        // `dataLength - live`: `live` counts the alignment pads of the ENTRY-order
        // layout, and a packed map whose moved values sit last needs fewer, so
        // right after a repack dataLength can be below `live` — the subtraction
        // underflowed, looked huge, and the grow retried the repack forever.
        abi::load_u64(r.v5, r.v20, COLLECTION_OFFSET_DATA_LENGTH),
        abi::add_registers(r.v6, r.v24, r.v24),
        abi::compare_registers(r.v5, r.v6),
        abi::branch_ls(&l("none")),
        abi::compare_immediate(r.v30, "0"),
        abi::branch_eq(&l("sorted")),
        // --- Gather the out-of-sequence entries into the bucket region. ---
        abi::move_immediate(r.v25, "Integer", "0"),
        abi::move_immediate(r.v27, "Integer", "0"), // slots filled
        abi::label(&l("gather")),
        abi::compare_registers(r.v25, r.v23),
        abi::branch_ge(&l("gathered")),
    ]);
    b.extend(entry_address(r, r.v26, r.v25));
    b.extend([
        abi::load_u8(r.v7, r.v26, OUT_OF_SEQUENCE_MARK),
        abi::compare_immediate(r.v7, "0"),
        abi::branch_eq(&l("gather_next")),
        abi::shift_left_immediate(r.v8, r.v27, 3),
        abi::add_registers(r.v8, r.v8, r.v12),
        abi::store_u64(r.v25, r.v8, 0),
        abi::add_immediate(r.v27, r.v27, 1),
        abi::label(&l("gather_next")),
        abi::add_immediate(r.v25, r.v25, 1),
        abi::branch(&l("gather")),
        abi::label(&l("gathered")),
        // The bucket region now holds the sort, not the index.
        abi::move_immediate(r.v7, "Byte", "0"),
        abi::store_u8(r.v7, r.v20, COLLECTION_OFFSET_BUCKETS_READY),
        // --- Heap sort slots [0, r) by value offset. ---
        // Build: root = r / 2 down to 0.
        abi::shift_right_immediate(r.v46, r.v30, 1),
        abi::label(&l("build")),
        abi::compare_immediate(r.v46, "0"),
        abi::branch_eq(&l("built")),
        abi::subtract_immediate(r.v46, r.v46, 1),
        abi::move_register(r.v40, r.v46),
        abi::move_register(r.v41, r.v30),
    ]);
    b.extend(sift_down(r, &l("build_sift")));
    b.extend([
        abi::branch(&l("build")),
        abi::label(&l("built")),
        // Sort: end = r - 1 down to 1: swap slot 0 with slot end, sift in [0, end).
        abi::move_register(r.v46, r.v30),
        abi::label(&l("sort")),
        abi::subtract_immediate(r.v46, r.v46, 1),
        abi::compare_immediate(r.v46, "0"),
        abi::branch_le(&l("sorted")),
        abi::move_immediate(r.v47, "Integer", "0"),
    ]);
    b.extend(heap_swap(r, r.v47, r.v46));
    b.extend([
        abi::move_immediate(r.v40, "Integer", "0"),
        abi::move_register(r.v41, r.v46),
    ]);
    b.extend(sift_down(r, &l("sort_sift")));
    b.extend([
        abi::branch(&l("sort")),
        abi::label(&l("sorted")),
        // --- Merge: in-sequence units in entry order (%v25 entry, %v32 = 0 key /
        // 1 value) with the sorted out-of-sequence values (%v33 slot). ---
        abi::move_immediate(r.v29, "Integer", "0"), // cursor
        abi::move_immediate(r.v25, "Integer", "0"),
        abi::move_immediate(r.v32, "Integer", "0"),
        abi::move_immediate(r.v33, "Integer", "0"),
        abi::label(&l("merge")),
        // Next in-sequence unit: its entry in %v26, its offset in %v34, or none.
        abi::label(&l("next_seq")),
        abi::compare_registers(r.v25, r.v23),
        abi::branch_ge(&l("seq_done")),
    ]);
    b.extend(entry_address(r, r.v26, r.v25));
    b.extend([
        abi::compare_immediate(r.v32, "0"),
        abi::branch_ne(&l("seq_value")),
        abi::load_u64(r.v34, r.v26, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::branch(&l("have_seq")),
        abi::label(&l("seq_value")),
        abi::load_u8(r.v7, r.v26, OUT_OF_SEQUENCE_MARK),
        abi::compare_immediate(r.v7, "0"),
        abi::branch_eq(&l("seq_value_in")),
        // This value is merged from the sorted slots; go on to the next key.
        abi::move_immediate(r.v32, "Integer", "0"),
        abi::add_immediate(r.v25, r.v25, 1),
        abi::branch(&l("next_seq")),
        abi::label(&l("seq_value_in")),
        abi::load_u64(r.v34, r.v26, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::branch(&l("have_seq")),
        abi::label(&l("seq_done")),
        // No in-sequence unit left: take a sorted one, or finish.
        abi::compare_registers(r.v33, r.v30),
        abi::branch_ge(&l("merged")),
        abi::branch(&l("take_sorted")),
        abi::label(&l("have_seq")),
        abi::compare_registers(r.v33, r.v30),
        abi::branch_ge(&l("take_seq")),
    ]);
    b.extend(heap_key(r, r.v35, r.v33));
    b.extend([
        abi::compare_registers(r.v35, r.v34),
        abi::branch_lo(&l("take_sorted")),
        abi::label(&l("take_seq")),
        abi::compare_immediate(r.v32, "0"),
        abi::branch_ne(&l("take_seq_value")),
    ]);
    b.extend(move_unit(
        r,
        &l("key"),
        r.v26,
        COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        r.v21,
    ));
    b.extend([
        abi::move_immediate(r.v32, "Integer", "1"),
        abi::branch(&l("merge")),
        abi::label(&l("take_seq_value")),
    ]);
    b.extend(move_unit(
        r,
        &l("value"),
        r.v26,
        COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        r.v22,
    ));
    b.extend([
        abi::move_immediate(r.v32, "Integer", "0"),
        abi::add_immediate(r.v25, r.v25, 1),
        abi::branch(&l("merge")),
        abi::label(&l("take_sorted")),
        abi::shift_left_immediate(r.v36, r.v33, 3),
        abi::add_registers(r.v36, r.v36, r.v12),
        abi::load_u64(r.v36, r.v36, 0),
    ]);
    b.extend(entry_address(r, r.v36, r.v36));
    b.extend(move_unit(
        r,
        &l("sorted_value"),
        r.v36,
        COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        r.v22,
    ));
    b.extend([
        abi::add_immediate(r.v33, r.v33, 1),
        abi::branch(&l("merge")),
        abi::label(&l("merged")),
        abi::store_u64(r.v29, r.v20, COLLECTION_OFFSET_DATA_LENGTH),
        abi::move_immediate(r.v37, "Integer", "1"),
        abi::branch(&l("clear")),
        // Nothing packed: `%v37` says whether the tombstones were dropped.
        abi::label(&l("none")),
        // --- Clear the transient marks (the walk may have stopped early). ---
        abi::label(&l("clear")),
        abi::move_immediate(r.v25, "Integer", "0"),
        abi::move_immediate(r.v7, "Byte", "0"),
        abi::label(&l("clear_loop")),
        abi::compare_registers(r.v25, r.v23),
        abi::branch_ge(&l("done")),
    ]);
    b.extend(entry_address(r, r.v26, r.v25));
    b.extend([
        abi::store_u8(r.v7, r.v26, OUT_OF_SEQUENCE_MARK),
        abi::add_immediate(r.v25, r.v25, 1),
        abi::branch(&l("clear_loop")),
        abi::label(&l("done")),
        abi::move_register(abi::mfb_return(0), r.v37),
        abi::return_(),
    ]);
    helper(symbol, "runtime.mapReclaim", b)
}
