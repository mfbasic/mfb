# bug-851: a `RESOURCE`-declared (LINK) resource type is treated as plain data by the comparability and union-classification checks

Last updated: 2026-10-06
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Regression Test: tests/syntax/native/native-declared-resource-not-comparable-invalid; tests/rt-behavior/native/native-declared-resource-union-rt

Two verifier predicates decide "is this a resource?" with
`src/ir/verify/mod.rs:is_resource_name`, which consults only the **built-in** close
table (`builtin_resource_close_function`). A user-declared `RESOURCE T CLOSE BY
alias.func` is therefore classified as an ordinary user type, while the rest of the
compiler (`is_resource_or_resource_union` / `close_op_for` in
`src/ir/verify/link.rs`) knows it is a resource. Observable consequences, all
measured on macOS AArch64 with /tmp/hardening/mfb:

1. Two native resource handles compare with `=`/`<>` and the program runs
   (`a = a` → TRUE, `a = b` → FALSE). `fs::File` is correctly rejected with
   `2-203-0061 TYPE_REQUIRES_COMPARABLE`.
2. `collections::contains(xs, a)` on `List OF RES Tr` compiles and returns `TRUE`;
   the `fs::File` form is rejected (`TYPE_REQUIRES_COMPARABLE`).
3. `=` on a record whose field is `RES Tr` passes the verifier and dies in codegen:
   `error: native comparable comparison does not support type 'RES Tr' while lowering eval call io.print`
   (no file, no line, no rule code). The `fs::File` field form is rejected properly.
4. `UNION U2 { Tr, Tr2 }` (two declared resources) is classified all-data, then
   codegen dies: `error: native code union wrap member 'Tr' is not a record while lowering bind u AS U2 while lowering if`.
5. `UNION MD { Tr, D }` (declared resource + data record) is **not** reported as
   `TYPE_MIXED_RESOURCE_UNION`; it is accepted and `RES x AS MD = D[3]` compiles and
   runs (a data value bound with `RES`). Meanwhile `UNION St { Tr, fs::File }` — all
   resources — IS wrongly reported as `TYPE_MIXED_RESOURCE_UNION`.

Class: CODE BUG. Correct behavior: a declared `RESOURCE` type is a resource everywhere
— not comparable, rejected by `contains`/`=`/records-with-it comparison with
`TYPE_REQUIRES_COMPARABLE`, a union of only resources (declared or built-in) is a
resource union, and any data+resource mix is `TYPE_MIXED_RESOURCE_UNION`.

References:

- `src/docs/spec/language/15_resource-management.md` line 68: "A resource handle cannot be printed, compared, serialized, or captured by a lambda or ordinary closure."
- same, line 145: "A record carrying a resource is still **not comparable**"
- same, line 219: "A union whose every variant is a resource type is itself a resource — a *resource union*"
- same, line 242: "A union may **not mix** data and resource variants (`TYPE_MIXED_RESOURCE_UNION`)."
- same, line 341: "the helpers that require a comparable element (`find`, `contains`, `replace`) remain unavailable because handles are not comparable"
- `planning/completed/plan-111-B-front-end.md` line 206 already intended "`is_resource_name` folded into the typed `close_op_for`".

## Failing Reproduction

Prelude (shared; project.json needs
`"libraries": {"c":[{"os":"macos","type":"system","source":"libSystem.B.dylib"}]}`):

```
IMPORT io
IMPORT collections
RESOURCE Tr CLOSE BY c::release
LINK "c" AS c
  FUNC open(name AS String) AS RES Tr
    SYMBOL "strdup"
    ABI (name CString) AS p CPtr
    ERROR_ON p = NOTHING
    RETURN p
  END FUNC
  FUNC release(RES t AS Tr) AS Nothing
    SYMBOL "write"
    ABI (fd CInt32, t CPtr, n CInt64) AS status CInt64
    CONST fd = 1
    CONST n = 4
    SUCCESS_ON status = 4
  END FUNC
END LINK
```

(1)+(2):
```
FUNC main() AS Nothing
  RES a AS Tr = c::open("~A1\n")
  RES b AS Tr = c::open("~B1\n")
  io::print(toString(a = b))
  LET xs AS List OF RES Tr = [a]
  io::print(toString(collections::contains(xs, a)))
END FUNC
```
- Observed: builds; prints `FALSE`, `TRUE`, then the two closes.
- Expected: `2-203-0061 TYPE_REQUIRES_COMPARABLE` at both lines (as for `fs::File`).

(3):
```
TYPE H
  h AS RES Tr
END TYPE
FUNC main() AS Nothing
  RES a AS Tr = c::open("~A1\n")
  LET x = H[a]
  LET y = H[a]
  io::print(toString(x = y))
END FUNC
```
- Observed: `error: native comparable comparison does not support type 'RES Tr' while lowering eval call io.print`
- Expected: `TYPE_REQUIRES_COMPARABLE` with file:line (what `h AS RES fs::File` gives).

(5):
```
TYPE D
  n AS Integer
END TYPE
UNION MD
  Tr
  D
END UNION
FUNC main() AS Nothing
  RES x AS MD = D[3]
  io::print("bound")
END FUNC
```
- Observed: builds, prints `bound`, exit 0.
- Expected: `2-203-0087 TYPE_MIXED_RESOURCE_UNION`.

Also: `UNION St { Tr, fs::File }` → wrongly `TYPE_MIXED_RESOURCE_UNION`; `UNION U2 { Tr, Tr2 }`
with a `RES u AS U2 = ...` bind → codegen `union wrap member 'Tr' is not a record`.
Projects: /tmp/hardening/B03/resources/{d_cmp2,d_contains2,d_reccmp,u_drop,u_two,u_mixdata2}.

## Root Cause

- `src/ir/verify/values.rs:is_comparable_seen` returns false only for
  `is_resource_name(&name)`; a declared resource falls through to "Unknown user type —
  permissive" → comparable.
- `src/ir/verify/types.rs` union check counts `resource_variants` with
  `is_resource_name(&v.name)`, so declared resources count as data.
- `src/ir/verify/mod.rs:is_resource_name` consults only
  `crate::codegen::resource::builtin_resource_close_function`.
Immune: built-in resources (in that table); paths that use
`Verifier::is_resource_or_resource_union` / `close_op_for` (e.g. Map-key ownership,
`TYPE_RESOURCE_REQUIRES_RES`).

## Goal

- Both predicates use `self.close_op_for(..)` / `is_resource_or_resource_union`; the five
  repros give the expected diagnostics; `UNION { Tr, Tr2 }` and `UNION { Tr, fs::File }`
  build as resource unions with tag-dispatched drop.

### Non-goals (must NOT change)

- Built-in resource behavior; comparability of ordinary records/enums.

## Blast Radius

- `src/ir/verify/values.rs:is_comparable_seen` — fixed by this bug.
- `src/ir/verify/types.rs` union `TYPE_MIXED_RESOURCE_UNION` count — fixed by this bug.
- `grep -rn "is_resource_name(" src/ir` finds only these two callers — no other site.

## Validation Plan

- Verifier unit tests for each repro; one rt fixture for a resource union of two
  declared resources (drop closes active variant once).

## Summary

One builtin-only predicate; the fix is mechanical, the audit of remaining
`is_resource_name` callers is where the risk is.

## Resolution

**Verifier.** `is_comparable_seen` and the union classifier in `types.rs` now ask
`TypeEnv::is_resource_base` (a declared/imported resource or a built-in one, the
predicate every other resource rule uses since bug-975). Neither asks the built-in
close table any more. `is_resource_name` had only those two callers
(`grep -rn is_resource_name src`) and is deleted.

**Codegen.** Once `{ Tr, Tr2 }` and `{ Tr, fs::File }` are resource unions, every
resource-union site has to agree. The doc's case 4 failed in codegen even before
the verifier change, so these sites were in scope. Each site asked the built-in
`is_resource_type` alone and now asks the `TypeModel`'s `is_resource_nominal`,
which includes `resource_names`:

- `builder_values.rs`: `UnionWrap` (whether the variant is a bare handle, and the
  payload word count) and `UnionExtract` (load the handle at +8).
- `builder_collection_layout.rs`: `inline_collection_payload_size`'s union
  sizing, `union_is_data`, and `type_contains_resource`.
- `builder_resource_cleanup.rs:resource_union_variants`: the tag-dispatched drop
  table. The declared close op resolves through `resource_cleanup_symbol`, which
  already handled user resources (bug-374).
- `builder_arena_transfer.rs`: the resource-union variant copy.

The module-level passes have no `TypeModel`. They now share a new
`validation::module_resource_names(module)`. `TypeModel`'s own `resource_names` is
built from it too: the collection code was moved there, not copied. These passes
are the union close-import scan in `plan/symbols.rs` (both functions),
`module_analysis.rs:module_drops_resource_union_close`, and the NIR backstop
`validate/mod.rs:validate_resource_rules`, which would otherwise have refused
`{ Tr, fs::File }` as mixed.

Measured (macOS AArch64, the doc's repros built with the fixed compiler):

- (1) `a = b`, (2) `contains(xs, a)`, (3) `x = y` on `H { h AS RES Tr }`: each is now
  `2-203-0061 TYPE_REQUIRES_COMPARABLE` with file and line. The messages are the
  same as for `fs::File`, apart from the type name.
- (5) `UNION MD { Tr, D }`: `2-203-0087 TYPE_MIXED_RESOURCE_UNION`.
- (4) `UNION Duo { Tr, Tr2 }` builds and runs. Two bindings close B2 then A1
  (reverse declaration order), once each. `UNION Mixed { Tr, fs::File }` builds; its
  `Tr` variant closes through `c::release` and its `fs::File` variant reads
  `hello` and closes.
- Before (base): (1)/(2) printed `FALSE`/`TRUE`; (3) gave
  `error: native comparable comparison does not support type 'RES Tr'`; (4) gave
  `union wrap member 'Tr' is not a record`; (5) printed `bound`; `{ Tr, fs::File }`
  gave `TYPE_MIXED_RESOURCE_UNION`.

Checks: `cargo test --bin mfb verify` (516 passed), `cargo test --test
no_type_strings` (7 passed; the moved `ParameterType::declared` stays within the
ratchet). `scripts/test-accept.sh` over the 193 native/resources/union fixtures gave
53 mismatches, all bug-976's predicted class: `.ast` plus the 10 `build.log` whose
sources dropped a parameter `RES`. None of them gained or lost a
`TYPE_REQUIRES_COMPARABLE`/`TYPE_MIXED_RESOURCE_UNION` line (`diff … | grep -c` = 0).

## Pre-existing, not changed here

A LINK-declared resource's record is never freed at drop (`--debug`: 2 blocks, 128 B
per bind/drop of a plain `RES t AS Tr`, the same with main at 468ff0f94).
`resource_record_freed_at_drop` deliberately keeps native `LINK` records as
tombstones ("producers … this analysis has not audited"). The new union fixture
inherits the same per-variant bytes.

## Golden impact

None beyond the two new fixtures. `scripts/artifact-gate.sh … all` gave 62 diffs, exactly bug-976's base set
(61 `.ast` + `app-mouse-surface` `.nir`), with no `.ncode`/`.ncodesum`/`.nplan` diff.
