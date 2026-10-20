//! plan-176-O: the `.mfp` writer's type payloads — function-type signatures, source-type and
//! field payloads, fixed decimals and resource flags. Project lowering and
//! `BinaryReprProject`'s encoder live in `binary_repr/writer/mod.rs`.

use super::*;

#[derive(Clone)]
pub(in crate::binary_repr) struct FunctionTypeSignature {
    pub(in crate::binary_repr) isolated: bool,
    pub(in crate::binary_repr) params: Vec<String>,
    pub(in crate::binary_repr) returns: String,
}

/// The wire signature of a function type.
///
/// plan-106-E: the decomposition is the [`crate::types::ParameterType::Func`]
/// variant's own fields — the `ISOLATED ` marker is its `isolated` flag, and the
/// parameter list is already split at the top level by the canonical grammar.
pub(in crate::binary_repr) fn parse_function_type(
    type_name: &str,
) -> Option<FunctionTypeSignature> {
    match crate::types::ParameterType::parse(type_name) {
        crate::types::ParameterType::Func(params, returns, isolated) => {
            Some(FunctionTypeSignature {
                isolated,
                params: params.iter().map(|p| p.name().into_owned()).collect(),
                returns: returns.name().into_owned(),
            })
        }
        _ => None,
    }
}

pub(in crate::binary_repr) fn source_type_payload(
    strings: &mut StringPool,
    types: &mut TypeTable,
    source_types: &HashMap<&str, &IrType>,
    ir_type: &IrType,
) -> Result<Vec<u8>, String> {
    let mut payload = Vec::new();
    match ir_type.kind.as_str() {
        "type" => {
            put_u32(&mut payload, ir_type.fields.len() as u32);
            for field in &ir_type.fields {
                put_field_payload(strings, types, &mut payload, field);
            }
        }
        "union" => {
            let variants = concrete_union_variants(source_types, ir_type)?;
            put_u32(&mut payload, variants.len() as u32);
            for variant in variants {
                put_u32(&mut payload, strings.intern(&variant.name));
                put_u32(&mut payload, variant.fields.len() as u32);
                for field in &variant.fields {
                    put_u32(&mut payload, strings.intern(&field.name));
                    put_u32(&mut payload, types.type_id(strings, &field.type_));
                }
            }
        }
        "enum" => {
            put_u32(&mut payload, ir_type.members.len() as u32);
            for (ordinal, member) in ir_type.members.iter().enumerate() {
                put_u32(&mut payload, strings.intern(&member.name));
                put_u32(&mut payload, ordinal as u32);
            }
        }
        _ => {}
    }
    Ok(payload)
}

pub(in crate::binary_repr) fn concrete_union_variants<'a>(
    source_types: &HashMap<&str, &'a IrType>,
    ir_type: &'a IrType,
) -> Result<Vec<&'a crate::ir::IrVariant>, String> {
    let mut variants = Vec::new();
    for include in &ir_type.includes {
        let included = source_types.get(include.as_str()).ok_or_else(|| {
            format!(
                "union `{}` includes unknown union `{include}`",
                ir_type.name
            )
        })?;
        variants.extend(concrete_union_variants(source_types, included)?);
    }
    variants.extend(ir_type.variants.iter());
    Ok(variants)
}

pub(in crate::binary_repr) fn put_field_payload(
    strings: &mut StringPool,
    types: &mut TypeTable,
    payload: &mut Vec<u8>,
    field: &crate::ir::IrField,
) {
    put_u32(payload, strings.intern(&field.name));
    put_u32(payload, types.type_id(strings, &field.type_));
    put_u32(
        payload,
        match field.visibility.as_deref() {
            Some("private") => 1,
            Some("package") => 2,
            Some("export") => 3,
            _ => 0,
        },
    );
}

/// The `.mfp` constant-pool encoding of a `Fixed` constant: exactly the carrier
/// source lowering materializes, from the one shared converter. This was a
/// second hand-copied converter without scientific-notation support, so a
/// package parameter default spelled `2.5e2F` failed to encode (bug-813).
pub(in crate::binary_repr) fn fixed_raw_from_decimal(value: &str) -> Result<i64, String> {
    crate::numeric::fixed_raw_from_decimal(value)
}

/// The `RESOURCE_TABLE` flags for a standard built-in resource, including the
/// "sendable to thread" bit (bit 2) when the registry marks the type sendable.
/// `RESOURCE_TABLE` flags for a built-in resource row.
///
/// **Takes the QUALIFIED id** (`fs.File`, `tcp.Socket`, `udp.Socket`) — every
/// registry lookup here does `resolve_type`, which splits on `'.'` and answers
/// `None` for a bare name. The three `add_standard_*` helpers used to pass the
/// BARE `"File"`/`"Socket"`/`"Listener"` constants, so `sendable` silently
/// answered `false` for all three and the bit was clear even for `fs::File`,
/// which has always been sendable. The unit test passed the qualified id, so it
/// never saw this (bug-464 fallout).
///
/// `close_may_fail` is registry-driven for the same reason the table is no
/// longer three hardcoded types: `process::Process` is the one built-in whose
/// close op cannot fail.
/// Takes the TYPE, not its spelling (plan-111 class 1b): every caller already
/// holds the `ParameterType` it hands `TypeTable::type_id`, so re-deriving one
/// from a string here would be a `declared` site for no gain.
pub(in crate::binary_repr) fn standard_resource_flags(type_: &crate::types::ParameterType) -> u32 {
    let mut flags = RESOURCE_FLAG_NATIVE | RESOURCE_FLAG_STANDARD;
    if crate::codegen::resource::is_builtin_sendable_resource_type(type_) {
        flags |= RESOURCE_FLAG_SENDABLE;
    }
    if crate::codegen::resource::builtin_resource_close_may_fail(type_) {
        flags |= RESOURCE_FLAG_CLOSE_MAY_FAIL;
    }
    flags
}
