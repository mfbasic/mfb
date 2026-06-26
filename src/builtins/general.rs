use std::borrow::Cow;

const ERROR: &str = "error";
const LEN: &str = "len";
const TYPE_NAME: &str = "typeName";
const TO_STRING: &str = "toString";
const TO_INT: &str = "toInt";
const TO_FLOAT: &str = "toFloat";
const TO_FIXED: &str = "toFixed";
const TO_BYTE: &str = "toByte";
const IS_NUMERIC: &str = "isNumeric";
const IS_EVEN: &str = "isEven";
const IS_ODD: &str = "isOdd";
const IS_POSITIVE: &str = "isPositive";
const IS_NEGATIVE: &str = "isNegative";
const IS_ZERO: &str = "isZero";
const IS_EMPTY: &str = "isEmpty";
const IS_NOT_EMPTY: &str = "isNotEmpty";

pub(crate) const BUILTIN_FUNCTION_ID_BASE: u32 = 0x8000_0000;
pub(crate) const BUILTIN_FUNCTION_IS_EVEN: u32 = BUILTIN_FUNCTION_ID_BASE + 1;
pub(crate) const BUILTIN_FUNCTION_IS_ODD: u32 = BUILTIN_FUNCTION_ID_BASE + 2;
pub(crate) const BUILTIN_FUNCTION_IS_POSITIVE: u32 = BUILTIN_FUNCTION_ID_BASE + 3;
pub(crate) const BUILTIN_FUNCTION_IS_NEGATIVE: u32 = BUILTIN_FUNCTION_ID_BASE + 4;
pub(crate) const BUILTIN_FUNCTION_IS_ZERO: u32 = BUILTIN_FUNCTION_ID_BASE + 5;
pub(crate) const BUILTIN_FUNCTION_IS_EMPTY: u32 = BUILTIN_FUNCTION_ID_BASE + 6;
pub(crate) const BUILTIN_FUNCTION_IS_NOT_EMPTY: u32 = BUILTIN_FUNCTION_ID_BASE + 7;
pub(crate) const BUILTIN_FUNCTION_IS_POSITIVE_FLOAT: u32 = BUILTIN_FUNCTION_ID_BASE + 8;
pub(crate) const BUILTIN_FUNCTION_IS_POSITIVE_FIXED: u32 = BUILTIN_FUNCTION_ID_BASE + 9;
pub(crate) const BUILTIN_FUNCTION_IS_NEGATIVE_FLOAT: u32 = BUILTIN_FUNCTION_ID_BASE + 10;
pub(crate) const BUILTIN_FUNCTION_IS_NEGATIVE_FIXED: u32 = BUILTIN_FUNCTION_ID_BASE + 11;
pub(crate) const BUILTIN_FUNCTION_IS_ZERO_FLOAT: u32 = BUILTIN_FUNCTION_ID_BASE + 12;
pub(crate) const BUILTIN_FUNCTION_IS_ZERO_FIXED: u32 = BUILTIN_FUNCTION_ID_BASE + 13;

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_general_call(name: &str) -> bool {
    matches!(
        name,
        ERROR
            | LEN
            | TYPE_NAME
            | TO_STRING
            | TO_INT
            | TO_FLOAT
            | TO_FIXED
            | TO_BYTE
            | IS_NUMERIC
            | IS_EVEN
            | IS_ODD
            | IS_POSITIVE
            | IS_NEGATIVE
            | IS_ZERO
            | IS_EMPTY
            | IS_NOT_EMPTY
    )
}

/// Whether a general built-in may be **overridden** by a user- or package-defined
/// `FUNC` of the same name for its own value types (plan-01-overload.md §A.2). Every
/// general call is overridable except `error`, which builds the read-only `Error`
/// record and is a reserved language primitive.
pub(crate) fn is_overridable(name: &str) -> bool {
    is_general_call(name) && name != ERROR
}

/// Whether a general built-in name is **reserved** and may not be declared as a
/// user `FUNC`/`SUB` (plan-01-overload.md §A.5). The reserved set is exactly
/// `{ error }`.
pub(crate) fn reserved_builtin_name(name: &str) -> bool {
    name == ERROR
}

/// The built-in's conventional result type for an overridable general call
/// (plan-01-overload.md §C, Phase 4). A **package-provided** override (routed
/// through the override registry) yields this declared result; a user override
/// yields its own declared return type instead. Returns `None` for `error` and any
/// non-general name.
pub(crate) fn override_result_type(name: &str) -> Option<&'static str> {
    match name {
        TO_STRING | TYPE_NAME => Some("String"),
        LEN | TO_INT => Some("Integer"),
        TO_FLOAT => Some("Float"),
        TO_FIXED => Some("Fixed"),
        TO_BYTE => Some("Byte"),
        IS_NUMERIC | IS_EVEN | IS_ODD | IS_POSITIVE | IS_NEGATIVE | IS_ZERO | IS_EMPTY
        | IS_NOT_EMPTY => Some("Boolean"),
        _ => None,
    }
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        ERROR => Some(&[&["code"], &["message"]]),
        LEN => Some(&[&["value"]]),
        TYPE_NAME => Some(&[&["value"]]),
        TO_STRING => Some(&[&["value"], &["precision", "decimals"]]),
        TO_INT => Some(&[&["value"]]),
        TO_FLOAT => Some(&[&["value"]]),
        TO_FIXED => Some(&[&["value"]]),
        TO_BYTE => Some(&[&["value"]]),
        IS_NUMERIC => Some(&[&["value"]]),
        IS_EVEN => Some(&[&["value"]]),
        IS_ODD => Some(&[&["value"]]),
        IS_POSITIVE => Some(&[&["value"]]),
        IS_NEGATIVE => Some(&[&["value"]]),
        IS_ZERO => Some(&[&["value"]]),
        IS_EMPTY => Some(&[&["value"]]),
        IS_NOT_EMPTY => Some(&[&["value"]]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        TO_INT => Some("Integer"),
        TO_FLOAT => Some("Float"),
        TO_FIXED => Some("Fixed"),
        TO_BYTE => Some("Byte"),
        _ => None,
    }
}

pub(crate) fn builtin_function_id(name: &str) -> Option<u32> {
    match name {
        IS_EVEN => Some(BUILTIN_FUNCTION_IS_EVEN),
        IS_ODD => Some(BUILTIN_FUNCTION_IS_ODD),
        IS_POSITIVE => Some(BUILTIN_FUNCTION_IS_POSITIVE),
        IS_NEGATIVE => Some(BUILTIN_FUNCTION_IS_NEGATIVE),
        IS_ZERO => Some(BUILTIN_FUNCTION_IS_ZERO),
        IS_EMPTY => Some(BUILTIN_FUNCTION_IS_EMPTY),
        IS_NOT_EMPTY => Some(BUILTIN_FUNCTION_IS_NOT_EMPTY),
        _ => None,
    }
}

pub(crate) fn builtin_function_id_for_type(name: &str, function_type: &str) -> Option<u32> {
    let (params, returns) = function_parts(function_type)?;
    if params.len() != 1 || returns != "Boolean" {
        return builtin_function_id(name);
    }
    match (name, params[0]) {
        (IS_POSITIVE, "Float") => Some(BUILTIN_FUNCTION_IS_POSITIVE_FLOAT),
        (IS_POSITIVE, "Fixed") => Some(BUILTIN_FUNCTION_IS_POSITIVE_FIXED),
        (IS_NEGATIVE, "Float") => Some(BUILTIN_FUNCTION_IS_NEGATIVE_FLOAT),
        (IS_NEGATIVE, "Fixed") => Some(BUILTIN_FUNCTION_IS_NEGATIVE_FIXED),
        (IS_ZERO, "Float") => Some(BUILTIN_FUNCTION_IS_ZERO_FLOAT),
        (IS_ZERO, "Fixed") => Some(BUILTIN_FUNCTION_IS_ZERO_FIXED),
        _ => builtin_function_id(name),
    }
}

pub(crate) fn filter_predicate_type(name: &str, element_type: &str) -> Option<String> {
    builtin_function_id(name)?;
    let arg_types = vec![element_type.to_string()];
    let resolved = resolve_call(name, &arg_types)?;
    (resolved.return_type == "Boolean").then(|| format!("FUNC({element_type}) AS Boolean"))
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let resolved = match name {
        ERROR => {
            if exact(arg_types, &["Integer", "String"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Error"),
                }
            } else {
                return None;
            }
        }
        LEN => {
            if arg_types.len() != 1 {
                return None;
            }
            if arg_types[0] == "String"
                || arg_types[0].starts_with("List OF ")
                || arg_types[0].starts_with("Map OF ")
            {
                ResolvedCall {
                    return_type: Cow::Borrowed("Integer"),
                }
            } else {
                return None;
            }
        }
        TYPE_NAME => {
            if arg_types.len() == 1 {
                ResolvedCall {
                    return_type: Cow::Borrowed("String"),
                }
            } else {
                return None;
            }
        }
        TO_STRING => {
            if arg_types.len() == 2
                && matches!(arg_types[0].as_str(), "Float" | "Fixed")
                && arg_types[1] == "Byte"
            {
                ResolvedCall {
                    return_type: Cow::Borrowed("String"),
                }
            } else if arg_types.len() == 1
                && (matches!(
                    arg_types[0].as_str(),
                    "Integer" | "Float" | "Fixed" | "Boolean" | "String" | "Byte"
                ) || arg_types[0] == "List OF Byte")
            {
                ResolvedCall {
                    return_type: Cow::Borrowed("String"),
                }
            } else {
                return None;
            }
        }
        TO_INT => {
            if exact_one_of(arg_types, &["String", "Byte", "Float", "Fixed"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Integer"),
                }
            } else {
                return None;
            }
        }
        TO_FLOAT => {
            if exact_one_of(arg_types, &["String", "Integer", "Fixed"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Float"),
                }
            } else {
                return None;
            }
        }
        TO_FIXED => {
            if exact_one_of(arg_types, &["String", "Integer", "Float"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Fixed"),
                }
            } else {
                return None;
            }
        }
        TO_BYTE => {
            if exact(arg_types, &["Integer"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Byte"),
                }
            } else {
                return None;
            }
        }
        IS_NUMERIC => {
            if exact(arg_types, &["String"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Boolean"),
                }
            } else {
                return None;
            }
        }
        IS_EVEN | IS_ODD => {
            if exact(arg_types, &["Integer"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Boolean"),
                }
            } else {
                return None;
            }
        }
        IS_POSITIVE | IS_NEGATIVE | IS_ZERO => {
            if exact_one_of(arg_types, &["Integer", "Float", "Fixed"]) {
                ResolvedCall {
                    return_type: Cow::Borrowed("Boolean"),
                }
            } else {
                return None;
            }
        }
        IS_EMPTY | IS_NOT_EMPTY => {
            if arg_types.len() == 1
                && (arg_types[0] == "String"
                    || arg_types[0].starts_with("List OF ")
                    || arg_types[0].starts_with("Map OF "))
            {
                ResolvedCall {
                    return_type: Cow::Borrowed("Boolean"),
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
        TYPE_NAME => Some("T"),
        TO_STRING => {
            Some("Integer, Float[, Byte], Fixed[, Byte], Boolean, String, Byte, or List OF Byte")
        }
        TO_INT => Some("String, Byte, Float, or Fixed"),
        TO_FLOAT => Some("String, Integer, or Fixed"),
        TO_FIXED => Some("String, Integer, or Float"),
        TO_BYTE => Some("Integer"),
        IS_NUMERIC => Some("String"),
        IS_EVEN => Some("Integer"),
        IS_ODD => Some("Integer"),
        IS_POSITIVE | IS_NEGATIVE | IS_ZERO => Some("Integer, Float, or Fixed"),
        IS_EMPTY | IS_NOT_EMPTY => Some("String, List OF T, or Map OF K TO V"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        LEN | TYPE_NAME | TO_INT | TO_FLOAT | TO_FIXED | TO_BYTE | IS_NUMERIC | IS_EVEN
        | IS_ODD | IS_POSITIVE | IS_NEGATIVE | IS_ZERO | IS_EMPTY | IS_NOT_EMPTY => Some((1, 1)),
        TO_STRING => Some((1, 2)),
        _ => None,
    }
}

/// List-overload resolvers for `find`/`mid`/`replace`, migrated to `collections::`
/// (plan-01-functions.md §5). These keep the original bare-name overload logic so
/// `collections::` can reuse it; the String overloads live in `strings::`.
pub(crate) fn resolve_find_list<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if !(2..=3).contains(&arg_types.len()) {
        return None;
    }
    let element = list_element(&arg_types[0])?;
    (arg_types.get(2).is_none_or(|type_| type_ == "Integer")
        && (arg_types[1] == element || arg_types[1] == arg_types[0]))
        .then_some(ResolvedCall {
            return_type: Cow::Borrowed("Integer"),
        })
}

pub(crate) fn resolve_mid_list<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    (arg_types.len() == 3
        && list_element(&arg_types[0]).is_some()
        && arg_types[1] == "Integer"
        && arg_types[2] == "Integer")
        .then_some(ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        })
}

pub(crate) fn resolve_replace_list<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let element = list_element(&arg_types[0])?;
    (arg_types.len() == 3 && arg_types[1] == element && arg_types[2] == element).then_some(
        ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        },
    )
}

pub(crate) fn resolve_get<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    if let Some(element) = list_element(&arg_types[0]) {
        return (arg_types[1] == "Integer").then_some(ResolvedCall {
            return_type: Cow::Borrowed(element),
        });
    }
    let (key, value) = map_parts(&arg_types[0])?;
    (arg_types[1] == key).then_some(ResolvedCall {
        return_type: Cow::Borrowed(value),
    })
}

pub(crate) fn resolve_get_or<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 3 {
        return None;
    }
    if let Some(element) = list_element(&arg_types[0]) {
        return (arg_types[1] == "Integer" && arg_types[2] == element).then_some(ResolvedCall {
            return_type: Cow::Borrowed(element),
        });
    }
    let (key, value) = map_parts(&arg_types[0])?;
    (arg_types[1] == key && arg_types[2] == value).then_some(ResolvedCall {
        return_type: Cow::Borrowed(value),
    })
}

pub(crate) fn resolve_set<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 3 {
        return None;
    }
    if let Some(element) = list_element(&arg_types[0]) {
        return (arg_types[1] == "Integer" && arg_types[2] == element).then_some(ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        });
    }
    let (key, value) = map_parts(&arg_types[0])?;
    (arg_types[1] == key && arg_types[2] == value).then_some(ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

pub(crate) fn resolve_append<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = list_element(&arg_types[0])?;
    (arg_types[1] == element || arg_types[1] == arg_types[0]).then_some(ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

pub(crate) fn resolve_prepend<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = list_element(&arg_types[0])?;
    (arg_types[1] == element).then_some(ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

pub(crate) fn resolve_insert<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 3 {
        return None;
    }
    let element = list_element(&arg_types[0])?;
    (arg_types[1] == "Integer" && arg_types[2] == element).then_some(ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

pub(crate) fn resolve_remove_at<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    (arg_types.len() == 2 && list_element(&arg_types[0]).is_some() && arg_types[1] == "Integer")
        .then_some(ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        })
}

pub(crate) fn resolve_remove_key<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let (key, _) = map_parts(&arg_types[0])?;
    (arg_types[1] == key).then_some(ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

pub(crate) fn resolve_keys<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 1 {
        return None;
    }
    let (key, _) = map_parts(&arg_types[0])?;
    Some(ResolvedCall {
        return_type: Cow::Owned(format!("List OF {key}")),
    })
}

pub(crate) fn resolve_values<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 1 {
        return None;
    }
    let (_, value) = map_parts(&arg_types[0])?;
    Some(ResolvedCall {
        return_type: Cow::Owned(format!("List OF {value}")),
    })
}

pub(crate) fn resolve_has_key<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let (key, _) = map_parts(&arg_types[0])?;
    (arg_types[1] == key).then_some(ResolvedCall {
        return_type: Cow::Borrowed("Boolean"),
    })
}

pub(crate) fn resolve_contains<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = list_element(&arg_types[0])?;
    (arg_types[1] == element).then_some(ResolvedCall {
        return_type: Cow::Borrowed("Boolean"),
    })
}

pub(crate) fn resolve_sum<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 1 {
        return None;
    }
    match arg_types[0].as_str() {
        "List OF Integer" => Some(ResolvedCall {
            return_type: Cow::Borrowed("Integer"),
        }),
        "List OF Float" => Some(ResolvedCall {
            return_type: Cow::Borrowed("Float"),
        }),
        "List OF Fixed" => Some(ResolvedCall {
            return_type: Cow::Borrowed("Fixed"),
        }),
        _ => None,
    }
}

pub(crate) fn resolve_for_each<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = list_element(&arg_types[0])?;
    let (params, returns) = function_parts(&arg_types[1])?;
    (params.len() == 1 && params[0] == element && returns == "Nothing").then_some(ResolvedCall {
        return_type: Cow::Borrowed("Nothing"),
    })
}

pub(crate) fn resolve_transform<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = list_element(&arg_types[0])?;
    let (params, returns) = function_parts(&arg_types[1])?;
    (params.len() == 1 && params[0] == element && returns != "Nothing").then_some(ResolvedCall {
        return_type: Cow::Owned(format!("List OF {returns}")),
    })
}

pub(crate) fn resolve_filter<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = list_element(&arg_types[0])?;
    let (params, returns) = function_parts(&arg_types[1])?;
    (params.len() == 1 && params[0] == element && returns == "Boolean").then_some(ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

pub(crate) fn resolve_reduce<'a>(arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 3 {
        return None;
    }
    let element = list_element(&arg_types[0])?;
    let (params, returns) = function_parts(&arg_types[2])?;
    (params.len() == 2
        && params[0] == arg_types[1]
        && params[1] == element
        && returns == arg_types[1])
        .then_some(ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[1]),
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
/// The element type of a `List`, with any `RES` ownership-axis marker stripped:
/// a `List OF RES File` yields the borrow element type `File`, since reading or
/// inserting an element works with the bare resource value (§15.6).
fn list_element(type_name: &str) -> Option<&str> {
    let element = type_name.strip_prefix("List OF ")?;
    Some(element.strip_prefix("RES ").unwrap_or(element))
}

fn map_parts(type_name: &str) -> Option<(&str, &str)> {
    let (key, value) = type_name.strip_prefix("Map OF ")?.split_once(" TO ")?;
    Some((key, value.strip_prefix("RES ").unwrap_or(value)))
}

fn function_parts(type_name: &str) -> Option<(Vec<&str>, &str)> {
    let rest = type_name.strip_prefix("FUNC(")?;
    let (params, returns) = rest.split_once(") AS ")?;
    let params = if params.trim().is_empty() {
        Vec::new()
    } else {
        params.split(", ").collect()
    };
    Some((params, returns))
}
