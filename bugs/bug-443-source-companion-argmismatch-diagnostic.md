# bug-443: source-companion call with wrong args reports "not a top-level function" (and leaks the `#` sigil)

STATUS: FIXED 2026-08-15. All 12 `*_invalid` fixtures (encoding/json/regex/collections)
now emit the correct `TYPE_CALL_ARITY_MISMATCH` / `TYPE_CALL_ARGUMENT_MISMATCH`; no
`SYMBOL_UNKNOWN_IDENTIFIER`, no `#`-sigil leak. Fixed across four layers (see below);
full `cargo test` (3812) + full acceptance green. Two under-coercion bugs surfaced and
were fixed as a bonus (json.getOr union-wrap, process arg `List OF Byte` typing).

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

### Status: Layers 1+2 FIXED (2026-08-15); Layer 3 remains

- **Layer 1 (monomorph public fallback) — DONE.** `src/monomorph/lower.rs` now keeps the
  public callee on overload-resolution failure. Fixes the SYMBOL_UNKNOWN class:
  `encoding/utf8Decode` and `utf8Encode` now emit the correct
  `TYPE_CALL_ARITY_MISMATCH`/`TYPE_CALL_ARGUMENT_MISMATCH`.
- **Layer 2 (source-generic routing) — DONE.** `check_builtin_call`
  (`src/syntaxcheck/builtins.rs`) now routes source-generic collections members
  (`owning_package==collections || is_source_generic_member`) to the collections checker,
  so a wrong-arg `collections::sort()` is validated (`TYPE_CALL_ARGUMENT_MISMATCH`) rather
  than silently compiling. (An earlier revert wrongly attributed a pre-existing
  `transform` silent-accept to Layer 1; it is Layer 3, see below — Layers 1+2 introduce no
  regression, verified by full `cargo test` + acceptance.)

- **Layer 3 (nominal argument-TYPE validation) — FIXED via a strict/lenient matcher split.**
  The clean registry's overload matcher `leaf_matches` was a *coarser* filter than the old
  per-package resolvers: `pattern == concrete || !(pattern.is_scalar() && concrete.is_scalar())`
  — a **nominal** pattern (`Named("Json")`) is non-scalar, so it accepted ANY scalar argument,
  and `json::get("bad", ["x"])` passed with no `TYPE_CALL_ARGUMENT_MISMATCH`. A blanket tighten
  regressed *valid*-program overload selection (`csv::stringify`'s nested-list arg degraded to
  `List OF Unknown`), because the matcher is used both to VALIDATE args and to DISPATCH/infer.
  Fix (option 1): thread a `strict` flag through `unify`/`leaf_matches` and split
  `RegistryFunction::select` into **`resolve`** (strict — a scalar never satisfies a nominal
  parameter) and **`dispatch`** (lenient — coarse, unchanged). `resolve_call` /
  `resolve_call_return_type` carry `strict`; syntaxcheck's argument validation passes `true`,
  every inference/codegen caller (`ir/lower`, `target/shared/code`, `ir/verify`) passes `false`.
  Result: `json`/`regex`/`process` now correctly reject nominal-vs-scalar mismatches; `csv`/`json`
  codegen `.ir` byte-identical (verified full acceptance). `process/*_invalid` goldens gained the
  now-correct `expected process.Process` rejection (regenerated, purely additive).

### Layer 4 — remaining pieces, all FIXED (2026-08-15)

- **json/regex `*_invalid` "expected …" WORDING — FIXED by decoupling the diagnostic
  from coercion.** The generic `expected_arguments` rendered only parameter 0. The blocker
  was that `expected_arguments` was NOT diagnostic-only: `builtins::argument_types` (the IR
  lowering per-argument coercion table) *parsed the `expected_arguments` string positionally*
  and skipped any string containing `[`. So widening the diagnostic changed codegen
  (`json.getOr`'s `Json` arg wrapped; `csv.stringify`'s bracketed render degraded its grid to
  `List OF Unknown`). Fix: a new **machine** table `registry::argument_types` (positional
  parameter types from the descriptor, `None` for overload/generic members) now drives
  coercion; `builtins::argument_types` consults it before the old string path; and
  `expected_arguments` is now diagnostic-only and full-renders the signature
  (`Json, List OF String`; `regex.find` → `String, String[, Integer]`). Widening the wording
  no longer touches codegen. Bonus: the decoupling *fixed* the truncated coercion table —
  `json.getOr`'s union arg now correctly `unionWrap`s and a `process` arg types as
  `List OF Byte` instead of `List OF Unknown` (both regenerated, native `.ncodesum`
  unchanged).
- **`collections/transform` — FIXED.** `unify` no longer binds a type variable to `Nothing`
  in STRICT (validation) mode, so a `SUB` (`FUNC(T) AS Nothing`) no longer satisfies the
  value-returning callback `FUNC(T) AS U`. The lenient dispatch path still binds it, so
  `Nothing`-returning callbacks (`forEach`) keep lowering.
- **`encoding/utf8Encode_ambiguous` — FIXED.** The monomorph overload-ambiguity diagnostic
  threads the PUBLIC callee (`resolve_overload`'s new `display` param), so it names
  `encoding.utf8Encode`, not the mangled `#encoding_utf8Encode`. The count stays (it matches
  the two user-function ambiguity fixtures `func_return_overload_ambiguous`/`bug107`); this
  fixture's golden regenerated to the consistent `matches 2 overloads`.
- **Defense-in-depth (not done, no longer reachable):** the `internal_name::display_name`
  guard at `resolution.rs:1236` is moot now that no source-companion call reaches the 2nd
  resolver pass mangled.

## Notes

Found during `/fix-bug 441`. The bug-441 resource cutover is complete and green;
these fixtures were reverted to their committed goldens (not regenerated) so this
regression stays visible rather than baked in.
