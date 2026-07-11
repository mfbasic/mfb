# bug-90 — LINK `FREE` block missing SYMBOL/ABI is silently dropped → native leak

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G1).
**Severity:** MED — silent wrong behavior; every call of the affected LINK
function leaks its native return buffer with zero diagnostics.
**Class:** footgun / silent-wrong-value.

## Finding

`src/ast/items.rs:881-883` — `parse_free_block` ends with:

```rust
let symbol = symbol?;
let (param_name, param_ctype) = param?;
let return_ctype = return_ctype?;
```

If a `FREE <slot> … END FREE` block omits its `SYMBOL` or `ABI` clause, the
`END FREE` terminator breaks the parse loop cleanly with those `Option`s still
`None`, and the `?` returns `None` **without any `report(...)`** — `had_error`
is never set. The caller (`parse_link_function`, items.rs:763-767) stores
`free: None`, which is indistinguishable from "no FREE declared" because
`FreeSpec` is legitimately optional on `LinkFunction`.

Contrast the sibling paths, which all diagnose:
- link FUNC missing SYMBOL/ABI → `MFB_PARSE_MISSING_NATIVE_SYMBOL`/`_ABI`
- malformed *present* FreeSpec → `NATIVE_FREE_INVALID`

Only the empty-FREE shape falls through silently.

## Trigger

```
LINK "sqlite3" AS l
  FUNC f() AS String
    SYMBOL "sqlite3_something" ABI "c"
    FREE return
    END FREE
  END FUNC
END LINK
```

Compiles clean; the declared deallocator vanishes; every call to `f` leaks the
native buffer.

## Fix sketch

In `parse_free_block`, when `END FREE` is reached with `symbol`/`param`/
`return_ctype` unset, emit `NATIVE_FREE_INVALID` (or the missing-symbol/abi
codes) instead of returning `None` silently.
