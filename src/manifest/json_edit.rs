use crate::json_string;
use crate::manifest::package::ProjectPackageDependency;
use std::collections::HashMap;
use tinyjson::JsonValue;

pub(crate) fn project_json_with_package(
    contents: &str,
    manifest: &HashMap<String, JsonValue>,
    dependency: &ProjectPackageDependency,
) -> Result<String, String> {
    let packages = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>());

    if packages.is_some_and(|packages| {
        packages.iter().any(|package| {
            package
                .get::<HashMap<String, JsonValue>>()
                .and_then(|package| package.get("name"))
                .and_then(|name| name.get::<String>())
                == Some(&dependency.name)
        })
    }) {
        return Err(format!(
            "project.json already declares package `{}`",
            dependency.name
        ));
    }

    let entry = package_dependency_json(dependency, 4);
    if packages.is_some() {
        insert_package_dependency(contents, &entry)
    } else {
        insert_packages_array(contents, &entry)
    }
}

pub(super) fn package_dependency_json(
    dependency: &ProjectPackageDependency,
    indent: usize,
) -> String {
    let pad = " ".repeat(indent);
    let field_pad = " ".repeat(indent + 2);
    let ident_key = if dependency.ident_key.is_empty() {
        String::new()
    } else {
        format!(
            ",\n{field_pad}\"identKey\": {}",
            json_string(&dependency.ident_key)
        )
    };
    format!(
        "{pad}{{\n{field_pad}\"name\": {},\n{field_pad}\"ident\": {},\n{field_pad}\"version\": {},\n{field_pad}\"pin\": {},\n{field_pad}\"source\": {}{ident_key}\n{pad}}}",
        json_string(&dependency.name),
        json_string(&dependency.ident),
        json_string(&dependency.version),
        dependency.pin,
        json_string(&dependency.source),
        pad = pad,
        field_pad = field_pad,
    )
}

pub(super) fn insert_package_dependency(contents: &str, entry: &str) -> Result<String, String> {
    let Some((array_start, array_end)) = json_array_bounds(contents, "packages") else {
        return Err("could not locate project.json `packages` array".to_string());
    };
    let inner = &contents[array_start + 1..array_end];
    let has_entries = !inner.trim().is_empty();
    let before_entry = contents[..array_end].trim_end_matches([' ', '\t', '\r', '\n']);
    let closing_indent = &contents[before_entry.len()..array_end];

    let mut updated = String::new();
    updated.push_str(before_entry);
    if has_entries {
        updated.push(',');
    }
    updated.push('\n');
    updated.push_str(entry);
    updated.push_str(closing_indent);
    updated.push_str(&contents[array_end..]);
    Ok(updated)
}

pub(super) fn insert_packages_array(contents: &str, entry: &str) -> Result<String, String> {
    let Some(root_end) = root_object_end(contents) else {
        return Err("could not locate end of project.json object".to_string());
    };
    let before = contents[..root_end].trim_end_matches([' ', '\t', '\r', '\n']);
    let between = &contents[before.len()..root_end];
    let needs_comma = before.as_bytes().last().is_some_and(|byte| *byte != b'{');

    let mut updated = String::new();
    updated.push_str(before);
    if needs_comma {
        updated.push(',');
    }
    updated.push_str("\n  \"packages\": [\n");
    updated.push_str(entry);
    updated.push_str("\n  ]");
    updated.push_str(between);
    updated.push_str(&contents[root_end..]);
    Ok(updated)
}

/// Rewrite (or insert) the pinned `identKey` of the dependency named `name`
/// in `project.json`, preserving the file's formatting (plan-23-B2
/// pin-follow after an ident rotation).
pub(crate) fn project_json_with_updated_ident_key(
    contents: &str,
    name: &str,
    new_key: &str,
) -> Result<String, String> {
    let Some((array_start, array_end)) = json_array_bounds(contents, "packages") else {
        return Err("could not locate project.json `packages` array".to_string());
    };
    let mut cursor = array_start + 1;
    while cursor < array_end {
        let Some(object_start) = contents[cursor..array_end].find('{').map(|at| cursor + at) else {
            break;
        };
        let Some(object_end) = matching_json_delimiter(contents, object_start, b'{', b'}') else {
            return Err("malformed project.json `packages` entry".to_string());
        };
        let object = &contents[object_start..=object_end];
        let is_target = object
            .parse::<JsonValue>()
            .ok()
            .and_then(|value| {
                value
                    .get::<HashMap<String, JsonValue>>()
                    .and_then(|entry| entry.get("name"))
                    .and_then(|value| value.get::<String>())
                    .cloned()
            })
            .is_some_and(|entry_name| entry_name == name);
        if !is_target {
            cursor = object_end + 1;
            continue;
        }
        let mut updated = String::new();
        if let Some(field_at) = json_field_name_position(object, "identKey")
            .or_else(|| json_field_name_position(object, "ident_key"))
        {
            let field_len = if object[field_at..].starts_with("\"identKey\"") {
                "\"identKey\"".len()
            } else {
                "\"ident_key\"".len()
            };
            let colon = find_json_punct(object, field_at + field_len, b':')
                .ok_or_else(|| "malformed identKey field".to_string())?;
            let value_start = next_json_string_start(object, colon + 1)
                .ok_or_else(|| "malformed identKey value".to_string())?;
            let value_end = json_string_end(object, value_start)
                .ok_or_else(|| "malformed identKey value".to_string())?;
            updated.push_str(&contents[..object_start + value_start]);
            updated.push_str(&json_string(new_key));
            updated.push_str(&contents[object_start + value_end..]);
        } else {
            // No pin recorded yet: append the field before the closing brace.
            let before_close = object[..object.len() - 1].trim_end_matches([' ', '\t', '\r', '\n']);
            let closing = &object[before_close.len()..];
            updated.push_str(&contents[..object_start]);
            updated.push_str(before_close);
            updated.push_str(",\n      \"identKey\": ");
            updated.push_str(&json_string(new_key));
            updated.push_str(closing);
            updated.push_str(&contents[object_end + 1..]);
        }
        return Ok(updated);
    }
    Err(format!("project.json does not declare package `{name}`"))
}

/// Remove one or more dependency entries by `ident` (plan-60-F §4.3).
///
/// Rebuilds the `packages` array from the entries that survive, rather than
/// splicing each removal out of the text — splicing has to reason about which
/// comma belongs to which neighbour, and gets the first/last/only cases wrong in
/// different ways. Surviving entries keep their **original text** byte for byte,
/// so per-entry formatting and comments survive; only the separators between
/// them are regenerated.
///
/// Removing every entry leaves `"packages": []`, which is what plan-60-B §4.3's
/// zero-dependency path expects.
pub(crate) fn project_json_without_packages(
    contents: &str,
    idents: &[&str],
) -> Result<String, String> {
    let Some((array_start, array_end)) = json_array_bounds(contents, "packages") else {
        return Err("could not locate project.json `packages` array".to_string());
    };

    let mut kept: Vec<&str> = Vec::new();
    let mut removed = 0usize;
    let mut cursor = array_start + 1;
    while cursor < array_end {
        let Some(object_start) = contents[cursor..array_end].find('{').map(|at| cursor + at) else {
            break;
        };
        let Some(object_end) = matching_json_delimiter(contents, object_start, b'{', b'}') else {
            return Err("malformed project.json `packages` entry".to_string());
        };
        let object = &contents[object_start..=object_end];
        let entry_ident = object
            .parse::<JsonValue>()
            .ok()
            .and_then(|value| value.get::<HashMap<String, JsonValue>>().cloned())
            .and_then(|entry| {
                entry
                    .get("ident")
                    .or_else(|| entry.get("name"))
                    .and_then(|value| value.get::<String>())
                    .cloned()
            });
        match entry_ident {
            Some(ident) if idents.contains(&ident.as_str()) => removed += 1,
            _ => kept.push(object),
        }
        cursor = object_end + 1;
    }

    if removed == 0 {
        return Err(format!(
            "project.json declares none of: {}",
            idents.join(", ")
        ));
    }

    let mut array = String::from("[");
    if !kept.is_empty() {
        for (index, object) in kept.iter().enumerate() {
            array.push_str(if index == 0 { "\n    " } else { ",\n    " });
            array.push_str(object);
        }
        array.push_str("\n  ");
    }
    array.push(']');

    let mut out = String::new();
    out.push_str(&contents[..array_start]);
    out.push_str(&array);
    out.push_str(&contents[array_end + 1..]);
    Ok(out)
}

/// Rewrite a declared dependency's `version`, and optionally its `pin`, by
/// surgical string edit (plan-60-E §4.1).
///
/// Matched on **`ident`**, not `name`: the targeted `mfb pkg update` form names
/// an `<owner>#<package>` ident, and `ident` defaults to `name` when absent, so
/// this finds a bare-name entry too.
///
/// `new_pin` is `None` for the default behavior — **pin state is preserved**
/// unless the user passed `--pin`/`--no-pin`. That is the whole point of the
/// targeted form: bumping a floating dependency's ABI floor must not silently
/// pin it, and re-pinning a pinned one must not silently unpin it.
///
/// Like its sibling `project_json_with_updated_ident_key`, this edits the text
/// rather than re-serializing the manifest, so formatting and comments survive.
pub(crate) fn project_json_with_updated_version(
    contents: &str,
    ident: &str,
    new_version: &str,
    new_pin: Option<bool>,
) -> Result<String, String> {
    let Some((array_start, array_end)) = json_array_bounds(contents, "packages") else {
        return Err("could not locate project.json `packages` array".to_string());
    };
    let mut cursor = array_start + 1;
    while cursor < array_end {
        let Some(object_start) = contents[cursor..array_end].find('{').map(|at| cursor + at) else {
            break;
        };
        let Some(object_end) = matching_json_delimiter(contents, object_start, b'{', b'}') else {
            return Err("malformed project.json `packages` entry".to_string());
        };
        let object = &contents[object_start..=object_end];
        let entry = object
            .parse::<JsonValue>()
            .ok()
            .and_then(|value| value.get::<HashMap<String, JsonValue>>().cloned());
        let entry_ident = entry.as_ref().and_then(|entry| {
            entry
                .get("ident")
                .or_else(|| entry.get("name"))
                .and_then(|value| value.get::<String>())
                .cloned()
        });
        if entry_ident.as_deref() != Some(ident) {
            cursor = object_end + 1;
            continue;
        }

        // Rewrite `version` in place. Every dependency this command can target
        // already has one, so an absent field is a malformed entry rather than
        // something to append.
        let field_at = json_field_name_position(object, "version")
            .ok_or_else(|| format!("project.json entry for `{ident}` has no `version` field"))?;
        let colon = find_json_punct(object, field_at + "\"version\"".len(), b':')
            .ok_or_else(|| "malformed version field".to_string())?;
        let value_start = next_json_string_start(object, colon + 1)
            .ok_or_else(|| "malformed version value".to_string())?;
        let value_end = json_string_end(object, value_start)
            .ok_or_else(|| "malformed version value".to_string())?;

        let mut rewritten = String::new();
        rewritten.push_str(&object[..value_start]);
        rewritten.push_str(&json_string(new_version));
        rewritten.push_str(&object[value_end..]);

        // Only touch `pin` when explicitly asked to.
        if let Some(pin) = new_pin {
            rewritten = rewrite_pin_field(&rewritten, pin)?;
        }

        let mut updated = String::new();
        updated.push_str(&contents[..object_start]);
        updated.push_str(&rewritten);
        updated.push_str(&contents[object_end + 1..]);
        return Ok(updated);
    }
    Err(format!("project.json does not declare package `{ident}`"))
}

/// Set a dependency object's `pin` to `pin`, adding the field if absent.
pub(super) fn rewrite_pin_field(object: &str, pin: bool) -> Result<String, String> {
    let literal = if pin { "true" } else { "false" };
    if let Some(field_at) = json_field_name_position(object, "pin") {
        let colon = find_json_punct(object, field_at + "\"pin\"".len(), b':')
            .ok_or_else(|| "malformed pin field".to_string())?;
        // The value is a bare `true`/`false` literal, not a string, so scan for
        // its extent rather than reusing the string helpers.
        let value_start = object[colon + 1..]
            .find(|c: char| !c.is_whitespace())
            .map(|at| colon + 1 + at)
            .ok_or_else(|| "malformed pin value".to_string())?;
        let value_end = object[value_start..]
            .find(|c: char| c == ',' || c == '}' || c.is_whitespace())
            .map(|at| value_start + at)
            .ok_or_else(|| "malformed pin value".to_string())?;
        let mut out = String::new();
        out.push_str(&object[..value_start]);
        out.push_str(literal);
        out.push_str(&object[value_end..]);
        return Ok(out);
    }
    // Absent: append before the closing brace, matching the sibling editor.
    let before_close = object[..object.len() - 1].trim_end_matches([' ', '\t', '\r', '\n']);
    let closing = &object[before_close.len()..];
    let mut out = String::new();
    out.push_str(before_close);
    out.push_str(",\n      \"pin\": ");
    out.push_str(literal);
    out.push_str(closing);
    Ok(out)
}

pub(super) fn json_array_bounds(contents: &str, field: &str) -> Option<(usize, usize)> {
    let field_start = json_field_name_position(contents, field)?;
    let colon = find_json_punct(contents, field_start + field.len() + 2, b':')?;
    let array_start = find_json_punct(contents, colon + 1, b'[')?;
    let array_end = matching_json_delimiter(contents, array_start, b'[', b']')?;
    Some((array_start, array_end))
}

pub(super) fn json_field_name_position(contents: &str, field: &str) -> Option<usize> {
    let needle = format!("\"{field}\"");
    let mut index = 0;

    loop {
        index = next_json_string_start(contents, index)?;
        let end = json_string_end(contents, index)?;
        if contents[index..end] == needle && contents[end..].trim_start().starts_with(':') {
            // Only match a key, not a string *value* that happens to equal the
            // field name: the next non-whitespace byte after the closing quote
            // must be `:` (bug-212).
            return Some(index);
        }
        index = end;
    }
}

pub(super) fn root_object_end(contents: &str) -> Option<usize> {
    let start = find_json_punct(contents, 0, b'{')?;
    matching_json_delimiter(contents, start, b'{', b'}')
}

pub(super) fn find_json_punct(contents: &str, start: usize, punct: u8) -> Option<usize> {
    let bytes = contents.as_bytes();
    let mut index = start;
    let mut in_string = false;
    let mut escaped = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == punct {
            return Some(index);
        } else if !byte.is_ascii_whitespace() {
            return None;
        }
        index += 1;
    }

    None
}

pub(super) fn matching_json_delimiter(
    contents: &str,
    start: usize,
    open: u8,
    close: u8,
) -> Option<usize> {
    let bytes = contents.as_bytes();
    let mut index = start;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == open {
            depth = depth.checked_add(1)?;
        } else if byte == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }

    None
}

pub(super) fn next_json_string_start(contents: &str, start: usize) -> Option<usize> {
    contents[start..].find('"').map(|offset| start + offset)
}

pub(super) fn json_string_end(contents: &str, start: usize) -> Option<usize> {
    let bytes = contents.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return None;
    }

    let mut index = start + 1;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}
