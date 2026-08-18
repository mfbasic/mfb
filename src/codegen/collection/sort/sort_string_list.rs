//! Split from `the retired flat codegen_utils.rs` (category `collection.sort`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
/// Symbol of the shared standalone string-list sort runtime helper (generic; not
/// path-specific — moved here from `fs/paths.rs`, bug-331 §J).
pub(crate) const SORT_STRING_LIST_SYMBOL: &str = "_mfb_rt_sort_string_list";

/// Lower the standalone string-list sort helper used to give `fs::listDirectory`
/// a deterministic, stable order. It takes a `List OF String` collection pointer
/// in `x0` and sorts its entries in place by ascending byte-wise (UTF-8
/// lexicographic) order using selection sort, swapping only the fixed-size entry
/// records and leaving the data region untouched. It makes no calls.
pub(crate) fn lower_sort_string_list_helper() -> CodeFunction {
    let symbol = SORT_STRING_LIST_SYMBOL;
    // x0  = collection pointer (preserved for the caller)
    // x9  = entries base (collection + header)
    // x10 = count
    // x11 = data region base (entries base + capacity * entry size)
    // x12 = i (outer index), x13 = min index, x14 = j (inner index)
    // x15 = entry[min] address, x16 = entry[j] address
    // x1..x7 = comparison/swap scratch
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let done = format!("{symbol}_done");
    let outer = format!("{symbol}_outer");
    let inner = format!("{symbol}_inner");
    let inner_done = format!("{symbol}_inner_done");
    let no_swap = format!("{symbol}_no_swap");
    let next_inner = format!("{symbol}_next_inner");
    let cmp_loop = format!("{symbol}_cmp_loop");
    let take_j = format!("{symbol}_take_j");
    let keep_min = format!("{symbol}_keep_min");

    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64("%v10", abi::c_arg(0), COLLECTION_OFFSET_COUNT),
        abi::compare_immediate("%v10", "1"),
        abi::branch_le(&done),
        abi::add_immediate("%v9", abi::c_arg(0), COLLECTION_HEADER_SIZE),
        abi::move_immediate("%v1", "Integer", &entry_size),
        // data region base = entries base + capacity * entry size (the data
        // region sits past the full lookup capacity for a grown list; §4.2).
        abi::load_u64("%v8", abi::c_arg(0), COLLECTION_OFFSET_CAPACITY),
        abi::multiply_registers("%v11", "%v8", "%v1"),
        abi::add_registers("%v11", "%v9", "%v11"),
        abi::move_immediate("%v12", "Integer", "0"),
        // outer: for i in 0..count-1
        abi::label(&outer),
        abi::add_immediate("%v2", "%v12", 1),
        abi::compare_registers("%v2", "%v10"),
        abi::branch_ge(&done),
        abi::move_register("%v13", "%v12"),
        abi::move_register("%v14", "%v2"),
        // inner: for j in i+1..count
        abi::label(&inner),
        abi::compare_registers("%v14", "%v10"),
        abi::branch_ge(&inner_done),
        // entry[min] -> x15, entry[j] -> x16
        abi::move_immediate("%v1", "Integer", &entry_size),
        abi::multiply_registers("%v15", "%v13", "%v1"),
        abi::add_registers("%v15", "%v9", "%v15"),
        abi::multiply_registers("%v16", "%v14", "%v1"),
        abi::add_registers("%v16", "%v9", "%v16"),
        // name pointers: data_base + value_offset ; lengths: value_length
        abi::load_u64("%v2", "%v15", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers("%v2", "%v11", "%v2"),
        abi::load_u64("%v3", "%v15", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::load_u64("%v4", "%v16", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers("%v4", "%v11", "%v4"),
        abi::load_u64("%v5", "%v16", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        // compare bytes: x2/x3 = min name ptr/len, x4/x5 = j name ptr/len
        abi::move_immediate("%v6", "Integer", "0"),
        abi::label(&cmp_loop),
        // if reached end of min name -> min is prefix; j<min iff j also ended? no: min shorter => min<j => keep_min
        abi::compare_registers("%v6", "%v3"),
        abi::branch_ge(&keep_min),
        // if reached end of j name -> j shorter, j<min => take_j
        abi::compare_registers("%v6", "%v5"),
        abi::branch_ge(&take_j),
        abi::load_u8("%v7", "%v2", 0),
        abi::load_u8("%v1", "%v4", 0),
        abi::compare_registers("%v1", "%v7"),
        abi::branch_lo(&take_j),
        abi::branch_hi(&keep_min),
        abi::add_immediate("%v2", "%v2", 1),
        abi::add_immediate("%v4", "%v4", 1),
        abi::add_immediate("%v6", "%v6", 1),
        abi::branch(&cmp_loop),
        abi::label(&take_j),
        abi::move_register("%v13", "%v14"),
        abi::label(&keep_min),
        abi::label(&next_inner),
        abi::add_immediate("%v14", "%v14", 1),
        abi::branch(&inner),
        abi::label(&inner_done),
        // swap entry[i] and entry[min] if different
        abi::compare_registers("%v13", "%v12"),
        abi::branch_eq(&no_swap),
        abi::move_immediate("%v1", "Integer", &entry_size),
        abi::multiply_registers("%v2", "%v12", "%v1"),
        abi::add_registers("%v2", "%v9", "%v2"),
        abi::multiply_registers("%v3", "%v13", "%v1"),
        abi::add_registers("%v3", "%v9", "%v3"),
    ];
    // swap COLLECTION_ENTRY_SIZE bytes (8 at a time)
    let mut offset = 0;
    while offset < COLLECTION_ENTRY_SIZE {
        instructions.extend([
            abi::load_u64("%v4", "%v2", offset),
            abi::load_u64("%v5", "%v3", offset),
            abi::store_u64("%v5", "%v2", offset),
            abi::store_u64("%v4", "%v3", offset),
        ]);
        offset += 8;
    }
    instructions.extend([
        abi::label(&no_swap),
        abi::add_immediate("%v12", "%v12", 1),
        abi::branch(&outer),
        abi::label(&done),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    CodeFunction {
        name: "runtime.sortStringList".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations: Vec::new(),
    }
}
