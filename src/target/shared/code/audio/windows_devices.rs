// devices(): enumerate active endpoints into a `List OF AudioDevice`.
//
// Device frame slots (this function has no `AudioHandle`, so the shared slots are
// reused as plain scratch): the enumerator at `STATE_OFF`, the device collection
// at `HANDLE_OFF`, the current `IMMDevice` at `SR_OFF`, its `IPropertyStore` at
// `CH_OFF`, the raw `GetId` LPWSTR out-slot at `BF_OFF`, and the 24-byte
// `PROPVARIANT` in the `WIDEID_OFF` region.
const D_ENUM: usize = STATE_OFF;
const D_COLL: usize = HANDLE_OFF;
const D_DEV: usize = SR_OFF;
const D_PROPS: usize = CH_OFF;
const D_IDRAW: usize = BF_OFF;
const D_PROPVAR: usize = WIDEID_OFF;

/// Build an MFBASIC `String` at `out_off` from the NUL-terminated UTF-16 string
/// whose pointer is in `%v9` (low byte of each unit — endpoint ids and friendly
/// names on the test box are ASCII). A null pointer yields an empty String.
fn emit_string_from_wstr(
    symbol: &str,
    tag: &str,
    out_off: usize,
    alloc_fail: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    let len_loop = format!("{symbol}_{tag}_wlen");
    let len_done = format!("{symbol}_{tag}_wlen_done");
    let copy_loop = format!("{symbol}_{tag}_wcopy");
    let copy_done = format!("{symbol}_{tag}_wcopy_done");
    ins.extend([
        abi::store_u64("%v9", abi::stack_pointer(), CSTR_OFF), // save wide ptr
        abi::move_immediate("%v10", "Integer", "0"),           // len (units)
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&len_done),
        abi::label(&len_loop),
        abi::load_u16("%v11", "%v9", 0),
        abi::compare_immediate("%v11", "0"),
        abi::branch_eq(&len_done),
        abi::add_immediate("%v9", "%v9", 2),
        abi::add_immediate("%v10", "%v10", 1),
        abi::branch(&len_loop),
        abi::label(&len_done),
        abi::store_u64("%v10", abi::stack_pointer(), TOTAL_OFF), // stash len (COUNT_OFF holds the device-loop bound)
        abi::add_immediate(abi::return_register(), "%v10", 9),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, ins, rel, alloc_fail);
    ins.extend([
        abi::move_register("%v15", abi::RET[1]),
        abi::load_u64("%v10", abi::stack_pointer(), TOTAL_OFF),
        abi::store_u64("%v10", "%v15", 0),
        abi::store_u64("%v15", abi::stack_pointer(), out_off),
        abi::load_u64("%v11", abi::stack_pointer(), CSTR_OFF), // wide ptr
        abi::add_immediate("%v12", "%v15", 8),
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("%v13", "%v10"),
        abi::branch_ge(&copy_done),
        abi::load_u8("%v14", "%v11", 0), // low byte of the wide unit
        abi::store_u8("%v14", "%v12", 0),
        abi::add_immediate("%v11", "%v11", 2),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, "%v12", 0),
    ]);
}

fn lower_devices(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let unavailable = format!("{symbol}_unavailable");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let fill_loop = format!("{symbol}_fill");
    let fill_done = format!("{symbol}_fill_done");
    let name_from_id = format!("{symbol}_name_id");
    let have_name = format!("{symbol}_have_name");
    let done = format!("{symbol}_done");
    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    // CoInitializeEx(NULL, MTA)
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate(abi::ARG[1], "Integer", COINIT_MULTITHREADED),
    ]);
    ole_call(symbol, "CoInitializeEx", 2, platform_imports, platform, &mut ins, &mut rel)?;
    // Zero the object slots so a bail-out never Releases garbage.
    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), D_ENUM),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), D_COLL),
    ]);
    // CoCreateInstance(CLSID_MMDeviceEnumerator, NULL, CLSCTX_ALL,
    //                  IID_IMMDeviceEnumerator, &enum)
    guid_addr(symbol, abi::return_register(), "CLSID_MMDeviceEnumerator", &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", CLSCTX_ALL));
    guid_addr(symbol, abi::ARG[3], "IID_IMMDeviceEnumerator", &mut ins, &mut rel);
    ins.push(abi::add_immediate(abi::ARG[4], abi::stack_pointer(), D_ENUM));
    ole_call(symbol, "CoCreateInstance", 5, platform_imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&unavailable),
    ]);
    // enum->EnumAudioEndpoints(eAll, DEVICE_STATE_ACTIVE, &coll)
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), D_ENUM),
        abi::store_u64("%v9", abi::stack_pointer(), OBJ_OFF),
        abi::move_immediate(abi::ARG[1], "Integer", E_ALL),
        abi::move_immediate(abi::ARG[2], "Integer", DEVICE_STATE_ACTIVE),
        abi::add_immediate(abi::ARG[3], abi::stack_pointer(), D_COLL),
    ]);
    com_call(SLOT_ENUM_ENDPOINTS, 4, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&unavailable),
        // coll->GetCount(&count)
        abi::store_u64(abi::ZERO, abi::stack_pointer(), COUNT_OFF),
        abi::load_u64("%v9", abi::stack_pointer(), D_COLL),
        abi::store_u64("%v9", abi::stack_pointer(), OBJ_OFF),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), COUNT_OFF),
    ]);
    com_call(SLOT_COLL_GET_COUNT, 2, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&unavailable),
        abi::load_u32("%v9", abi::stack_pointer(), COUNT_OFF),
        abi::store_u64("%v9", abi::stack_pointer(), COUNT_OFF),
    ]);
    // Allocate List OF AudioDevice (48-byte records inline; mirrors alsa).
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), COUNT_OFF),
        abi::move_immediate("%v11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v12", "%v10", "%v11"),
        abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
        abi::move_immediate("%v13", "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers("%v14", "%v10", "%v13"),
        abi::add_registers(abi::return_register(), "%v12", "%v14"),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::move_register("%v15", abi::RET[1]),
        abi::store_u64("%v15", abi::stack_pointer(), LIST_OFF),
        abi::move_immediate("%v9", "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KIND),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_OBJECT.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("%v9", "Byte", "1"),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64("%v10", abi::stack_pointer(), COUNT_OFF),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_COUNT),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("%v13", "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers("%v14", "%v10", "%v13"),
        abi::store_u64("%v14", "%v15", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("%v14", "%v15", COLLECTION_OFFSET_DATA_CAPACITY),
        // data region base = list + HEADER + count*ENTRY
        abi::add_immediate("%v11", "%v15", COLLECTION_HEADER_SIZE),
        abi::move_immediate("%v12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v13", "%v10", "%v12"),
        abi::add_registers("%v14", "%v11", "%v13"),
        abi::store_u64("%v14", abi::stack_pointer(), COLL_SRC_OFF),
        abi::store_u64("%v11", abi::stack_pointer(), COLL_ENTRY_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), OFFSET_OFF), // index
        abi::label(&fill_loop),
        abi::load_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), COUNT_OFF),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_ge(&fill_done),
    ]);
    // coll->Item(index, &dev)
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), D_COLL),
        abi::store_u64("%v9", abi::stack_pointer(), OBJ_OFF),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), OFFSET_OFF),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), D_DEV),
    ]);
    com_call(SLOT_COLL_ITEM, 3, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&unavailable),
        // dev->GetId(&pId)
        abi::store_u64(abi::ZERO, abi::stack_pointer(), D_IDRAW),
        abi::load_u64("%v9", abi::stack_pointer(), D_DEV),
        abi::store_u64("%v9", abi::stack_pointer(), OBJ_OFF),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), D_IDRAW),
    ]);
    com_call(SLOT_DEV_GET_ID, 2, &mut ins);
    // id = string(pId)
    ins.push(abi::load_u64("%v9", abi::stack_pointer(), D_IDRAW));
    emit_string_from_wstr(symbol, "id", DEVID_OFF, &alloc_fail, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), D_IDRAW));
    ole_call(symbol, "CoTaskMemFree", 1, platform_imports, platform, &mut ins, &mut rel)?;
    // name via property store (best effort).
    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), D_PROPS),
        abi::load_u64("%v9", abi::stack_pointer(), D_DEV),
        abi::store_u64("%v9", abi::stack_pointer(), OBJ_OFF),
        abi::move_immediate(abi::ARG[1], "Integer", STGM_READ),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), D_PROPS),
    ]);
    com_call(SLOT_DEV_OPEN_PROPSTORE, 3, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&name_from_id),
        // zero PROPVARIANT (24 bytes)
        abi::store_u64(abi::ZERO, abi::stack_pointer(), D_PROPVAR),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), D_PROPVAR + 8),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), D_PROPVAR + 16),
        // props->GetValue(&PKEY_FriendlyName, &propvar)
        abi::load_u64("%v9", abi::stack_pointer(), D_PROPS),
        abi::store_u64("%v9", abi::stack_pointer(), OBJ_OFF),
    ]);
    guid_addr(symbol, abi::ARG[1], "PKEY_Device_FriendlyName", &mut ins, &mut rel);
    ins.push(abi::add_immediate(abi::ARG[2], abi::stack_pointer(), D_PROPVAR));
    com_call(SLOT_PROPS_GET_VALUE, 3, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&name_from_id),
        // pwszVal at propvar+8
        abi::load_u64("%v9", abi::stack_pointer(), D_PROPVAR + 8),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&name_from_id),
        abi::store_u64("%v9", abi::stack_pointer(), GOTBYTES_OFF), // stash pwszVal
    ]);
    emit_string_from_wstr(symbol, "nm", NAME_OFF, &alloc_fail, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), GOTBYTES_OFF));
    ole_call(symbol, "CoTaskMemFree", 1, platform_imports, platform, &mut ins, &mut rel)?;
    // Release the property store.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), D_PROPS),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&have_name),
        abi::store_u64("%v9", abi::stack_pointer(), OBJ_OFF),
    ]);
    com_call(SLOT_RELEASE, 1, &mut ins);
    ins.extend([abi::branch(&have_name), abi::label(&name_from_id)]);
    // Release props if it was opened, then name = id.
    let props_done = format!("{symbol}_props_done");
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), D_PROPS),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&props_done),
        abi::store_u64("%v9", abi::stack_pointer(), OBJ_OFF),
    ]);
    com_call(SLOT_RELEASE, 1, &mut ins);
    ins.extend([
        abi::label(&props_done),
        abi::load_u64("%v9", abi::stack_pointer(), DEVID_OFF),
        abi::store_u64("%v9", abi::stack_pointer(), NAME_OFF),
        abi::label(&have_name),
    ]);
    // Build the AudioDevice record (id, name, canInput=1, canOutput=1, defaults=0).
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::move_immediate("%v10", "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers("%v11", "%v9", "%v10"),
        abi::load_u64("%v12", abi::stack_pointer(), COLL_SRC_OFF),
        abi::add_registers("%v12", "%v12", "%v11"), // record ptr
        abi::load_u64("%v13", abi::stack_pointer(), DEVID_OFF),
        abi::store_u64("%v13", "%v12", DEVICE_FIELD_ID),
        abi::load_u64("%v13", abi::stack_pointer(), NAME_OFF),
        abi::store_u64("%v13", "%v12", DEVICE_FIELD_NAME),
        abi::move_immediate("%v13", "Integer", "1"),
        abi::store_u64("%v13", "%v12", DEVICE_FIELD_CAN_INPUT),
        abi::store_u64("%v13", "%v12", DEVICE_FIELD_CAN_OUTPUT),
        abi::store_u64(abi::ZERO, "%v12", DEVICE_FIELD_IS_DEFAULT_INPUT),
        abi::store_u64(abi::ZERO, "%v12", DEVICE_FIELD_IS_DEFAULT_OUTPUT),
        // entry descriptor
        abi::load_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::move_immediate("%v10", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v11", "%v9", "%v10"),
        abi::load_u64("%v12", abi::stack_pointer(), COLL_ENTRY_OFF),
        abi::add_registers("%v12", "%v12", "%v11"),
        abi::move_immediate("%v13", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("%v13", "%v12", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(abi::ZERO, "%v12", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64(abi::ZERO, "%v12", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::move_immediate("%v10", "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers("%v11", "%v9", "%v10"),
        abi::store_u64("%v11", "%v12", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate("%v13", "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::store_u64("%v13", "%v12", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
    ]);
    // Release the device, advance.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), D_DEV),
        abi::store_u64("%v9", abi::stack_pointer(), OBJ_OFF),
    ]);
    com_call(SLOT_RELEASE, 1, &mut ins);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::add_immediate("%v9", "%v9", 1),
        abi::store_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
    ]);
    // Release the collection and enumerator.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), D_COLL),
        abi::store_u64("%v9", abi::stack_pointer(), OBJ_OFF),
    ]);
    com_call(SLOT_RELEASE, 1, &mut ins);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), D_ENUM),
        abi::store_u64("%v9", abi::stack_pointer(), OBJ_OFF),
    ]);
    com_call(SLOT_RELEASE, 1, &mut ins);
    ins.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), LIST_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&unavailable),
    ]);
    emit_fail(symbol, ERR_AUDIO_UNAVAILABLE_CODE, ERR_AUDIO_UNAVAILABLE_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&done));
    ins.push(abi::return_());
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME);
    Ok((frame, ins, rel, slots))
}
