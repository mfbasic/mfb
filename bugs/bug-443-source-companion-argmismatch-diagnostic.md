# bug-443: source-companion call with wrong args reports "not a top-level function" (and leaks the `#` sigil)

STATUS: OPEN (pre-existing; unrelated to bug-441 — discovered while regenerating
acceptance goldens for the resource package-scoping cutover).

## The single correct behavior a fix produces

Calling a **source-companion** builtin (the `.mfb`-implemented packages:
`encoding`, `json`, `regex`, `collections`) with the wrong argument count or
types produces the normal argument diagnostic — `TYPE_CALL_ARITY_MISMATCH` /
`TYPE_CALL_ARGUMENT_MISMATCH` naming the public call (`encoding.utf8Decode`) —
exactly as a native builtin does. It never reports `SYMBOL_UNKNOWN_IDENTIFIER`,
and never prints the compiler-internal `#` sigil to the user.

## Failing reproduction

`tests/syntax/encoding/func_encoding_utf8Decode_invalid` (a `mfb build -ast -ir`
compile-fail fixture). Source calls `encoding::utf8Decode()` (arity 0, needs 1)
and `encoding::utf8Decode("notbytes")` (String, needs `List OF Byte`).

Committed golden (intended):
```
error[2-203-0022 TYPE_CALL_ARITY_MISMATCH]: ... Call to `encoding.utf8Decode` has 0 argument(s), expected 1.
error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: ... argument type(s) (String), expected List OF Byte or List OF Integer.
```

Current binary (regression):
```
error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: identifier could not be resolved
               Callable `#encoding_utf8Decode` is not a top-level function.
```

Reproduces on this worktree's base commit `8a0bd49c2` too, i.e. it predates the
net/tls/audio resource work — this is NOT a resource regression.

Same shape (confirmed via the acceptance harness) for:
`tests/syntax/json/func_json_{get,getOr,stringify}_invalid`,
`tests/syntax/regex/func_regex_{find,findAll,match,replace}_invalid`,
`tests/syntax/encoding/func_encoding_utf8Encode{,_ambiguous}_invalid`,
`tests/syntax/collections/func_collection_transform_invalid`.

## Root cause (hypothesis — to confirm)

The source-companion packages internalize their private helpers `__pkg_name` ->
`#pkg_name` via `src/internal_name.rs` (`INTERNAL_SIGIL = '#'`, recent
registry-migration infrastructure). On the argument-mismatch path a public call
to one of these packages appears to be rewritten to its internalized
implementation name (`#encoding_utf8Decode`) and re-run through
`Resolver::resolve_callable` (`src/resolver/resolution.rs:1211`), which — for a
bare, non-visible callee — reports `SYMBOL_UNKNOWN_IDENTIFIER` "is not a
top-level function" (line 1234-1239) instead of letting the descriptor emit the
arity/argument diagnostic. That diagnostic also interpolates the **raw** callee,
so the untypeable `#` sigil leaks; per `internal_name::display_name` it should
never reach a user message.

Two distinct defects likely: (1) the arg-mismatch path routes a public
source-companion call to the internal name before the argument diagnostic fires;
(2) the resolver diagnostic at `resolution.rs:1236` does not pass the callee
through `internal_name::display_name`.

## Non-goals

- Do NOT re-baseline these goldens to the `SYMBOL_UNKNOWN_IDENTIFIER` output
  (that masks the bug — the acceptance goldens were deliberately left at the
  committed/intended state during the bug-441 cutover).
- No change to the resource package-scoping.

## Blast radius

The `encoding`/`json`/`regex`/`collections` source-companion packages' arg-error
paths. Native builtins are unaffected (they already emit the correct
diagnostic). This sits in the same `internal_name` / registry-migration subsystem
as the in-flight `datetime`/enum migration work, so coordinate before fixing.

## Verified root cause (2026-08-15) — CONFIRMED, and it is MULTI-LAYER

The §Root cause hypothesis is confirmed and the pipeline mechanics are pinned down,
but the defect is **not one fix** — the migrations dropped diagnostic coverage in
several places, and a naive fix at any one layer regresses another. Evidence:

**Pipeline.** `build_project` (`src/cli/build/mod.rs`) runs the resolver TWICE:
`resolve_project` (L328, original AST) then, after monomorph (L333), `resolve_augmented`
(L338, on the monomorphized `concrete_ast`). The `SYMBOL_UNKNOWN_IDENTIFIER` fires in
the **second** pass and aborts (via `?`) before syntaxcheck (L388) or ir::verify (L424)
run.

**Layer 1 — monomorph mangles overloaded/source-generic calls unconditionally.**
`src/monomorph/lower.rs` (~L1219-1254) rewrites `encoding.utf8Decode`→`#encoding_utf8Decode`
and `collections.<m>`→`#collections_<m>`, then on a wrong-arg call every
instantiate/resolve_overload returns `None` and the `else { callee.clone() }` leaves the
unresolvable bare `#name`, which the 2nd resolver pass rejects. Falling back to the
PUBLIC callee here fixes ENCODING (its syntaxcheck table checker validates the public
`encoding.utf8Decode`) — but for COLLECTIONS it merely exposes Layer 2/3 and the invalid
program then compiles **exit 0** (worse than the spurious error). So Layer 1 cannot land
alone.

**Layer 2 — syntaxcheck routing is source-generic-blind.** `check_builtin_call`
(`src/syntaxcheck/builtins.rs:244`) routes to `check_collections_builtin_call` on
`registry().owning_package(callee) == Some("collections")`. `owning_package` keys on
`resolve_func`, which returns `None` for a **source-generic** member (`sort`/`transform`),
so those fall through to the unchecked generic tail. The migration replaced the old
`is_collections_call` (which caught source-generics) with generic-blind `owning_package`.

**Layer 3 — the package validators lost per-member cases.** Even routed,
`check_collections_builtin_call` has NO `transform` arm (transform was migrated in
plan-96, `func_transform.rs`; its arg validation was dropped). The `json`/`regex` table
path (`check_table_builtin_call`) still emits ARITY but drops the ARGUMENT-TYPE check for
a well-arity'd call (e.g. `json::get("bad", ["x"])` with first param `Named("Json")` is
silently accepted) — a distinct gap from the encoding/collections mangle, present with or
without Layer 1.

**Correct fix shape (do all together, coordinate goldens):** (a) monomorph falls back to
the public callee on overload-resolution failure; (b) `check_builtin_call` routes
source-generic members to their package checker (`owning_package(..)==Some(pkg) ||
is_source_generic_member(callee)`); (c) restore the dropped per-member argument
validation in `check_collections_builtin_call` (transform, …) and close the json/regex
argument-type gap in the table path; (d) also render resolver diagnostics through
`internal_name::display_name` as defense-in-depth. Verify by running the 11 `*_invalid`
fixtures against their committed goldens AND a full `cargo test` (the monomorph fallback
touches every failed-overload call, so guard against silent-accept regressions).

A partial attempt (Layers 1+2 only) was implemented and reverted 2026-08-15 because it
regressed `collections/func_collection_transform_invalid` to a silent exit-0 compile.

## Notes

Found during `/fix-bug 441`. The bug-441 resource cutover is complete and green;
these fixtures were reverted to their committed goldens (not regenerated) so this
regression stays visible rather than baked in.
