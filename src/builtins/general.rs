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
        TO_INT => Some(&[&["value"], &["text", "base"]]),
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
            // 1-arg: parse base-10 (String) or numeric narrowing (Byte/Float/Fixed).
            // 2-arg: `toInt(text AS String, base AS Integer)` parses `text` in
            // `base` (plan-02-cleanup §5). The optional `base` is a second arity,
            // not a user-level default parameter, since `toInt` is overloaded.
            if exact_one_of(arg_types, &["String", "Byte", "Float", "Fixed"])
                || exact(arg_types, &["String", "Integer"])
            {
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
        TO_INT => Some("String[, Integer], Byte, Float, or Fixed"),
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
        LEN | TYPE_NAME | TO_FLOAT | TO_FIXED | TO_BYTE | IS_NUMERIC | IS_EVEN | IS_ODD
        | IS_POSITIVE | IS_NEGATIVE | IS_ZERO | IS_EMPTY | IS_NOT_EMPTY => Some((1, 1)),
        TO_STRING | TO_INT => Some((1, 2)),
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

/// Splits a `FUNC(<params>) AS <return>` type into its parameter types and its
/// return type.
///
/// A parameter can itself be a function type — `FUNC(FUNC(Integer, Integer) AS
/// Integer) AS Integer` is what `collections::transform` receives over a list of
/// two-argument function values — so the parameter list is scanned with paren
/// depth: the closing paren and the separating commas are the ones at depth 0.
fn function_parts(type_name: &str) -> Option<(Vec<&str>, &str)> {
    let rest = type_name.strip_prefix("FUNC(")?;

    let mut depth = 0usize;
    let mut close = None;
    let mut splits = Vec::new();
    for (index, ch) in rest.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' if depth == 0 => {
                close = Some(index);
                break;
            }
            ')' => depth -= 1,
            ',' if depth == 0 => splits.push(index),
            _ => {}
        }
    }
    let close = close?;
    let returns = rest.get(close..)?.strip_prefix(") AS ")?;

    let params_text = &rest[..close];
    let params = if params_text.trim().is_empty() {
        Vec::new()
    } else {
        let mut params = Vec::with_capacity(splits.len() + 1);
        let mut start = 0;
        for split in splits {
            params.push(params_text[start..split].trim_start());
            start = split + 1;
        }
        params.push(params_text[start..].trim_start());
        params
    };
    Some((params, returns))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn rt(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &strings(args)).map(|r| r.return_type.into_owned())
    }

    const ALL_GENERAL: &[&str] = &[
        ERROR,
        LEN,
        TYPE_NAME,
        TO_STRING,
        TO_INT,
        TO_FLOAT,
        TO_FIXED,
        TO_BYTE,
        IS_NUMERIC,
        IS_EVEN,
        IS_ODD,
        IS_POSITIVE,
        IS_NEGATIVE,
        IS_ZERO,
        IS_EMPTY,
        IS_NOT_EMPTY,
    ];

    #[test]
    fn is_general_call_covers_all() {
        for name in ALL_GENERAL {
            assert!(is_general_call(name), "{name}");
        }
        assert!(!is_general_call("nope"));
        assert!(!is_general_call("collections.get"));
    }

    #[test]
    fn overridable_and_reserved() {
        assert!(!is_overridable(ERROR));
        assert!(reserved_builtin_name(ERROR));
        for name in ALL_GENERAL.iter().filter(|n| **n != ERROR) {
            assert!(is_overridable(name), "{name}");
            assert!(!reserved_builtin_name(name), "{name}");
        }
        assert!(!is_overridable("nope"));
        assert!(!reserved_builtin_name("nope"));
    }

    #[test]
    fn override_result_type_all_arms() {
        assert_eq!(override_result_type(TO_STRING), Some("String"));
        assert_eq!(override_result_type(TYPE_NAME), Some("String"));
        assert_eq!(override_result_type(LEN), Some("Integer"));
        assert_eq!(override_result_type(TO_INT), Some("Integer"));
        assert_eq!(override_result_type(TO_FLOAT), Some("Float"));
        assert_eq!(override_result_type(TO_FIXED), Some("Fixed"));
        assert_eq!(override_result_type(TO_BYTE), Some("Byte"));
        assert_eq!(override_result_type(IS_NUMERIC), Some("Boolean"));
        assert_eq!(override_result_type(IS_NOT_EMPTY), Some("Boolean"));
        assert_eq!(override_result_type(ERROR), None);
        assert_eq!(override_result_type("nope"), None);
    }

    #[test]
    fn call_param_names_all_arms() {
        assert_eq!(call_param_names(ERROR).unwrap().len(), 2);
        assert_eq!(call_param_names(LEN).unwrap().len(), 1);
        assert_eq!(call_param_names(TYPE_NAME).unwrap().len(), 1);
        assert_eq!(call_param_names(TO_STRING).unwrap().len(), 2);
        assert_eq!(call_param_names(TO_INT).unwrap().len(), 2);
        assert_eq!(call_param_names(TO_FLOAT).unwrap().len(), 1);
        assert_eq!(call_param_names(TO_FIXED).unwrap().len(), 1);
        assert_eq!(call_param_names(TO_BYTE).unwrap().len(), 1);
        assert_eq!(call_param_names(IS_NUMERIC).unwrap().len(), 1);
        assert_eq!(call_param_names(IS_EVEN).unwrap().len(), 1);
        assert_eq!(call_param_names(IS_ODD).unwrap().len(), 1);
        assert_eq!(call_param_names(IS_POSITIVE).unwrap().len(), 1);
        assert_eq!(call_param_names(IS_NEGATIVE).unwrap().len(), 1);
        assert_eq!(call_param_names(IS_ZERO).unwrap().len(), 1);
        assert_eq!(call_param_names(IS_EMPTY).unwrap().len(), 1);
        assert_eq!(call_param_names(IS_NOT_EMPTY).unwrap().len(), 1);
        assert!(call_param_names("nope").is_none());
    }

    #[test]
    fn call_return_type_name_arms() {
        assert_eq!(call_return_type_name(TO_INT), Some("Integer"));
        assert_eq!(call_return_type_name(TO_FLOAT), Some("Float"));
        assert_eq!(call_return_type_name(TO_FIXED), Some("Fixed"));
        assert_eq!(call_return_type_name(TO_BYTE), Some("Byte"));
        assert_eq!(call_return_type_name(LEN), None);
        assert_eq!(call_return_type_name("nope"), None);
    }

    #[test]
    fn builtin_function_id_arms() {
        assert_eq!(builtin_function_id(IS_EVEN), Some(BUILTIN_FUNCTION_IS_EVEN));
        assert_eq!(builtin_function_id(IS_ODD), Some(BUILTIN_FUNCTION_IS_ODD));
        assert_eq!(
            builtin_function_id(IS_POSITIVE),
            Some(BUILTIN_FUNCTION_IS_POSITIVE)
        );
        assert_eq!(
            builtin_function_id(IS_NEGATIVE),
            Some(BUILTIN_FUNCTION_IS_NEGATIVE)
        );
        assert_eq!(builtin_function_id(IS_ZERO), Some(BUILTIN_FUNCTION_IS_ZERO));
        assert_eq!(
            builtin_function_id(IS_EMPTY),
            Some(BUILTIN_FUNCTION_IS_EMPTY)
        );
        assert_eq!(
            builtin_function_id(IS_NOT_EMPTY),
            Some(BUILTIN_FUNCTION_IS_NOT_EMPTY)
        );
        assert_eq!(builtin_function_id(LEN), None);
        assert_eq!(builtin_function_id("nope"), None);
    }

    #[test]
    fn builtin_function_id_for_type_specialized() {
        assert_eq!(
            builtin_function_id_for_type(IS_POSITIVE, "FUNC(Float) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_POSITIVE_FLOAT)
        );
        assert_eq!(
            builtin_function_id_for_type(IS_POSITIVE, "FUNC(Fixed) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_POSITIVE_FIXED)
        );
        assert_eq!(
            builtin_function_id_for_type(IS_NEGATIVE, "FUNC(Float) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_NEGATIVE_FLOAT)
        );
        assert_eq!(
            builtin_function_id_for_type(IS_NEGATIVE, "FUNC(Fixed) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_NEGATIVE_FIXED)
        );
        assert_eq!(
            builtin_function_id_for_type(IS_ZERO, "FUNC(Float) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_ZERO_FLOAT)
        );
        assert_eq!(
            builtin_function_id_for_type(IS_ZERO, "FUNC(Fixed) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_ZERO_FIXED)
        );
        // Integer element falls through to the plain id.
        assert_eq!(
            builtin_function_id_for_type(IS_POSITIVE, "FUNC(Integer) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_POSITIVE)
        );
        // Non-predicate specialization name (isEven) -> plain id.
        assert_eq!(
            builtin_function_id_for_type(IS_EVEN, "FUNC(Integer) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_EVEN)
        );
    }

    #[test]
    fn builtin_function_id_for_type_non_predicate_shape() {
        // Not a single-param Boolean predicate -> falls back to builtin_function_id.
        assert_eq!(
            builtin_function_id_for_type(IS_EVEN, "FUNC(Integer, Integer) AS Boolean"),
            Some(BUILTIN_FUNCTION_IS_EVEN)
        );
        assert_eq!(
            builtin_function_id_for_type(IS_EVEN, "FUNC(Integer) AS Integer"),
            Some(BUILTIN_FUNCTION_IS_EVEN)
        );
        // Not a FUNC type at all -> None from function_parts.
        assert_eq!(builtin_function_id_for_type(IS_EVEN, "Integer"), None);
    }

    #[test]
    fn filter_predicate_type_cases() {
        assert_eq!(
            filter_predicate_type(IS_EVEN, "Integer"),
            Some("FUNC(Integer) AS Boolean".to_string())
        );
        assert_eq!(
            filter_predicate_type(IS_POSITIVE, "Float"),
            Some("FUNC(Float) AS Boolean".to_string())
        );
        // Not a builtin_function_id name -> None.
        assert_eq!(filter_predicate_type(LEN, "String"), None);
        // Element type the predicate does not resolve for -> None.
        assert_eq!(filter_predicate_type(IS_EVEN, "String"), None);
    }

    #[test]
    fn resolve_error() {
        assert_eq!(rt(ERROR, &["Integer", "String"]), Some("Error".to_string()));
        assert_eq!(rt(ERROR, &["String", "Integer"]), None);
        assert_eq!(rt(ERROR, &["Integer"]), None);
    }

    #[test]
    fn resolve_len() {
        assert_eq!(rt(LEN, &["String"]), Some("Integer".to_string()));
        assert_eq!(rt(LEN, &["List OF Integer"]), Some("Integer".to_string()));
        assert_eq!(
            rt(LEN, &["Map OF String TO Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(rt(LEN, &["Integer"]), None);
        assert_eq!(rt(LEN, &["String", "String"]), None);
    }

    #[test]
    fn resolve_type_name() {
        assert_eq!(rt(TYPE_NAME, &["Anything"]), Some("String".to_string()));
        assert_eq!(rt(TYPE_NAME, &["a", "b"]), None);
    }

    #[test]
    fn resolve_to_string() {
        assert_eq!(
            rt(TO_STRING, &["Float", "Byte"]),
            Some("String".to_string())
        );
        assert_eq!(
            rt(TO_STRING, &["Fixed", "Byte"]),
            Some("String".to_string())
        );
        assert_eq!(rt(TO_STRING, &["Integer"]), Some("String".to_string()));
        assert_eq!(rt(TO_STRING, &["Boolean"]), Some("String".to_string()));
        assert_eq!(rt(TO_STRING, &["List OF Byte"]), Some("String".to_string()));
        assert_eq!(rt(TO_STRING, &["Integer", "Byte"]), None); // Integer has no 2-arg form
        assert_eq!(rt(TO_STRING, &["Float", "Integer"]), None);
        assert_eq!(rt(TO_STRING, &["List OF Integer"]), None);
    }

    #[test]
    fn resolve_to_int() {
        assert_eq!(rt(TO_INT, &["String"]), Some("Integer".to_string()));
        assert_eq!(rt(TO_INT, &["Byte"]), Some("Integer".to_string()));
        assert_eq!(rt(TO_INT, &["Float"]), Some("Integer".to_string()));
        assert_eq!(rt(TO_INT, &["Fixed"]), Some("Integer".to_string()));
        assert_eq!(
            rt(TO_INT, &["String", "Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(rt(TO_INT, &["Boolean"]), None);
        assert_eq!(rt(TO_INT, &["Integer", "Integer"]), None);
    }

    #[test]
    fn resolve_to_float_fixed_byte() {
        assert_eq!(rt(TO_FLOAT, &["String"]), Some("Float".to_string()));
        assert_eq!(rt(TO_FLOAT, &["Integer"]), Some("Float".to_string()));
        assert_eq!(rt(TO_FLOAT, &["Fixed"]), Some("Float".to_string()));
        assert_eq!(rt(TO_FLOAT, &["Boolean"]), None);
        assert_eq!(rt(TO_FIXED, &["String"]), Some("Fixed".to_string()));
        assert_eq!(rt(TO_FIXED, &["Integer"]), Some("Fixed".to_string()));
        assert_eq!(rt(TO_FIXED, &["Float"]), Some("Fixed".to_string()));
        assert_eq!(rt(TO_FIXED, &["Boolean"]), None);
        assert_eq!(rt(TO_BYTE, &["Integer"]), Some("Byte".to_string()));
        assert_eq!(rt(TO_BYTE, &["String"]), None);
    }

    #[test]
    fn resolve_predicates() {
        assert_eq!(rt(IS_NUMERIC, &["String"]), Some("Boolean".to_string()));
        assert_eq!(rt(IS_NUMERIC, &["Integer"]), None);
        assert_eq!(rt(IS_EVEN, &["Integer"]), Some("Boolean".to_string()));
        assert_eq!(rt(IS_ODD, &["Integer"]), Some("Boolean".to_string()));
        assert_eq!(rt(IS_EVEN, &["Float"]), None);
        assert_eq!(rt(IS_POSITIVE, &["Integer"]), Some("Boolean".to_string()));
        assert_eq!(rt(IS_NEGATIVE, &["Float"]), Some("Boolean".to_string()));
        assert_eq!(rt(IS_ZERO, &["Fixed"]), Some("Boolean".to_string()));
        assert_eq!(rt(IS_POSITIVE, &["String"]), None);
        assert_eq!(rt(IS_EMPTY, &["String"]), Some("Boolean".to_string()));
        assert_eq!(
            rt(IS_EMPTY, &["List OF Integer"]),
            Some("Boolean".to_string())
        );
        assert_eq!(
            rt(IS_NOT_EMPTY, &["Map OF String TO Integer"]),
            Some("Boolean".to_string())
        );
        assert_eq!(rt(IS_NOT_EMPTY, &["Integer"]), None);
    }

    #[test]
    fn resolve_call_unknown() {
        assert_eq!(rt("nope", &["Integer"]), None);
    }

    #[test]
    fn expected_arguments_all_arms() {
        assert!(expected_arguments(LEN).is_some());
        assert!(expected_arguments(TYPE_NAME).is_some());
        assert!(expected_arguments(TO_STRING).is_some());
        assert!(expected_arguments(TO_INT).is_some());
        assert!(expected_arguments(TO_FLOAT).is_some());
        assert!(expected_arguments(TO_FIXED).is_some());
        assert!(expected_arguments(TO_BYTE).is_some());
        assert!(expected_arguments(IS_NUMERIC).is_some());
        assert!(expected_arguments(IS_EVEN).is_some());
        assert!(expected_arguments(IS_ODD).is_some());
        assert!(expected_arguments(IS_POSITIVE).is_some());
        assert!(expected_arguments(IS_NEGATIVE).is_some());
        assert!(expected_arguments(IS_ZERO).is_some());
        assert!(expected_arguments(IS_EMPTY).is_some());
        assert!(expected_arguments(IS_NOT_EMPTY).is_some());
        assert!(expected_arguments(ERROR).is_none());
        assert!(expected_arguments("nope").is_none());
    }

    #[test]
    fn arity_all_arms() {
        assert_eq!(arity(LEN), Some((1, 1)));
        assert_eq!(arity(TYPE_NAME), Some((1, 1)));
        assert_eq!(arity(TO_FLOAT), Some((1, 1)));
        assert_eq!(arity(TO_FIXED), Some((1, 1)));
        assert_eq!(arity(TO_BYTE), Some((1, 1)));
        assert_eq!(arity(IS_NUMERIC), Some((1, 1)));
        assert_eq!(arity(IS_EMPTY), Some((1, 1)));
        assert_eq!(arity(IS_NOT_EMPTY), Some((1, 1)));
        assert_eq!(arity(TO_STRING), Some((1, 2)));
        assert_eq!(arity(TO_INT), Some((1, 2)));
        assert_eq!(arity(ERROR), None);
        assert_eq!(arity("nope"), None);
    }

    fn rc(r: Option<ResolvedCall>) -> Option<String> {
        r.map(|r| r.return_type.into_owned())
    }

    #[test]
    fn resolve_find_list_cases() {
        assert_eq!(
            rc(resolve_find_list(&strings(&["List OF Integer", "Integer"]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_find_list(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("Integer".to_string())
        );
        // sublist search (arg1 == whole list type)
        assert_eq!(
            rc(resolve_find_list(&strings(&[
                "List OF Integer",
                "List OF Integer"
            ]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_find_list(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(
            rc(resolve_find_list(&strings(&["Integer", "Integer"]))),
            None
        );
        assert_eq!(rc(resolve_find_list(&strings(&["List OF Integer"]))), None);
        assert_eq!(
            rc(resolve_find_list(&strings(&[
                "List OF Integer",
                "Integer",
                "String"
            ]))),
            None
        );
    }

    #[test]
    fn resolve_mid_list_cases() {
        assert_eq!(
            rc(resolve_mid_list(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_mid_list(&strings(&[
                "List OF Integer",
                "Integer",
                "String"
            ]))),
            None
        );
        assert_eq!(
            rc(resolve_mid_list(&strings(&[
                "Integer", "Integer", "Integer"
            ]))),
            None
        );
    }

    #[test]
    fn resolve_replace_list_cases() {
        assert_eq!(
            rc(resolve_replace_list(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_replace_list(&strings(&[
                "List OF Integer",
                "Integer",
                "String"
            ]))),
            None
        );
        assert_eq!(rc(resolve_replace_list(&strings(&["Integer"]))), None);
    }

    #[test]
    fn resolve_get_and_getor() {
        assert_eq!(
            rc(resolve_get(&strings(&["List OF Integer", "Integer"]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_get(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(
            rc(resolve_get(&strings(&[
                "Map OF String TO Integer",
                "String"
            ]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_get(&strings(&[
                "Map OF String TO Integer",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_get(&strings(&["Integer", "Integer"]))), None);
        assert_eq!(rc(resolve_get(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_get_or(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_get_or(&strings(&[
                "List OF Integer",
                "Integer",
                "String"
            ]))),
            None
        );
        assert_eq!(
            rc(resolve_get_or(&strings(&[
                "Map OF String TO Integer",
                "String",
                "Integer"
            ]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_get_or(&strings(&[
                "Map OF String TO Integer",
                "String",
                "String"
            ]))),
            None
        );
        assert_eq!(rc(resolve_get_or(&strings(&["List OF Integer"]))), None);
    }

    #[test]
    fn resolve_set_cases() {
        assert_eq!(
            rc(resolve_set(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_set(&strings(&[
                "List OF Integer",
                "String",
                "Integer"
            ]))),
            None
        );
        assert_eq!(
            rc(resolve_set(&strings(&[
                "Map OF String TO Integer",
                "String",
                "Integer"
            ]))),
            Some("Map OF String TO Integer".to_string())
        );
        assert_eq!(
            rc(resolve_set(&strings(&[
                "Map OF String TO Integer",
                "Integer",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_set(&strings(&["Integer", "a", "b"]))), None);
        assert_eq!(rc(resolve_set(&strings(&["List OF Integer"]))), None);
    }

    #[test]
    fn resolve_append_prepend_insert() {
        assert_eq!(
            rc(resolve_append(&strings(&["List OF Integer", "Integer"]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_append(&strings(&[
                "List OF Integer",
                "List OF Integer"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_append(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(rc(resolve_append(&strings(&["Integer", "Integer"]))), None);
        assert_eq!(rc(resolve_append(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_prepend(&strings(&["List OF Integer", "Integer"]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_prepend(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(rc(resolve_prepend(&strings(&["Integer", "Integer"]))), None);
        assert_eq!(rc(resolve_prepend(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_insert(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_insert(&strings(&[
                "List OF Integer",
                "String",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_insert(&strings(&["Integer", "a", "b"]))), None);
        assert_eq!(rc(resolve_insert(&strings(&["List OF Integer"]))), None);
    }

    #[test]
    fn resolve_remove_at_and_key() {
        assert_eq!(
            rc(resolve_remove_at(&strings(&["List OF Integer", "Integer"]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_remove_at(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(
            rc(resolve_remove_at(&strings(&["Integer", "Integer"]))),
            None
        );

        assert_eq!(
            rc(resolve_remove_key(&strings(&[
                "Map OF String TO Integer",
                "String"
            ]))),
            Some("Map OF String TO Integer".to_string())
        );
        assert_eq!(
            rc(resolve_remove_key(&strings(&[
                "Map OF String TO Integer",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_remove_key(&strings(&["Integer", "a"]))), None);
        assert_eq!(
            rc(resolve_remove_key(&strings(&["Map OF String TO Integer"]))),
            None
        );
    }

    #[test]
    fn resolve_keys_values() {
        assert_eq!(
            rc(resolve_keys(&strings(&["Map OF String TO Integer"]))),
            Some("List OF String".to_string())
        );
        assert_eq!(rc(resolve_keys(&strings(&["Integer"]))), None);
        assert_eq!(
            rc(resolve_keys(&strings(&["Map OF String TO Integer", "x"]))),
            None
        );
        assert_eq!(
            rc(resolve_values(&strings(&["Map OF String TO Integer"]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(rc(resolve_values(&strings(&["Integer"]))), None);
        assert_eq!(
            rc(resolve_values(&strings(&["Map OF String TO Integer", "x"]))),
            None
        );
    }

    #[test]
    fn resolve_has_key_contains() {
        assert_eq!(
            rc(resolve_has_key(&strings(&[
                "Map OF String TO Integer",
                "String"
            ]))),
            Some("Boolean".to_string())
        );
        assert_eq!(
            rc(resolve_has_key(&strings(&[
                "Map OF String TO Integer",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_has_key(&strings(&["Integer", "a"]))), None);
        assert_eq!(
            rc(resolve_has_key(&strings(&["Map OF String TO Integer"]))),
            None
        );

        assert_eq!(
            rc(resolve_contains(&strings(&["List OF Integer", "Integer"]))),
            Some("Boolean".to_string())
        );
        assert_eq!(
            rc(resolve_contains(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(
            rc(resolve_contains(&strings(&["Integer", "Integer"]))),
            None
        );
        assert_eq!(rc(resolve_contains(&strings(&["List OF Integer"]))), None);
    }

    #[test]
    fn resolve_sum_cases() {
        assert_eq!(
            rc(resolve_sum(&strings(&["List OF Integer"]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_sum(&strings(&["List OF Float"]))),
            Some("Float".to_string())
        );
        assert_eq!(
            rc(resolve_sum(&strings(&["List OF Fixed"]))),
            Some("Fixed".to_string())
        );
        assert_eq!(rc(resolve_sum(&strings(&["List OF String"]))), None);
        assert_eq!(rc(resolve_sum(&strings(&["List OF Integer", "x"]))), None);
    }

    #[test]
    fn resolve_for_each_transform_filter_reduce() {
        assert_eq!(
            rc(resolve_for_each(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS Nothing"
            ]))),
            Some("Nothing".to_string())
        );
        // wrong return
        assert_eq!(
            rc(resolve_for_each(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS Boolean"
            ]))),
            None
        );
        // wrong element
        assert_eq!(
            rc(resolve_for_each(&strings(&[
                "List OF Integer",
                "FUNC(String) AS Nothing"
            ]))),
            None
        );
        assert_eq!(rc(resolve_for_each(&strings(&["Integer", "x"]))), None);
        assert_eq!(rc(resolve_for_each(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_transform(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS String"
            ]))),
            Some("List OF String".to_string())
        );
        assert_eq!(
            rc(resolve_transform(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS Nothing"
            ]))),
            None
        );
        assert_eq!(rc(resolve_transform(&strings(&["Integer", "x"]))), None);
        assert_eq!(rc(resolve_transform(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_filter(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS Boolean"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_filter(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_filter(&strings(&["Integer", "x"]))), None);
        assert_eq!(rc(resolve_filter(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_reduce(&strings(&[
                "List OF Integer",
                "String",
                "FUNC(String, Integer) AS String"
            ]))),
            Some("String".to_string())
        );
        assert_eq!(
            rc(resolve_reduce(&strings(&[
                "List OF Integer",
                "String",
                "FUNC(String, Integer) AS Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_reduce(&strings(&["Integer", "a", "b"]))), None);
        assert_eq!(
            rc(resolve_reduce(&strings(&["List OF Integer", "String"]))),
            None
        );
    }

    #[test]
    fn helpers_exact_and_one_of() {
        assert!(exact(
            &strings(&["Integer", "String"]),
            &["Integer", "String"]
        ));
        assert!(!exact(&strings(&["Integer"]), &["Integer", "String"]));
        assert!(!exact(&strings(&["String"]), &["Integer"]));
        assert!(exact_one_of(&strings(&["String"]), &["String", "Integer"]));
        assert!(!exact_one_of(
            &strings(&["Boolean"]),
            &["String", "Integer"]
        ));
        assert!(!exact_one_of(&strings(&["String", "Integer"]), &["String"]));
    }

    #[test]
    fn helpers_list_map_function_parts() {
        assert_eq!(list_element("List OF Integer"), Some("Integer"));
        assert_eq!(list_element("List OF RES File"), Some("File"));
        assert_eq!(list_element("Integer"), None);
        assert_eq!(
            map_parts("Map OF String TO Integer"),
            Some(("String", "Integer"))
        );
        assert_eq!(
            map_parts("Map OF String TO RES File"),
            Some(("String", "File"))
        );
        assert_eq!(map_parts("Integer"), None);
        assert_eq!(map_parts("Map OF String"), None);
        assert_eq!(
            function_parts("FUNC(Integer, String) AS Boolean"),
            Some((vec!["Integer", "String"], "Boolean"))
        );
        assert_eq!(
            function_parts("FUNC() AS Nothing"),
            Some((vec![], "Nothing"))
        );
        assert_eq!(function_parts("Integer"), None);
        assert_eq!(function_parts("FUNC(Integer)"), None);
    }

    #[test]
    fn function_parts_splits_nested_function_parameters() {
        // A flat `split_once(") AS ")` cut at the *inner* `) AS `, yielding the
        // garbage params ["FUNC(Integer", "Integer"] and return "Integer) AS X".
        assert_eq!(
            function_parts("FUNC(FUNC(Integer, Integer) AS Integer) AS Integer"),
            Some((vec!["FUNC(Integer, Integer) AS Integer"], "Integer"))
        );
        assert_eq!(
            function_parts("FUNC(String, FUNC(Integer, Integer) AS Integer) AS Boolean"),
            Some((
                vec!["String", "FUNC(Integer, Integer) AS Integer"],
                "Boolean"
            ))
        );
        // The return type may itself be a function type.
        assert_eq!(
            function_parts("FUNC(Integer) AS FUNC(Integer) AS Integer"),
            Some((vec!["Integer"], "FUNC(Integer) AS Integer"))
        );
        // An unbalanced parameter list has no top-level close paren.
        assert_eq!(function_parts("FUNC(FUNC(Integer) AS Integer"), None);
    }

    #[test]
    fn higher_order_resolvers_accept_function_valued_elements() {
        // `transform` over a list of two-argument function values: the mapper's
        // sole parameter *is* the element type, so the call must resolve.
        let element = "FUNC(Integer, Integer) AS Integer";
        let mapper = strings(&[
            &format!("List OF {element}"),
            &format!("FUNC({element}) AS String"),
        ]);
        let resolved =
            resolve_transform(&mapper).expect("transform over function-valued elements resolves");
        assert_eq!(resolved.return_type, "List OF String");

        let predicate = strings(&[
            &format!("List OF {element}"),
            &format!("FUNC({element}) AS Boolean"),
        ]);
        let resolved =
            resolve_filter(&predicate).expect("filter over function-valued elements resolves");
        assert_eq!(resolved.return_type, format!("List OF {element}"));

        // A mapper whose parameter is a *different* function type still fails.
        let mismatched = strings(&[
            &format!("List OF {element}"),
            "FUNC(FUNC(String) AS Integer) AS String",
        ]);
        assert!(resolve_transform(&mismatched).is_none());
    }
}
