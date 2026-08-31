# bug-466: field access on an un-imported package's record escapes the type checker into native lowering, producing an unlocated internal error

Last updated: 2026-08-31
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (diagnostics)

Status: Fixed
Regression Test: `tests/syntax/tcp/local-address-field-*` (4 new fixtures) plus five
`src/ir/shape.rs` unit tests (`foreign_record_field_*`,
`unrelated_import_does_not_make_a_foreign_record_field_readable`)

Reading a field off a value whose record type belongs to a package the file did
not `IMPORT` yields `Unknown`. The type checker catches that `Unknown` when it is
bound (`LET p AS Integer = b.port`) or fed to an operator, reporting
`TYPE_UNKNOWN_VALUE` (2-203-0043) with a source location. It does **not** catch it
when the `Unknown` is passed as a **call argument** — to a user-declared `FUNC` or
to an overloaded builtin. There it survives every front-end gate and reaches
native lowering, which fails with a bare, unlocated, uncoded message:

```
error: native plan has no storage class for type 'Unknown'
```

There is no file, no line, no error code, no caret — nothing pointing at
`b.port` or naming the missing `IMPORT net`. It reads like an internal compiler
assertion, not a user error, and gives no hint that adding one import fixes it.

**The single correct behavior a fix produces:** every one of the reproductions
below is refused by the type checker with a located, coded diagnostic naming the
offending field access; no `Unknown` reaches `src/target/shared/plan/lower.rs`.
Ideally the diagnostic names the missing import, since that is always the cause
and always the fix.

A second, quieter defect sits underneath it: **whether the program compiles at all
depends on which unrelated packages you imported.** Adding `IMPORT udp` — never
referenced — makes the failing `tcp` program build. See Reproduction 4.

References:

- `src/target/shared/plan/lower.rs:180` — where the first escape lands.
- `src/codegen/memory/value/builder_value_semantics.rs:560` — where the second escape lands.
- `src/rules/table.rs:549` `TYPE_UNKNOWN_VALUE` — the diagnostic that *should* fire and does in the non-call cases.
- `mfb man tcp localAddress` — documents the rule ("Without it the returned value has no nameable type and the next call that consumes it fails to resolve"). The rule is right; the enforcement is incomplete, and the prose's "the next call that consumes it" is exactly the case that is NOT enforced.
- Found during: probing bug-465's reproduction, where the first version of the probe hit this instead of the behavior under test.

## Failing Reproduction

All probes on macos-aarch64 with `target/release/mfb`, in a standard executable
project. Note `IMPORT net` is absent from every failing case and present in none —
that is the trigger.

### 1. `Unknown` into an overloaded builtin call — unlocated error

```
IMPORT tcp

FUNC main AS Integer
  RES s = tcp::listen("127.0.0.1", 0)
  LET b = tcp::localAddress(s)
  RES c = tcp::connect("127.0.0.1", b.port)
  RETURN 0
END FUNC
```

- Observed: `error: native plan has no storage class for type 'Unknown'` — no path, no line, no code, exit 1.
- Expected: a located `TYPE_UNKNOWN_VALUE`-class diagnostic on line 6 pointing at `b.port`.

### 2. `Unknown` into a user-declared `FUNC` — a *different* unlocated error

```
IMPORT tcp
IMPORT io

FUNC take(n AS Integer) AS Integer
  RETURN n
END FUNC

FUNC main AS Integer
  RES s = tcp::listen("127.0.0.1", 0)
  LET b = tcp::localAddress(s)
  io::print(toString(take(b.port)))
  RETURN 0
END FUNC
```

- Observed: `error: native code field access target 'Address' is not a record or variant while lowering eval call io.print` — again unlocated and uncoded, and it blames `io.print`, which is three calls away from the actual mistake.
- Expected: same located diagnostic as case 1.

Note the message proves the mechanism: the *type name* `Address` propagated fine;
it is the record's **field table** that is absent.

### 3. Contrast cases the checker DOES catch correctly (must keep working)

```
LET p AS Integer = b.port
```
→ `./src/main.mfb:6 error[2-203-0043 TYPE_UNKNOWN_VALUE]: value type could not be determined` ✅ located, coded.

```
LET p = b.port + 1
```
→ `2-203-0043` + `2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH` ✅ located, coded.

```
io::print(toString(b.port))
```
→ `./src/main.mfb:7 error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: … has argument type(s) (Unknown), expected Integer, …` ✅ located, coded.

So the gate exists and works — it simply has holes at the two call-argument paths.

### 4. An unrelated import silently changes the verdict

Identical program to Reproduction 1, with one unused import added:

```
IMPORT tcp
IMPORT udp        ' never referenced

FUNC main AS Integer
  RES s = tcp::listen("127.0.0.1", 0)
  LET b = tcp::localAddress(s)
  RES c = tcp::connect("127.0.0.1", b.port)
  RETURN 0
END FUNC
```

- Observed: **`Wrote executable to ./build/diag.out`** — builds clean.
- Expected: the same diagnostic as Reproduction 1 (the program is no more correct than before), or `IMPORT net` genuinely being required and `IMPORT udp` being irrelevant.

Passing the whole record rather than a field is also accepted without `IMPORT net`
— `RES c = tcp::connect(b)` compiles — which is reasonable but means the man
prose's "the next call using it fails to resolve" overstates the rule.

## Root Cause

**The escape (primary).** Field access on a value whose record definition is not
in the file's type table produces `ParameterType::Unknown`. The checker's
`TYPE_UNKNOWN_VALUE` gate fires on binding sites and operator operands, but
argument checking does not reject `Unknown`: for a user `FUNC` the declared
parameter type accepts it, and for an overloaded builtin like `tcp::connect`
(4 overloads) `Unknown` unifies against a candidate rather than failing
resolution. This is consistent with `Unknown` being deliberately *provisional and
refinable* rather than an error type — the property an empty-collection binding
depends on — but nothing re-checks that a provisional `Unknown` was ever actually
refined before lowering. It reaches:

- `src/target/shared/plan/lower.rs:180` — "native plan has no storage class for type '{base}'", when the `Unknown` needs a storage class; or
- `src/codegen/memory/value/builder_value_semantics.rs:560` — "native code field access target '{}' is not a record or variant", when the field access itself is lowered.

Neither site has a source location to attach, so both print bare. Both are
codegen-internal invariants being asserted about a condition the front end should
have rejected — a missing gate, not a codegen bug.

**The visibility inconsistency (Reproduction 4).** `net::Address`'s field table is
loaded into a file when an imported package **declares a record** that references
it — not when an imported package merely returns it from a function. Verified by
counting record declarations:

```
$ grep -c "add_record" src/codegen/builtins/{tcp,tls,udp}/mod.rs
tcp: 0
tls: 0
udp: 1        # `Datagram`, whose `from` field is a net::Address
```

`tcp` and `tls` declare **zero** records — only opaque resources — so importing
either brings in no record definitions and `Address`'s fields stay invisible.
`udp` declares `Datagram`, whose field type drags `Address`'s definition in, which
is why `IMPORT udp` fixes an unrelated `tcp` program. All three packages declare
`pkg.add_imports(vec!["net"])` identically (`tcp/mod.rs:161`, `tls/mod.rs:144`,
`udp/mod.rs:139`), so the declared package-import list is *not* what governs it.

That makes the user-facing rule ("a program that uses those addresses must
`IMPORT net`") true but under-enforced, and accidentally satisfiable.

**Correction (found while fixing): the visibility inconsistency has a second
half the analysis above missed, and it is the half that matters to the fix.**
The type-table account is right about why the `IMPORT udp` program *compiles* —
`Address`'s field table is present, so `b.port` types. But the file's **import
list** is widened too, and by a different mechanism: `monomorph::lower` emits
every generated instantiation into the **first** file and unions every other
file's imports into it, so those bodies' package-qualified calls still resolve
("union every source file's imports into it", `src/monomorph/lower.rs`). After
monomorphization `main.mfb`'s import list therefore reads
`{tcp, udp, net, strings, collections}` for a program that wrote only `IMPORT
tcp` and `IMPORT udp` — `net`, `strings` and `collections` all arrive from the
injected `udp` companion source. Measured with a probe on
`Walker::check_foreign_record_field`:

```
PROBE-CHK file=main.mfb member=port target_type=Address imports={"tcp": "tcp"}
PROBE-CHK file=main.mfb member=port target_type=Address imports={"strings": "strings",
          "net": "net", "udp": "udp", "tcp": "tcp", "collections": "collections"}
```

(first line: `IMPORT tcp` alone; second: with `IMPORT udp` added.)

The widening is correct for lowering, which needs it, and invisible to name
resolution, which ran on the original AST and still refuses a `net::` call in a
file that did not import `net` (verified: `net::percentEncode` in a file
importing only `udp` and `io` gives `2-201-0014 SYMBOL_UNKNOWN_IMPORT`). But it
means *the post-monomorph HIR cannot answer "what did the author import?"* — so a
gate keyed on `HirFile::imports` would have let the unrelated `IMPORT udp`
silently reopen the hole. `HirFile` now carries `own_imports`, snapshotted at
elaboration and copied verbatim by monomorph.

## Goal

- Reproductions 1 and 2 are refused by the type checker with a located, coded diagnostic pointing at the field access.
- No `Unknown` reaches `src/target/shared/plan/lower.rs:180` or `builder_value_semantics.rs:560` from any source program; if one does it is an ICE-class bug, not a user-facing error.
- Reproduction 4 no longer depends on an unrelated import: the same program either compiles in both cases or fails in both.
- Reproduction 3's existing correct diagnostics are unchanged.

### Non-goals (must NOT change)

- **Do NOT make `Unknown` an error type in unification.** It must stay provisional and refinable — an empty-collection binding (`MUT xs = []`) depends on a later use refining it, and hard-failing at unification would break that. The fix is a *post-inference* check that every `Unknown` was refined, not a change to `unify_type`.
- **Do NOT fix this by widening what `IMPORT tcp` makes visible.** Auto-importing `net`'s records because `tcp` returns them would make imports transitive, which the language deliberately refuses (spec: packages cannot re-export another's types). The rule stays; only its enforcement and consistency change.
- Do not remove or weaken `TYPE_UNKNOWN_VALUE`'s existing binding/operator coverage (Reproduction 3).
- No change to `tcp`/`tls`/`udp` public surface, record layout, or `net::Address`.

## Blast Radius

The trigger is "field access on a record type the file did not import", so the
exposure is every builtin function returning another package's record. Found with
`grep -rn "ParameterType::named(\"Address\")"`-class searches over the registry
plus the record census above.

- `tcp::localAddress`, `tcp::remoteAddress` — **reproduce** (verified, cases 1–2). `tcp` declares no records.
- `tls::localAddress`, `tls::remoteAddress` — **reproduce** (verified: the tls form gives the identical `no storage class for type 'Unknown'`).
- `udp::localAddress`, `udp::receive` — **do not reproduce**, because `udp` declares `Datagram` and that pulls `Address`'s fields in. Same latent hole, accidentally masked.
- `net::lookup` + `net::toUrl` — **unaffected**; using them requires `IMPORT net` already, so `Address`/`Url` fields are always visible.
- `http` — **unaffected in practice**; `http::Request`/`Response` are declared by `http` itself, and `http::startRead` takes a `net::Url` which forces `IMPORT net` at the call site anyway.
- **User packages returning another package's record** — **verified: DOES NOT reproduce.** Built the three-project model `rt_foreign_type_reexport.rs` uses (`pa466` exports `TYPE A`; `pc466` imports it and exports `makesA() AS A`; `app466` imports only `pc466`) and read `v.n` off the result with no `IMPORT pa466`: it builds and runs clean. A `.mfp` carries the foreign type's full definition through the re-export path bug-390 added, so the app has `A`'s field table without importing its owner. The hole is specific to BUILTIN packages, whose companion source is injected only when some file imports them — there is no `.mfp` to carry `net::Address`'s fields into a `tcp`-only file. The gate is therefore scoped to builtin-package records; the user-package behavior is left exactly as it is.
- `src/target/shared/plan/lower.rs:180` and `builder_value_semantics.rs:560` — **hardened by this bug**: after the front-end gate closes, these become unreachable from source and should say so.

## Fix Design

The fix is a **pre-lowering rule on the field access itself**, in `ir::shape` —
not a change to unification, and not a check on the resulting `Unknown`.

Reading a field off a record a **builtin package** declares requires that
package's own `IMPORT` in the file. That is the documented language rule (`mfb
man tcp localAddress`); what changes is that violating it is now diagnosed,
located and coded, at the read. `Unknown`'s provisional role in inference is
untouched.

Gating the *access* rather than the *value* is what closes the hole for good.
The value was already caught wherever it was bound or fed to an operator, and
escaped only through the two call-argument paths; a check on the value has to be
right in every position it can reach, and the two positions nobody checked are
exactly the ones this bug is. The access has one position.

**Rejected: the post-inference `Unknown` sweep this document originally
proposed.** It would fix Reproductions 1 and 2, but it cannot fix Reproduction
4 — under `IMPORT udp` the read *does* type (`Address`'s field table is in the
project-wide table), so a sweep for surviving `Unknown`s sees nothing to report
and the program still compiles. "Same verdict either way" is not derivable from
whether the type resolved; only from what the file imported.

**Rejected: rejecting `Unknown` in argument checking specifically.** Fixes both
reproductions, leaves the general hole open — any other expression position that
forgets to check produces the same unlocated error.

Where the rule does *not* apply: to compiler-injected package source
(`HirFile::internal`), which declares the guarded types and is authored against
the registry's own import graph; to `.state`, which is not a record field and has
its own more precise rules; and to any type name the project itself declares, an
imported `.mfp` exports, or two builtin packages both declare — there the nominal
does not unambiguously mean the builtin's record, so there is no import to name.

Improving the message is worthwhile but secondary: when the `Unknown` came from a
field access on a known-but-undefined record type, the diagnostic should say so
and name the package to import — the information is available at
`builder_value_semantics.rs:560` today (it prints `'Address'`), so it is available
to the front end too.

Reproduction 4's inconsistency is arguably a separate decision — see Open
Decisions — but the sweep makes it *safe* either way, because both variants would
then be diagnosed rather than one silently compiling.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [x] Add `tests/syntax/tcp/local-address-field-without-net-import/` and `…-user-func-without-net-import/` covering Reproductions 1 and 2. RED: the front end lets both through (`[exit 0]`, `.ast`/`.ir` dumped); the failure was only in native lowering.
- [x] Add the Reproduction 3 cases as `…-binding-without-net-import/` — **characterization**, already passing.
- [x] Add Reproduction 4 as `…-unrelated-udp-import/`, pinning the "unrelated import changes the verdict" behavior so the Open Decision is made deliberately.
- [x] Five `src/ir/shape.rs` unit tests, RED for the documented mechanism: the field read produced **zero** diagnostics where a located one must appear.
- [x] Confirm whether a **user package** returning another package's record reproduces. **Verdict: it does not** — see Blast Radius. A `.mfp` carries the foreign type's definition through bug-390's re-export path, so the consumer has the field table without importing the owner. The hole is specific to builtin packages.

Acceptance: met. Reproductions re-run on `target/release/mfb` (macos-aarch64) before any change:
repro 1 `error: native plan has no storage class for type 'Unknown'`; repro 2 `error: native code field access target 'Address' is not a record or variant while lowering eval call io.print`; repro 4 `Wrote executable to …`; repro 3 located `2-203-0043` / `2-203-0021`.
Commit: `175997a63`

### Phase 2 — the gate

- [x] Add the field-access rule in `ir::shape` (`check_foreign_record_field`), reporting `TYPE_UNKNOWN_VALUE` at the read's own line.
- [x] Message names the field, the owning package, the type, and the import to add.
- [x] Add `HirFile::own_imports` so the rule reads the author's import list, not the post-monomorph union (see the Root Cause correction).
- [x] Fill the goldens for all four fixtures with the new located diagnostic.

Acceptance: met. Reproductions 1, 2 and 4 now report the identical located `2-203-0043` at the field read; Reproduction 3's binding/operator/argument diagnostics are unchanged; the `IMPORT net` program builds and runs.
Commit: `79b8ec348`

### Phase 3 — validate + resolve the visibility inconsistency

- [x] Open Decision for Reproduction 4 acted on: **refuse in both cases** (the recommended option). Its golden now matches Reproduction 1's byte for byte apart from the line number.
- [x] `cargo test --release --no-fail-fast`, `test-accept.sh`, `artifact-gate.sh all`.
- [x] Re-run all four reproductions.
- [x] Correct `mfb man tcp localAddress` and `mfb man tcp`: passing the whole record does resolve (verified — `tcp::connect(bound)` compiles with no `IMPORT net`); it is field access that fails.

Acceptance: PHASE3_ACCEPTANCE
Commit: PHASE3_COMMIT

## Validation Plan

- Regression tests: `tests/syntax/tcp/local-address-field-{without-net-import,user-func-without-net-import,unrelated-udp-import}` (RED-then-green) and `…-binding-without-net-import` (characterization), plus five `src/ir/shape.rs` unit tests — four RED before the fix, one (`…with_the_owning_import_is_accepted`) green throughout so the rule cannot be satisfied by rejecting everything.
- Runtime proof: not applicable — this is a compile-time diagnostic bug; the proof is the golden `build.log`. The positive side is covered at runtime by the pre-existing `tests/rt_tls_listener_local_address.rs`, which reads `bound.host`/`bound.port` with `IMPORT net` present.
- Doc sync: `mfb man tcp localAddress` and `mfb man tcp` corrected. The `tls`/`udp` equivalents did **not** repeat the wrong sentence — grep for "next call that uses it fails to resolve" found exactly one occurrence, and `tcp/mod.rs` carried a paraphrase of the same claim; both are fixed and the others ("nameable only where `net` is imported") were already accurate.
- Full suite: `cargo test --release --no-fail-fast`, `test-accept.sh`, `artifact-gate.sh all`.

## Open Decisions

- **Should `IMPORT udp` keep making `net::Address`'s fields visible to a `tcp` program?** **Resolved: no** (the recommended option). The field-access rule keys on the file's own imports, so both spellings are refused identically.
  Note what was *not* done: the project-wide type table still holds `Address` whenever some file imports `net`, and `HirFile::imports` is still widened by monomorph. Both are load-bearing (lowering needs the widened list; the type table is how injected companions see each other) and neither is observable now that the rule asks the author's list instead. Scoping record-definition visibility per file would be a much larger change to `TypeIndex` with no remaining user-visible payoff.
- **Diagnostic code for the improved message.** **Resolved: reuse `TYPE_UNKNOWN_VALUE`** (2-203-0043), as recommended — it is what the working cases already emit, and Reproduction 3 now shows the two forms side by side under one code.

## Summary

The engineering risk was in the gate's placement, and this document's original
proposal placed it wrong. `Unknown` must stay provisional through inference (the
empty-collection binding depends on it), so the check could not go in
`unify_type` — that much was right. But the proposed alternative, a post-inference
sweep for surviving `Unknown`s, cannot satisfy this bug's own goal: under `IMPORT
udp` the offending read *does* type, so there is no `Unknown` left to sweep and
Reproduction 4 still compiles. "Same verdict regardless of unrelated imports" is
not a property of whether a type resolved; it is a property of what the file
imported. The gate is therefore on the field **access**, keyed on the author's
import list, pre-lowering in `ir::shape`.

Getting the author's import list took a second finding: monomorphization widens
the first file's `imports` with the project-wide union, so the post-monomorph HIR
cannot answer "what did the author write?". `HirFile::own_imports` now carries
that answer, snapshotted at elaboration.

Two codegen sites used to absorb the escapee and print unlocated, uncoded errors,
one of which blamed a call three levels away from the mistake. The underlying
visibility rule is sound and stays; what changed is that violating it is
diagnosed, and diagnosed the same way regardless of which unrelated packages the
file imported. Untouched: `Unknown`'s role in unification, import
non-transitivity, the binding/operator diagnostics that already worked, the
project-wide type table, monomorph's import widening, and the `tcp`/`tls`/`udp`
public surface.
