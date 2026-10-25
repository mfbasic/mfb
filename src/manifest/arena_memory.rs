//! plan-179-A: the `options.maxArenaMemory*` keys — the process-wide arena memory
//! cap. This module reads and validates them; the startup probe and the allocator
//! accounting that act on them live in codegen (plan-179-B, plan-179-C).
//!
//! The reader and the validator share one predicate per key, so the reader never
//! returns a value the validator would reject: an invalid value reaching the
//! reader unvalidated reads as the default.

use std::collections::HashMap;
use std::path::Path;
use tinyjson::JsonValue;

use super::field_position;
use crate::ir::{ArenaMemoryCheck, ArenaMemoryLimits};
use crate::rules;

/// The default `maxArenaMemoryPercent`: half of physical RAM.
pub(crate) const DEFAULT_ARENA_MEMORY_PERCENT: u64 = 50;
/// The default `maxArenaMemoryMiB`: `-1`, unset.
pub(crate) const DEFAULT_ARENA_MEMORY_MIB: i64 = -1;
/// The default `maxArenaMemoryCheck`, `"halt"`.
pub(crate) const DEFAULT_ARENA_MEMORY_CHECK: ArenaMemoryCheck = ArenaMemoryCheck::Halt;
/// The largest `maxArenaMemoryMiB`: the largest MiB count whose byte count
/// (`× 1048576`) fits in u64, `2^44 - 1`. Exact in f64, so the comparison against
/// a JSON number is exact.
pub(crate) const MAX_ARENA_MEMORY_MIB: u64 = u64::MAX >> 20;

const KEY_DISABLE: &str = "maxArenaMemoryDisable";
const KEY_PERCENT: &str = "maxArenaMemoryPercent";
const KEY_MIB: &str = "maxArenaMemoryMiB";
const KEY_CHECK: &str = "maxArenaMemoryCheck";

fn option<'a>(manifest: &'a HashMap<String, JsonValue>, key: &str) -> Option<&'a JsonValue> {
    manifest
        .get("options")
        .and_then(|options| options.get::<HashMap<String, JsonValue>>())
        .and_then(|options| options.get(key))
}

fn valid_disable(value: &JsonValue) -> Option<bool> {
    value.get::<bool>().copied()
}

fn valid_percent(value: &JsonValue) -> Option<u64> {
    value
        .get::<f64>()
        .filter(|n| n.fract() == 0.0 && (1.0..=100.0).contains(*n))
        .map(|n| *n as u64)
}

fn valid_mib(value: &JsonValue) -> Option<i64> {
    value
        .get::<f64>()
        .filter(|n| {
            n.fract() == 0.0 && (**n == -1.0 || (**n >= 1.0 && **n <= MAX_ARENA_MEMORY_MIB as f64))
        })
        .map(|n| *n as i64)
}

fn valid_check(value: &JsonValue) -> Option<ArenaMemoryCheck> {
    match value.get::<String>().map(String::as_str) {
        Some("warn") => Some(ArenaMemoryCheck::Warn),
        Some("halt") => Some(ArenaMemoryCheck::Halt),
        _ => None,
    }
}

/// Read the four `options.maxArenaMemory*` keys from a validated manifest. An
/// absent key, or one the validator would reject, reads as its default.
///
/// **This is the CONSUMING project's setting**, like `options.maxBuffer`: the cap
/// covers the whole process, so a package's own value is never encoded into its
/// `.mfp`.
pub(crate) fn arena_memory_limits(manifest: &HashMap<String, JsonValue>) -> ArenaMemoryLimits {
    let defaults = ArenaMemoryLimits::default();
    ArenaMemoryLimits {
        disable: option(manifest, KEY_DISABLE)
            .and_then(valid_disable)
            .unwrap_or(defaults.disable),
        percent: option(manifest, KEY_PERCENT)
            .and_then(valid_percent)
            .unwrap_or(defaults.percent),
        mib: option(manifest, KEY_MIB)
            .and_then(valid_mib)
            .unwrap_or(defaults.mib),
        check: option(manifest, KEY_CHECK)
            .and_then(valid_check)
            .unwrap_or(defaults.check),
    }
}

/// Validate the optional `options.maxArenaMemory*` keys. Each invalid value is a
/// `PROJECT_JSON_FIELD_TYPE` error naming its key. With `maxArenaMemoryDisable:
/// true`, each other key that is present gets a `PROJECT_JSON_ARENA_MEMORY_IGNORED`
/// warning; the build continues.
pub(super) fn validate_arena_memory(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool {
    let mut valid = true;
    let report = |key: &str, rule: &str, message: &str| {
        let (line, column) = field_position(contents, key);
        rules::show_diagnostic(
            rule,
            message,
            project_path,
            line,
            column,
            column + key.len() + 2,
        );
    };
    let mut disabled = false;
    if let Some(value) = option(manifest, KEY_DISABLE) {
        match valid_disable(value) {
            Some(flag) => disabled = flag,
            None => {
                report(
                    KEY_DISABLE,
                    "PROJECT_JSON_FIELD_TYPE",
                    "Field `maxArenaMemoryDisable` must be `true` or `false`, for example `\"maxArenaMemoryDisable\": false`.",
                );
                valid = false;
            }
        }
    }
    if option(manifest, KEY_PERCENT).is_some_and(|value| valid_percent(value).is_none()) {
        report(
            KEY_PERCENT,
            "PROJECT_JSON_FIELD_TYPE",
            "Field `maxArenaMemoryPercent` must be a whole number from 1 to 100, for example `\"maxArenaMemoryPercent\": 50`.",
        );
        valid = false;
    }
    if option(manifest, KEY_MIB).is_some_and(|value| valid_mib(value).is_none()) {
        report(
            KEY_MIB,
            "PROJECT_JSON_FIELD_TYPE",
            &format!(
                "Field `maxArenaMemoryMiB` must be -1 (unset) or a whole number of MiB from 1 to {MAX_ARENA_MEMORY_MIB}, for example `\"maxArenaMemoryMiB\": 4096`."
            ),
        );
        valid = false;
    }
    if option(manifest, KEY_CHECK).is_some_and(|value| valid_check(value).is_none()) {
        report(
            KEY_CHECK,
            "PROJECT_JSON_FIELD_TYPE",
            "Field `maxArenaMemoryCheck` must be `\"warn\"` or `\"halt\"`, for example `\"maxArenaMemoryCheck\": \"halt\"`.",
        );
        valid = false;
    }
    if disabled {
        for key in [KEY_PERCENT, KEY_MIB, KEY_CHECK] {
            if option(manifest, key).is_some() {
                report(
                    key,
                    "PROJECT_JSON_ARENA_MEMORY_IGNORED",
                    &format!("`maxArenaMemoryDisable` is true, so `{key}` is ignored."),
                );
            }
        }
    }
    valid
}

#[cfg(test)]
#[path = "tests/arena_memory.rs"]
mod tests;
