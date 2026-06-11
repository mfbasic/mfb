use crate::bytecode::{
    BuiltinCallLowerer, ValueSlot, OPCODE_GENERAL_FIND, OPCODE_GENERAL_IS_NUMERIC,
    OPCODE_GENERAL_LEN, OPCODE_GENERAL_MID, OPCODE_GENERAL_REPLACE, OPCODE_GENERAL_TO_BYTE,
    OPCODE_GENERAL_TO_FIXED, OPCODE_GENERAL_TO_FLOAT, OPCODE_GENERAL_TO_INT,
    OPCODE_GENERAL_TO_STRING, TYPE_BOOLEAN, TYPE_BYTE, TYPE_FIXED, TYPE_FLOAT, TYPE_INTEGER,
    TYPE_STRING,
};
use crate::ir::IrValue;
use std::collections::HashMap;

const LEN: &str = "len";
const FIND: &str = "find";
const MID: &str = "mid";
const REPLACE: &str = "replace";
const TYPE_NAME: &str = "typeName";
const TO_STRING: &str = "toString";
const TO_INT: &str = "toInt";
const TO_FLOAT: &str = "toFloat";
const TO_FIXED: &str = "toFixed";
const TO_BYTE: &str = "toByte";
const IS_NUMERIC: &str = "isNumeric";

#[derive(Clone, Copy)]
pub(crate) struct ResolvedCall {
    pub(crate) return_type: &'static str,
}

pub(crate) fn is_general_call(name: &str) -> bool {
    matches!(
        name,
        LEN | FIND
            | MID
            | REPLACE
            | TYPE_NAME
            | TO_STRING
            | TO_INT
            | TO_FLOAT
            | TO_FIXED
            | TO_BYTE
            | IS_NUMERIC
    )
}

pub(crate) fn resolve_call(name: &str, arg_types: &[String]) -> Option<ResolvedCall> {
    let resolved = match name {
        LEN => {
            if arg_types.len() != 1 {
                return None;
            }
            if arg_types[0] == "String"
                || arg_types[0].starts_with("List OF ")
                || arg_types[0].starts_with("Map OF ")
            {
                ResolvedCall {
                    return_type: "Integer",
                }
            } else {
                return None;
            }
        }
        FIND => {
            if !(2..=3).contains(&arg_types.len()) {
                return None;
            }
            if arg_types[0] == "String"
                && arg_types[1] == "String"
                && arg_types.get(2).is_none_or(|type_| type_ == "Integer")
            {
                ResolvedCall {
                    return_type: "Integer",
                }
            } else {
                return None;
            }
        }
        MID => {
            if exact(arg_types, &["String", "Integer", "Integer"]) {
                ResolvedCall {
                    return_type: "String",
                }
            } else {
                return None;
            }
        }
        REPLACE => {
            if exact(arg_types, &["String", "String", "String"]) {
                ResolvedCall {
                    return_type: "String",
                }
            } else {
                return None;
            }
        }
        TYPE_NAME => {
            if arg_types.len() == 1 {
                ResolvedCall {
                    return_type: "String",
                }
            } else {
                return None;
            }
        }
        TO_STRING => {
            if arg_types.len() != 1 {
                return None;
            }
            if matches!(
                arg_types[0].as_str(),
                "Integer" | "Float" | "Fixed" | "Boolean" | "String" | "Byte"
            ) || arg_types[0] == "List OF Byte"
            {
                ResolvedCall {
                    return_type: "String",
                }
            } else {
                return None;
            }
        }
        TO_INT => {
            if exact_one_of(arg_types, &["String", "Float", "Fixed"]) {
                ResolvedCall {
                    return_type: "Integer",
                }
            } else {
                return None;
            }
        }
        TO_FLOAT => {
            if exact_one_of(arg_types, &["String", "Integer", "Fixed"]) {
                ResolvedCall {
                    return_type: "Float",
                }
            } else {
                return None;
            }
        }
        TO_FIXED => {
            if exact_one_of(arg_types, &["String", "Integer", "Float"]) {
                ResolvedCall {
                    return_type: "Fixed",
                }
            } else {
                return None;
            }
        }
        TO_BYTE => {
            if exact(arg_types, &["Integer"]) {
                ResolvedCall {
                    return_type: "Byte",
                }
            } else {
                return None;
            }
        }
        IS_NUMERIC => {
            if exact(arg_types, &["String"]) {
                ResolvedCall {
                    return_type: "Boolean",
                }
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(resolved)
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        LEN => Some("String, List OF T, or Map OF K TO V"),
        FIND => Some("String, String, Integer"),
        MID => Some("String, Integer, Integer"),
        REPLACE => Some("String, String, String"),
        TYPE_NAME => Some("T"),
        TO_STRING => Some("Integer, Float, Fixed, Boolean, String, Byte, or List OF Byte"),
        TO_INT => Some("String, Float, or Fixed"),
        TO_FLOAT => Some("String, Integer, or Fixed"),
        TO_FIXED => Some("String, Integer, or Float"),
        TO_BYTE => Some("Integer"),
        IS_NUMERIC => Some("String"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        LEN | TYPE_NAME | TO_STRING | TO_INT | TO_FLOAT | TO_FIXED | TO_BYTE | IS_NUMERIC => {
            Some((1, 1))
        }
        FIND => Some((2, 3)),
        MID | REPLACE => Some((3, 3)),
        _ => None,
    }
}

pub(crate) fn lower_bytecode_call(
    lowerer: &mut dyn BuiltinCallLowerer,
    name: &str,
    args: &[IrValue],
    locals: &HashMap<String, ValueSlot>,
) -> Result<ValueSlot, String> {
    let lowered = args
        .iter()
        .map(|arg| lowerer.lower_value(arg, locals))
        .collect::<Result<Vec<_>, _>>()?;
    let arg_types = lowered
        .iter()
        .map(|slot| slot.type_name.clone())
        .collect::<Vec<_>>();
    let resolved = resolve_call(name, &arg_types).ok_or_else(|| {
        format!(
            "built-in `{name}` does not accept ({})",
            arg_types.join(", ")
        )
    })?;

    if name == TYPE_NAME {
        return lowerer.push_string_const(&arg_types[0]);
    }
    if name == TO_STRING && arg_types[0] == "String" {
        return Ok(lowered[0].clone());
    }

    let dst_type_id = type_id(resolved.return_type)?;
    let dst = lowerer.add_register(dst_type_id, 0);
    let opcode = opcode_for(name)?;
    let mut operands = vec![dst];
    operands.extend(lowered.iter().map(|slot| slot.register));
    lowerer.push(opcode, operands);
    Ok(ValueSlot {
        register: dst,
        type_name: resolved.return_type.to_string(),
    })
}

fn exact(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == expected.len()
        && arg_types
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| actual == expected)
}

fn exact_one_of(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == 1 && expected.iter().any(|expected| arg_types[0] == *expected)
}

fn type_id(type_name: &str) -> Result<u32, String> {
    match type_name {
        "Boolean" => Ok(TYPE_BOOLEAN),
        "Byte" => Ok(TYPE_BYTE),
        "Integer" => Ok(TYPE_INTEGER),
        "Float" => Ok(TYPE_FLOAT),
        "Fixed" => Ok(TYPE_FIXED),
        "String" => Ok(TYPE_STRING),
        _ => Err(format!(
            "unsupported General built-in return type `{type_name}`"
        )),
    }
}

fn opcode_for(name: &str) -> Result<u16, String> {
    match name {
        LEN => Ok(OPCODE_GENERAL_LEN),
        FIND => Ok(OPCODE_GENERAL_FIND),
        MID => Ok(OPCODE_GENERAL_MID),
        REPLACE => Ok(OPCODE_GENERAL_REPLACE),
        TO_STRING => Ok(OPCODE_GENERAL_TO_STRING),
        TO_INT => Ok(OPCODE_GENERAL_TO_INT),
        TO_FLOAT => Ok(OPCODE_GENERAL_TO_FLOAT),
        TO_FIXED => Ok(OPCODE_GENERAL_TO_FIXED),
        TO_BYTE => Ok(OPCODE_GENERAL_TO_BYTE),
        IS_NUMERIC => Ok(OPCODE_GENERAL_IS_NUMERIC),
        _ => Err(format!("unsupported General built-in `{name}`")),
    }
}
