//! `__json_revive` — shared private helper for the `json` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-120-E: the post-order walk behind `json::parse(text, reviver)`.
'
' Runs AFTER `__json_parse` has built the whole tree, rather than interleaving
' revival into the parser. That keeps every parse helper untouched and is not
' observably different: the reviver is specified to see a fully-parsed subtree,
' so there is nothing it could witness mid-parse that it is allowed to act on.
'
' Order is JavaScript's, confirmed against Node v24.12.0: children first, then
' the container itself, so a reviver always receives an ALREADY-revived subtree.
' Keys are the member key for an object, the index rendered as a decimal string
' for an array element, and "" for the root -- which is called last and is the
' only call that sees the whole document.
'
' Objects are rebuilt member by member in iteration order, preserving
' plan-120-C's document-order contract. Duplicate keys were already collapsed
' last-wins by the parser, so the reviver sees each key once, as in JavaScript.
'
' No deletion: MFBASIC has no `undefined`, so whatever the reviver returns is
' stored verbatim. Returning `JsonNull[NOTHING]` stores a JSON null rather than
' dropping the member -- the one documented divergence from JavaScript.
FUNC __json_revive(key AS String, value AS Json, reviver AS FUNC(String, Json) AS Json) AS Json
  MATCH value
    CASE JsonArr(arrValue)
      MUT items AS List OF Json = []
      MUT index AS Integer = 0
      FOR EACH item IN arrValue.items
        LET revivedItem AS Json = __json_revive(toString(index), item, reviver)
        items = collections::append(items, revivedItem)
        index = index + 1
      NEXT
      LET rebuiltArr AS Json = JsonArr[items]
      RETURN reviver(key, rebuiltArr)
    CASE JsonObj(objValue)
      MUT fields AS Map OF String TO Json = Map OF String TO Json {}
      FOR EACH entry IN objValue.fields
        LET revivedValue AS Json = __json_revive(entry.key, entry.value, reviver)
        fields = collections::set(fields, entry.key, revivedValue)
      NEXT
      LET rebuiltObj AS Json = JsonObj[fields]
      RETURN reviver(key, rebuiltObj)
    CASE ELSE
      RETURN reviver(key, value)
  END MATCH
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("json_revive", BODY));
}
