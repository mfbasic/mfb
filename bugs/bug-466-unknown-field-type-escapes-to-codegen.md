# bug-466: field access on an un-imported package's record escapes the type checker into native lowering, producing an unlocated internal error

Last updated: 2026-08-30
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (diagnostics)

Status: Open
Regression Test: `tests/syntax/tcp/local-address-field-without-net-import/` (new)

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
- **User packages returning another package's record** — **latent, not verified.** The mechanism is in the shared type table, not in builtin-specific code, so a user package should hit the same hole; confirming needs a multi-package fixture and is deferred to Phase 1.
- `src/target/shared/plan/lower.rs:180` and `builder_value_semantics.rs:560` — **hardened by this bug**: after the front-end gate closes, these become unreachable from source and should say so.

## Fix Design

The fix is a **post-inference sweep**, not a change to unification. After type
inference completes and before IR verification hands off to lowering, walk the
function bodies for any expression whose resolved type is still `Unknown` and
report `TYPE_UNKNOWN_VALUE` at its location. That preserves `Unknown`'s
provisional role during inference (the empty-collection case refines and passes
the sweep) while guaranteeing nothing provisional survives into codegen.

Rejected alternative: rejecting `Unknown` in argument checking specifically. It
would fix both reproductions but leaves the general hole open — any *other*
expression position that forgets to check would produce the same unlocated error.
The sweep is one gate covering all of them.

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

- [ ] Add `tests/syntax/tcp/local-address-field-without-net-import/` covering Reproductions 1 and 2, with the current (bare) output as the golden. RED: the golden shows the unlocated error that must change.
- [ ] Add the Reproduction 3 cases to the same fixture family as **characterization** tests — they already pass and must keep passing.
- [ ] Add Reproduction 4 as a fixture, pinning today's "unrelated import changes the verdict" behavior so the Open Decision below is made deliberately.
- [ ] Confirm whether a **user package** returning another package's record reproduces (Blast Radius, unverified item). Write the verdict into this file.

Acceptance: fixtures 1/2/4 pin the current wrong behavior; fixture 3 passes; the user-package verdict is recorded.
Commit: —

### Phase 2 — the gate

- [ ] Add the post-inference `Unknown` sweep, reporting `TYPE_UNKNOWN_VALUE` at the offending expression's location.
- [ ] Where the `Unknown` originates in a field access on a type with no loaded definition, extend the message to name the type and the package to import.
- [ ] Update the goldens for fixtures 1/2 to the new located diagnostic.

Acceptance: Reproductions 1 and 2 produce located, coded diagnostics; Reproduction 3 unchanged; no source program reaches either codegen escape site.
Commit: —

### Phase 3 — validate + resolve the visibility inconsistency

- [ ] Act on the Open Decision for Reproduction 4 and update its golden accordingly.
- [ ] `cargo test --release --no-fail-fast` plus `test-accept.sh`; `artifact-gate.sh all` (a front-end-only gate should drift no `.ncodesum` — an unexpected diff here is a bug-hunt trigger, not a re-baseline).
- [ ] Re-run all four reproductions.
- [ ] Correct `mfb man tcp localAddress`'s "the next call that consumes it fails to resolve" — passing the whole record does resolve; it is field access that fails.

Acceptance: full suite green; no `.ncodesum` drift; all four reproductions behave as designed.
Commit: —

## Validation Plan

- Regression tests: the syntax fixture family above (2 RED-then-green, 3 characterization, 1 decision-pinning).
- Runtime proof: not applicable — this is a compile-time diagnostic bug; the proof is the golden `build.log`.
- Doc sync: `mfb man tcp localAddress` (and the `tls`/`udp` equivalents, which repeat the same sentence).
- Full suite: `cargo test --release --no-fail-fast`, `test-accept.sh`, `artifact-gate.sh all`.

## Open Decisions

- **Should `IMPORT udp` keep making `net::Address`'s fields visible to a `tcp` program?** Recommended: **no** — record-definition visibility should follow the file's own imports, not leak through whichever imported package happens to declare a record referencing the type. It makes the `IMPORT net` rule honest and the failure reproducible. Alternative (cheaper): accept the leak as harmless once Phase 2 guarantees a located diagnostic in the unmasked case, and document it. The Phase 1 fixture exists to force this choice rather than let it drift.
- **Diagnostic code for the improved message.** Recommended: reuse `TYPE_UNKNOWN_VALUE` (2-203-0043) with a richer message, since that is what the working cases already emit and consistency beats a new code. Alternative: a dedicated "field access requires import" code, better targeted but a new rules-table entry.

## Summary

The engineering risk is in the gate's placement, not its logic: `Unknown` must stay
provisional through inference (the empty-collection binding depends on it), so the
check has to be a post-inference sweep rather than a unification change — that is
the one way to get this wrong. Two codegen sites currently absorb the escapee and
print unlocated, uncoded errors, one of which blames a call three levels away from
the mistake. The underlying visibility rule is sound and stays; what changes is
that violating it is diagnosed, and diagnosed the same way regardless of which
unrelated packages the file imported. Untouched: `Unknown`'s role in unification,
import non-transitivity, and the binding/operator diagnostics that already work.
