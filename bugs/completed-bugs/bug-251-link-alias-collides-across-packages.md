# bug-251: two imported packages sharing a `LINK` alias + function name silently route to one library

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Fixed
Regression Test: `tests/rt-behavior/native/native-link-alias-collision-rt` — two
imported packages share the `LINK` alias `fooLink` + function name `raw`, each
bound to a different `sqlite3` symbol; correct per-package routing exits **73**,
any misroute exits **70** (the pre-fix observed value).

Fix (Phase 2, `src/ir/package.rs:prefix_package_symbols`): qualify each imported
package's `LINK` function `alias` with the package's content-addressed identity
prefix — the same prefix regular functions already receive — rewriting the
wrapper-body routing references (`alias.func`), the CSTRUCT-table `alias` join,
and the re-export alias targets in the same pass. The merge dedup key
(`alias, name`), the `link_thunk_symbol`, and the routing import name all inherit
the package-distinct identity in lockstep, so two packages that independently
choose the same alias + function name no longer collide, while a diamond import
(same package, same prefix) still collapses to one thunk. The `.mfp` trailer is
untouched: `prefix_package_symbols` runs only in the executable-merge path, never
in `write_package`.

A native `LINK` function's routing identity is its **package-internal**
`alias.func` pair, but `merge_package` de-duplicates link functions in a
**project-global** namespace. So when a consumer imports two binding packages that
each declare a `LINK` block with the same alias *and* a function of the same name
— `LINK "fooliba" AS fooLink / FUNC raw` and `LINK "foolibb" AS fooLink / FUNC
raw` — the second package's link function is dropped as a duplicate. Every call
into the second package's wrapper is then routed to the **first package's thunk**,
which `dlopen`s the **first package's library** and calls its symbol.

The correct behavior a fix produces: **each package's `LINK` functions resolve to
that package's own library and symbol, regardless of what any other imported
package names its `LINK` alias or functions.** Two packages that never see each
other's source must not be able to collide.

This is silent and dangerous. There is no diagnostic, no link error, and no
runtime error — the call succeeds and returns a plausible value computed by the
**wrong native library**. Neither package author can prevent it (they cannot see
each other's alias names), and the consumer cannot fix it either; the alias is
internal to each package. `fooLink`, `link`, `lib`, and `sql` are exactly the kind
of names two authors pick independently.

References:

- `./mfb spec package native-bindings` — the `LINK` metadata model
- `./mfb spec language native-libraries` — the loading model and `_mfb_linker_init`
- Found while verifying plan-46-D §4.5's vendored-library collision behavior. The
  vendor prefix (`<declaring-unit>-<source>`) works correctly; this bug is
  upstream of it and unrelated — it misroutes the *thunk*, not the file.

## Failing Reproduction

Two binding packages whose `LINK` blocks share the alias `fooLink` and the
function name `raw`, each binding a **different** native library. A consumer
imports both.

```sh
# Two different native libraries, deliberately distinguishable:
printf 'int foo_op(int v, int *out) { *out = v * 7; return 0; }\n' > a.c
printf 'int foo_op(int v, int *out) { *out = v * 3; return 0; }\n' > b.c
cc -dynamiclib -o a/vendor/libfoo.dylib a.c     # pkga: multiplies by 7
cc -dynamiclib -o b/vendor/libbar.dylib b.c     # pkgb: multiplies by 3
```

```basic
' pkga/src/lib.mfb
LINK "fooliba" AS fooLink            ' <-- same alias
  FUNC raw(value AS Integer) AS Integer   ' <-- same function name
    SYMBOL "foo_op"
    ABI (value CInt32, return OUT CInt32) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK
EXPORT FUNC op(value AS Integer) AS Integer
  RETURN fooLink::raw(value)
END FUNC

' pkgb/src/lib.mfb — identical except LINK "foolibb"
```

```basic
' consumer: pkga::op(1) must be 7, pkgb::op(1) must be 3 -> exit 73
FUNC main AS Integer
  LET a AS Integer = pkga::op(1) TRAP(e)
    RECOVER 0
  END TRAP
  LET b AS Integer = pkgb::op(1) TRAP(e)
    RECOVER 0
  END TRAP
  RETURN a * 10 + b
END FUNC
```

- Observed: **exit 77** — both calls returned `7`, i.e. both bindings loaded and
  called **pkga's** library. No diagnostic of any kind.
- Expected: **exit 73** — `pkga::op(1) = 7` from pkga's library, `pkgb::op(1) = 3`
  from pkgb's.

Contrast cases that work correctly today (these bound the bug and become
regression guards):

- **Distinct aliases** (`AS fooLinkA` / `AS fooLinkB`), everything else identical
  → **exit 73**. Verified. This is the single-variable proof: renaming one alias
  is the only change.
- **Same alias, different function names** → no `(alias, name)` collision, so both
  survive.
- **A diamond import** (the same package reached twice) → the dedup is correct and
  required here; this is the case it was written for.
- Regular (non-`LINK`) exported functions with the same name in two packages →
  unaffected: they are package-qualified before merge (see Root Cause).

| Environment | Details | Result |
| --- | --- | --- |
| macOS aarch64 | console build, two vendored dylibs | fails ✗ (77, expected 73) |

Platform-independent by inspection: the defect is in target-neutral IR merging,
above any backend.

## Root Cause

`src/ir/package.rs:merge_package` de-duplicates link functions by
`(alias, name)`:

```rust
// Native `LINK` functions keep their package-internal `alias.func` routing
// names (wrapper bodies reference them unprefixed), de-duplicated across
// diamond imports (plan-linker.md §12).
for link in package.link_functions {
    if !project.link_functions.iter()
        .any(|existing| existing.alias == link.alias && existing.name == link.name)
    {
        project.link_functions.push(link);
    }
}
```

The comment states the mechanism and, unintentionally, the bug: the routing name
is **package-internal** (`fooLink.raw`), because wrapper bodies reference it
unprefixed — but after merge every package's link functions share **one global
`link_functions` vector**. The dedup cannot tell "the same package imported twice"
(which it must collapse) from "two different packages that chose the same alias"
(which it must keep). It collapses both.

Why the contrast cases are immune:

- **Regular functions** are deduped by their *already-namespaced* name — the
  merge's own doc comment says so (`src/ir/package.rs:86-87`), and
  `prefix_package_symbols` (`src/ir/package.rs:10`) rewrites them to
  `<id>.<package>.<name>` before merge. **It never touches `link_functions`**
  (verified: `link_functions` appears in that file only at the merge site,
  lines 119-125). That asymmetry is the bug.
- **Diamond imports** are the intended case: identical `(alias, name)` from the
  same package genuinely is a duplicate.

Downstream, `emit_link_support` (`src/target/shared/code/link_thunk.rs`) builds
its library index from the merged `link_functions` and emits one thunk per
surviving entry; the dropped function has no thunk, so the consumer's call to
`pkgb::op` resolves to `fooLink.raw` — pkga's.

## Goal

- A consumer importing two packages that share a `LINK` alias and function name
  builds, and each package's wrapper calls **its own** library/symbol
  (the reproduction exits **73**).
- The diamond-import case still collapses to one entry (no duplicate thunk, no
  duplicate `dlopen`).

### Non-goals (must NOT change)

- **The `.mfp` format and the `(alias, name, library, symbol, …)` IR trailer.**
  This is a merge/routing defect; the on-disk interface record is correct.
- **Diamond-import de-duplication.** Collapsing the same package imported twice
  is required behavior, not collateral.
- **Wrapper source syntax.** `fooLink::raw` inside a binding's own body must keep
  working unqualified; authors must not have to rename aliases to be safe. A
  "fix" that merely *documents* "don't reuse common alias names", or that emits a
  diagnostic telling the consumer to rename something they cannot reach, is
  explicitly forbidden — neither author can see the other's manifest, so neither
  can act on it.
- **Tempting wrong fix:** widening the dedup key to `(alias, name, library)`.
  That makes *this* reproduction pass (the two libraries differ) while leaving the
  real hazard: two packages sharing an alias, a function name, **and** a logical
  library name still collide and still silently misroute. The key must be the
  *declaring package*, not the payload.

## Blast Radius

Found by searching `src/ir/package.rs` for every merge-time identity decision,
not from memory:

- `src/ir/package.rs:merge_package` (link functions, lines 119-125) — **the bug**;
  fixed here.
- `src/ir/package.rs:merge_package` (link aliases, lines 128-140) — **already
  correct, and the model for the fix**: it qualifies with
  `format!("{}.{}", package.name, alias_name)` before deduping, for exactly this
  reason (plan-link-update.md §5a).
- `src/ir/package.rs:merge_package` (functions, lines 107-115) — unaffected: keyed
  by the already-namespaced name (`prefix_package_symbols`).
- `src/ir/package.rs:merge_package` (types, lines 89-95) — dedups types by **bare
  name**. Same shape of hazard, but out of scope: type identity across packages is
  a separate design question (the doc comment records the bare-name choice as
  deliberate), and no misrouting was observed. Worth its own audit.
- `src/target/shared/code/link_thunk.rs:emit_link_support` — a consumer of the
  merged list, not a cause. It will index whatever the merge produces; no change
  expected beyond one thunk per surviving function.
- `src/target/shared/code/link_locator.rs:LibraryTables::resolve` (plan-46-C) —
  unaffected: it resolves per *logical library name* per declaring unit and was
  never keyed on the alias. Confirmed by the reproduction: the correct
  `pkga-libfoo.dylib` / `pkgb-libbar.dylib` files were both copied and both
  RPATH-resolvable; only the *thunk* was misrouted.

## Fix Design

Mirror what the link-**alias** merge already does one block below: make the
declaring package part of the link function's merged identity.

The risk concentrates in the fact that `alias` is load-bearing in three places at
once — it is the dedup key, the wrapper body's reference (`fooLink::raw`), and
part of the emitted thunk symbol (`link_thunk_symbol(alias, name)`). Qualifying it
naively at merge time would break the wrapper bodies that already reference it
unprefixed. So the fix must either:

1. **Qualify the alias during `prefix_package_symbols`** (before merge), rewriting
   the wrapper bodies' references in the same pass — symmetric with how regular
   functions are handled, and the reason that pass exists; or
2. **Add a `package` field to `IrLinkFunction`** and dedup on
   `(package, alias, name)`, leaving `alias` untouched for body references and
   folding the package into `link_thunk_symbol` so two packages' thunks get
   distinct symbols.

(2) is the smaller change and keeps the wrapper-body contract literally unchanged;
(1) is more uniform with the rest of the merge. Prefer (2) unless the thunk-symbol
change proves to churn more than expected.

Either way the thunk symbol must become package-distinct, or two thunks collide at
the symbol level instead of the IR level — the same bug one layer down.

Expected output shift: binding-package `.mfp`/IR goldens do not change (the
trailer is untouched). Executable codegen goldens change **only** for a fixture
importing two colliding packages — none exists today, so the churn should be
zero. Verify with the artifact gate rather than assuming.

## Resolution (2026-07-17)

Fixed via approach **(1)** — qualify the alias during `prefix_package_symbols`.
Approach (1) turned out *cleaner* than (2), not just more uniform: once the
`alias` string carries the identity prefix, every downstream consumer that keys
off it — the merge dedup (`alias, name`), `link_thunk_symbol(alias, name)`, the
routing import name (`alias.func`), and the CSTRUCT join (`c.alias ==
function.alias`) — becomes package-distinct automatically, with **zero** changes
to `link_thunk_symbol`, `nir::link_routing_imports`, or the merge dedup key
itself. The wrapper-body references are rewritten in the same pass by folding the
routing names into the existing `own_fns` prefixing set, so the "three
load-bearing places" move together and the collision cannot reappear one layer
down. (2) would have left the routing import *name* colliding — two surviving
link functions both named `fooLink.raw` overwrite each other in the
name→symbol map — so it was insufficient without also rewriting body references,
i.e. it collapses into (1) anyway.

Reproduction note: the checked-in fixture distinguishes the two packages by
binding two different **symbols in the same system library** (`sqlite3_strglob`
vs `sqlite3_stricmp`), not two different vendored libraries. The routing defect is
identical — a dropped thunk misroutes to the surviving package's `(library,
symbol)` — and system `sqlite3` keeps the fixture deterministic and portable
across every supported arch without shipping custom multi-arch binaries. Observed
pre-fix exit was **70** (not 77) with these symbols; post-fix **73**.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Runtime fixture `tests/rt-behavior/native/native-link-alias-collision-rt`:
      two packages sharing alias `fooLink` + function `raw`, each bound to a
      different `sqlite3` symbol; consumer asserts each routes to its own symbol.
      Confirmed it exited **70** (not 73) against pre-fix behavior.
- [x] The collision fixture asserts BOTH directions in one program (exit encodes
      both `op()` results), so a misroute of either thunk is caught.
- [x] Blast-radius verdicts confirmed against the tree.

### Phase 2 — the fix

- [x] Alias qualified in `prefix_package_symbols` with the identity prefix; the
      routing name, thunk symbol, and merge dedup key all inherit it. No change to
      `link_thunk_symbol` was needed — the qualified alias makes it distinct.
- [x] Diamond import still collapses: the same package reached twice gets the same
      content-addressed prefix, so `(alias, name)` matches and dedups to one thunk.

### Phase 3 — validation

- [x] `scripts/test-accept.sh native-*` green (43 tests, incl. the new fixture and
      the imported-resource / cstruct / re-export fixtures — no golden churn).
- [x] Fixture exits 73 end-to-end; manual two-package repro also 73.

### Phase 3 — regenerate expected outputs + full validation

- [ ] Run `scripts/artifact-gate.sh` and confirm the codegen delta is only the
      intended thunk-symbol change (expected: nothing, since no existing fixture
      collides).
- [ ] Regenerate any goldens the symbol change shifts; diff and confirm the delta
      is only that.
- [ ] `scripts/test-accept.sh` green; re-run the reproduction end-to-end.

Acceptance: full suite green; golden deltas are exactly the intended change; the
reproduction exits 73.
Commit: —

## Validation Plan

- Regression test: the two-package collision fixture under
  `tests/rt-behavior/native/` (exit 73), plus the distinct-alias contrast.
- Runtime proof: **required** — this bug is invisible to a build assertion. Only
  running the binary and observing which library answered proves the routing
  (`.ai/compiler.md` runtime completion gate). The exit code encodes both
  libraries' results, so a misroute is unambiguous.
- Doc sync: if the alias's routing identity becomes package-qualified, update
  `./mfb spec package native-bindings`.
- Full suite: `scripts/test-accept.sh`, `scripts/artifact-gate.sh`,
  `cargo test --bin mfb`.

## Open Decisions

- **Fix shape** — recommend (2) `(package, alias, name)` identity + a
  package-distinct thunk symbol, over (1) qualifying the alias in
  `prefix_package_symbols`. (§Fix Design)
- **Type dedup by bare name** (`merge_package`, types) — audit separately; it is
  the same shape of hazard and is currently deliberate. Out of scope here.

## Summary

A one-line dedup key is wrong: link functions are deduped on a **package-internal**
identity in a **project-global** namespace, so two packages that never see each
other silently share one thunk and one library. The engineering risk is not the
dedup key itself but the three-way load-bearing role of `alias` (dedup key, body
reference, thunk symbol) — change it in one place only and the collision simply
moves down a layer. The `.mfp` format, the interface trailer, diamond-import
collapsing, and plan-46's vendor resolution are all correct and stay untouched.
