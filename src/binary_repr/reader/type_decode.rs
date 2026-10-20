//! plan-176-O: the `.mfp` reader's type-table decoding — type exports and fields, the string
//! pool, type entries and type names. Container framing and the per-section table decoders live
//! in `binary_repr/reader/mod.rs`.

use super::*;

// === Type-table + type-name decoding =======================================

pub(in crate::binary_repr) fn decode_type_export(
    name: &str,
    kind: BinaryReprExportKind,
    entry: &TypeEntry,
    type_names: &HashMap<u32, String>,
    strings: &[String],
) -> Result<BinaryReprTypeExport, String> {
    let mut offset = 0usize;
    let (fields, variants, members) = match kind {
        BinaryReprExportKind::Type => {
            let field_count = cursor_u32(&entry.payload, &mut offset)? as usize;
            let mut fields = Vec::with_capacity(bounded_capacity(
                field_count,
                entry.payload.len() - offset,
                12,
            ));
            for _ in 0..field_count {
                fields.push(decode_type_field(
                    &entry.payload,
                    &mut offset,
                    type_names,
                    strings,
                )?);
            }
            (fields, Vec::new(), Vec::new())
        }
        BinaryReprExportKind::Union => {
            let variant_count = cursor_u32(&entry.payload, &mut offset)? as usize;
            let mut variants = Vec::with_capacity(bounded_capacity(
                variant_count,
                entry.payload.len() - offset,
                8,
            ));
            for _ in 0..variant_count {
                let variant_name =
                    string_at(strings, cursor_u32(&entry.payload, &mut offset)?)?.to_string();
                let field_count = cursor_u32(&entry.payload, &mut offset)? as usize;
                let mut fields = Vec::with_capacity(bounded_capacity(
                    field_count,
                    entry.payload.len() - offset,
                    8,
                ));
                for _ in 0..field_count {
                    let field_name =
                        string_at(strings, cursor_u32(&entry.payload, &mut offset)?)?.to_string();
                    let field_type =
                        type_name(type_names, cursor_u32(&entry.payload, &mut offset)?)?
                            .to_string();
                    fields.push(BinaryReprTypeField {
                        name: field_name,
                        type_: field_type,
                        visibility: BinaryReprTypeVisibility::Export,
                    });
                }
                variants.push(BinaryReprTypeVariant {
                    name: variant_name,
                    fields,
                });
            }
            (Vec::new(), variants, Vec::new())
        }
        BinaryReprExportKind::Enum => {
            let member_count = cursor_u32(&entry.payload, &mut offset)? as usize;
            let mut members = Vec::with_capacity(bounded_capacity(
                member_count,
                entry.payload.len() - offset,
                8,
            ));
            for _ in 0..member_count {
                members.push(
                    string_at(strings, cursor_u32(&entry.payload, &mut offset)?)?.to_string(),
                );
                let _ordinal = cursor_u32(&entry.payload, &mut offset)?;
            }
            (Vec::new(), Vec::new(), members)
        }
        BinaryReprExportKind::Func => {
            return Err(format!("export `{name}` is not a type export"));
        }
    };
    if offset != entry.payload.len() {
        return Err(format!("exported type `{name}` has trailing payload bytes"));
    }
    Ok(BinaryReprTypeExport {
        name: name.to_string(),
        kind,
        fields,
        variants,
        members,
        foreign_owner: None,
    })
}

pub(in crate::binary_repr) fn decode_type_field(
    payload: &[u8],
    offset: &mut usize,
    type_names: &HashMap<u32, String>,
    strings: &[String],
) -> Result<BinaryReprTypeField, String> {
    let name = string_at(strings, cursor_u32(payload, offset)?)?.to_string();
    let type_ = type_name(type_names, cursor_u32(payload, offset)?)?.to_string();
    let visibility = match cursor_u32(payload, offset)? {
        0 => BinaryReprTypeVisibility::Export,
        1 => BinaryReprTypeVisibility::Private,
        2 => BinaryReprTypeVisibility::Public,
        3 => BinaryReprTypeVisibility::Export,
        other => return Err(format!("unsupported type field visibility {other}")),
    };
    Ok(BinaryReprTypeField {
        name,
        type_,
        visibility,
    })
}

pub(in crate::binary_repr) fn read_string_pool(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut strings = Vec::with_capacity(bounded_capacity(count, bytes.len() - offset, 4));
    for _ in 0..count {
        // Each entry is a length-prefixed string, exactly what `cursor_string`
        // decodes (B5); `read_import_table` already reads this way.
        strings.push(cursor_string(bytes, &mut offset)?);
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in string pool".to_string());
    }
    Ok(strings)
}

pub(in crate::binary_repr) fn read_type_entries(
    bytes: &[u8],
    strings: &[String],
) -> Result<TypeTable, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let entries_end = 4usize
        .checked_add(
            count
                .checked_mul(20)
                .ok_or_else(|| "invalid type table length".to_string())?,
        )
        .ok_or_else(|| "invalid type table length".to_string())?;
    if entries_end > bytes.len() {
        return Err("truncated type table".to_string());
    }

    let mut entries = Vec::with_capacity(count);
    let mut ids = HashMap::new();
    // bug-282 B2: a `(name, kind)` pair identifies one type definition. Accepting
    // two lets `validate_abi_index` (any same-name candidate may reproduce the
    // hash) and `package_type_exports` (last-wins `HashMap`) resolve the same
    // export to *different* definitions -- validation passes against entry A while
    // importers compile against entry B. The writer interns a name once, so a
    // duplicate only arises from a crafted package.
    let mut seen_definitions = HashSet::new();
    for index in 0..count {
        let kind = cursor_u16(bytes, &mut offset)?;
        let _reserved = cursor_u16(bytes, &mut offset)?;
        let name = cursor_u32(bytes, &mut offset)?;
        let owner_package = cursor_u32(bytes, &mut offset)?;
        let payload_offset = cursor_u32(bytes, &mut offset)? as usize;
        let payload_length = cursor_u32(bytes, &mut offset)? as usize;
        let payload_end = payload_offset
            .checked_add(payload_length)
            .ok_or_else(|| "invalid type payload length".to_string())?;
        if payload_offset < entries_end || payload_end > bytes.len() {
            return Err("invalid type payload bounds".to_string());
        }
        let id = FIRST_TABLE_TYPE_ID + index as u32;
        if !seen_definitions.insert((name, kind)) {
            return Err(format!(
                "duplicate type table entry `{}` (kind {kind})",
                string_at(strings, name)?
            ));
        }
        ids.insert(string_at(strings, name)?.to_string(), id);
        entries.push(TypeEntry {
            kind,
            name,
            owner_package,
            abi_export_kind: None,
            payload: bytes[payload_offset..payload_end].to_vec(),
        });
    }

    Ok(TypeTable {
        entries,
        ids,
        // The read path never re-writes foreign references, so no owning-type
        // metadata is needed here (bug-390).
        foreign_types: HashMap::new(),
    })
}

pub(in crate::binary_repr) fn type_entry_names(
    types: &TypeTable,
    strings: &[String],
) -> Result<HashMap<u32, String>, String> {
    let raw = types
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            (
                FIRST_TABLE_TYPE_ID + index as u32,
                (entry.kind, entry.name, entry.payload.clone()),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut decoded = HashMap::new();
    let mut in_progress = HashSet::new();
    for id in raw.keys().copied().collect::<Vec<_>>() {
        let name = decode_type_name(id, &raw, strings, &mut decoded, &mut in_progress)?;
        decoded.insert(id, name);
    }
    Ok(decoded)
}

pub(in crate::binary_repr) fn decode_type_name(
    id: u32,
    raw: &HashMap<u32, (u16, u32, Vec<u8>)>,
    strings: &[String],
    decoded: &mut HashMap<u32, String>,
    in_progress: &mut HashSet<u32>,
) -> Result<String, String> {
    if let Some(name) = primitive_type_name(id) {
        return Ok(name.to_string());
    }
    if let Some(name) = decoded.get(&id) {
        return Ok(name.clone());
    }
    // Cycle guard (PKG-04): a composite type whose payload references its own id
    // (directly or via a mutual reference) would otherwise recurse forever until
    // the stack overflows. Mark the id in-progress *before* recursing — mirroring
    // `AbiSerializer::serialize_type`'s `type_refs` guard — and reject re-entry.
    if !in_progress.insert(id) {
        return Err(format!("cyclic type id {id}"));
    }
    // Depth cap (bug-153): `in_progress` holds exactly the ids on the active
    // recursion path (each is removed as its subtree unwinds), so its size is the
    // current depth. A deep-but-acyclic chain passes the cycle guard above but
    // must still be rejected before it overflows the native stack.
    if in_progress.len() > MAX_TYPE_GRAPH_DEPTH {
        in_progress.remove(&id);
        return Err(format!(
            "type graph too deep (exceeds {MAX_TYPE_GRAPH_DEPTH})"
        ));
    }
    let result = decode_type_name_body(id, raw, strings, decoded, in_progress);
    in_progress.remove(&id);
    let decoded_name = result?;
    decoded.insert(id, decoded_name.clone());
    Ok(decoded_name)
}

fn decode_type_name_body(
    id: u32,
    raw: &HashMap<u32, (u16, u32, Vec<u8>)>,
    strings: &[String],
    decoded: &mut HashMap<u32, String>,
    in_progress: &mut HashSet<u32>,
) -> Result<String, String> {
    let Some((kind, name, payload)) = raw.get(&id) else {
        return Err(format!("unknown type id {id}"));
    };
    let decoded_name = match *kind {
        4 => {
            let element = read_payload_type(payload, 0, raw, strings, decoded, in_progress)?;
            format!("List OF {element}")
        }
        5 => {
            let key = read_payload_type(payload, 0, raw, strings, decoded, in_progress)?;
            let value = read_payload_type(payload, 4, raw, strings, decoded, in_progress)?;
            format!("Map OF {key} TO {value}")
        }
        6 => {
            let success = read_payload_type(payload, 0, raw, strings, decoded, in_progress)?;
            format!("#Result OF {success}")
        }
        7 => {
            let message = read_payload_type(payload, 0, raw, strings, decoded, in_progress)?;
            let output = read_payload_type(payload, 4, raw, strings, decoded, in_progress)?;
            let resource = if payload.len() >= 12 {
                Some(read_payload_type(
                    payload,
                    8,
                    raw,
                    strings,
                    decoded,
                    in_progress,
                )?)
            } else {
                None
            };
            crate::types::format_thread_type("Thread", &message, resource.as_deref(), &output)
        }
        8 => decode_function_type(payload, raw, strings, decoded, in_progress)?,
        9 => {
            let key = read_payload_type(payload, 0, raw, strings, decoded, in_progress)?;
            let value = read_payload_type(payload, 4, raw, strings, decoded, in_progress)?;
            format!("MapEntry OF {key} TO {value}")
        }
        10 => {
            let message = read_payload_type(payload, 0, raw, strings, decoded, in_progress)?;
            let output = read_payload_type(payload, 4, raw, strings, decoded, in_progress)?;
            let resource = if payload.len() >= 12 {
                Some(read_payload_type(
                    payload,
                    8,
                    raw,
                    strings,
                    decoded,
                    in_progress,
                )?)
            } else {
                None
            };
            crate::types::format_thread_type("ThreadWorker", &message, resource.as_deref(), &output)
        }
        // A resource carrying a `STATE` payload (plan-52-D §4). Reconstructs the
        // `" STATE "` spelling every front-end stage pattern-matches on, so an
        // imported `FUNC(String) AS File STATE Cursor` reaches the consumer with
        // its STATE intact.
        11 => {
            let base = read_payload_type(payload, 0, raw, strings, decoded, in_progress)?;
            let state = read_payload_type(payload, 4, raw, strings, decoded, in_progress)?;
            format!("{base} STATE {state}")
        }
        // `Set OF T` (plan-63): a single element payload id, kind 13.
        13 => {
            let element = read_payload_type(payload, 0, raw, strings, decoded, in_progress)?;
            format!("Set OF {element}")
        }
        _ => string_at(strings, *name)?.to_string(),
    };
    Ok(decoded_name)
}

pub(in crate::binary_repr) fn decode_function_type(
    payload: &[u8],
    raw: &HashMap<u32, (u16, u32, Vec<u8>)>,
    strings: &[String],
    decoded: &mut HashMap<u32, String>,
    in_progress: &mut HashSet<u32>,
) -> Result<String, String> {
    let mut offset = 0;
    let isolated = cursor_u32(payload, &mut offset)? != 0;
    let param_count = cursor_u32(payload, &mut offset)? as usize;
    let return_type = cursor_u32(payload, &mut offset)?;
    let returns = decode_type_name(return_type, raw, strings, decoded, in_progress)?;
    let mut params = Vec::with_capacity(bounded_capacity(param_count, payload.len() - offset, 4));
    for _ in 0..param_count {
        let param = cursor_u32(payload, &mut offset)?;
        params.push(decode_type_name(param, raw, strings, decoded, in_progress)?);
    }
    let prefix = if isolated { "ISOLATED FUNC" } else { "FUNC" };
    Ok(format!("{prefix}({}) AS {returns}", params.join(", ")))
}

pub(in crate::binary_repr) fn read_payload_type(
    payload: &[u8],
    offset: usize,
    raw: &HashMap<u32, (u16, u32, Vec<u8>)>,
    strings: &[String],
    decoded: &mut HashMap<u32, String>,
    in_progress: &mut HashSet<u32>,
) -> Result<String, String> {
    let id = checked_u32_at(payload, offset)?;
    decode_type_name(id, raw, strings, decoded, in_progress)
}

pub(in crate::binary_repr) fn primitive_type_name(id: u32) -> Option<&'static str> {
    match id {
        TYPE_NOTHING => Some("Nothing"),
        TYPE_BOOLEAN => Some("Boolean"),
        TYPE_INTEGER => Some("Integer"),
        TYPE_FLOAT => Some("Float"),
        TYPE_FIXED => Some("Fixed"),
        TYPE_MONEY => Some("Money"),
        TYPE_SCALAR => Some("Scalar"),
        TYPE_ATTRIBUTED_STRING => Some("AttributedString"),
        TYPE_STRING => Some("String"),
        TYPE_BYTE => Some("Byte"),
        TYPE_ERROR => Some("Error"),
        // Retired by plan-122-F; kept so a package published before that change
        // decodes to a name instead of an opaque failure. Nothing encodes it.
        TYPE_TERM_COLOR => Some("TermColor"),
        TYPE_TERM_SIZE => Some("TermSize"),
        TYPE_FILE_HANDLE => Some("fs.File"),
        // plan-110-E: the wire ids are unchanged, but the identity they name
        // is `tcp`'s now -- net has no stream resources left to decode to.
        TYPE_SOCKET_HANDLE => Some("tcp.Socket"),
        TYPE_LISTENER_HANDLE => Some("tcp.Listener"),
        _ => None,
    }
}
