# bug-486: an inline `TRAP` does not catch `toString(List OF Byte)` failing on invalid UTF-8

Last updated: 2026-09-02
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: `tests/rt-behavior/trap/inline-trap-tostring-bytes-rt/` (new, Phase 1)

`toString(List OF Byte)` decodes bytes as UTF-8 and **raises** `77020004`
(`7-702-0004`, "Text encoding or decoding failed") when they are not valid UTF-8 —
`toByte(0xE9)` in a Latin-1 / windows-1252 byte run is enough. An inline `TRAP`
attached to that call does not catch it: the process dies with exit 255 and the
`RECOVER` never runs. A function-level `TRAP` in the same function *does* catch it.

The compiler is not merely silent about this, it argues the opposite. For a
bare-local scrutinee it emits `TYPE_INLINE_TRAP_DEAD_HANDLER` — *"`toString` cannot
fail, so the handler is dead code"* — advising the author to delete the handler that
is in fact load-bearing. For a scrutinee containing a nested call it emits **no
diagnostic at all** and silently drops the protection. `mfb spec language error-model`
§8.6 rule 11 states the same wrong claim, listing `toString` among the
"provably-infallible inline-lowered built-ins".

This matters wherever bytes arrive from outside the program. Any client decoding a
network or file body with the documented call-site idiom — `LET text = toString(resp.body)
TRAP(e) RECOVER ""` — aborts the whole process on hostile or merely legacy input,
with no way to recover at that call site.

**The single correct behavior a fix produces:** an inline `TRAP` on
`toString(<List OF Byte>)` catches the UTF-8 decode failure, binds the error to the
handler binding, and `RECOVER`s — behaving exactly like an inline `TRAP` on `toInt`
(spec rule 14's "conversion built-ins like `toInt` ... support inline TRAP the same
way"). No `TYPE_INLINE_TRAP_DEAD_HANDLER` is emitted for that argument type, and
`toString` on every *other* argument type stays infallible and keeps warning.

References:

- `mfb spec language error-model` §8.6 rule 11 (the wrong "provably-infallible"
  claim, which this fix must correct) and rule 14 (the conversion-built-in contract
  the fix restores).
- `mfb man errors` — "Local handling with an inline TRAP", the idiom that fails here.
- Sibling inline-`TRAP` defect: `bugs/bug-479-inline-trap-on-thread-start-fails-native-lowering.md`
  (same feature, different failure — that one fails to *build*; this one builds and
  silently drops the handler). Not the same root cause; do not fold them.
- `bugs/bug-457` is cited in `src/ir/lower.rs:lower_inline_trap` for the nested-call
  hoist that shape A here re-enters from the other side.
- Found while auditing `examples/browser` (`fetch::pageResult`) error handling.

## Failing Reproduction

Committed at `bugs/repro/bug-486-tostring-bytes-inline-trap.mfb`. Copy it to a scratch
executable project's `src/main.mfb` and build:

```
rm -rf /tmp/bug486 && mkdir -p /tmp/bug486/src
cp bugs/repro/bug-486-tostring-bytes-inline-trap.mfb /tmp/bug486/src/main.mfb
cat > /tmp/bug486/project.json <<'EOF'
{ "name": "bug486", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
EOF
./target/release/mfb build /tmp/bug486 && /tmp/bug486/build/bug486.out
```

The repro carries three shapes over the same invalid bytes (`99 97 102 233`):

| Shape | Scrutinee | Build diagnostic | Runtime |
| --- | --- | --- | --- |
| A | `toString(latin1())` — nested call | **none** | aborts ✗ |
| B | `toString(bytes)` — bare local | `TYPE_INLINE_TRAP_DEAD_HANDLER` (wrong) | aborts ✗ |
| C | `RETURN toString(bytes)` + function-level `TRAP` | none | catches ✓ |

- Observed (build): one warning, on shape B only —
  `warn[2-203-0104 TYPE_INLINE_TRAP_DEAD_HANDLER]: ... `toString` cannot fail, so the handler is dead code.`
- Observed (run): `Error: 7-702-0004` / `Text encoding or decoding failed.`, exit `255`,
  before shape A's `io::print` — so A never even reaches B.
- Expected (run), exit `0`:
  ```
  A caught
  B caught
  C caught: 77020004
  ```
- Expected (build): no `TYPE_INLINE_TRAP_DEAD_HANDLER` for either A or B.

**Contrast cases that work correctly today** (all must stay working — they bound the
bug and become regression guards):

- Inline `TRAP` on a raising package call — `strings::mid("abc", 10, 1)` — catches.
- Inline `TRAP` on the sibling conversion `toInt("zz", 10)` — catches.
- Function-level `TRAP` around `toString(<bytes>)` — catches (shape C). This is why
  `examples/browser` survives a non-UTF-8 page: `fetch::pageResult` decodes with no
  inline trap and relies on `fetch::fetch`'s function-level handler.
- `toString(42)` under an inline `TRAP` — correctly warns `TYPE_INLINE_TRAP_DEAD_HANDLER`
  and the handler is genuinely dead
  (`tests/rt-behavior/trap/inline-trap-infallible-builtin-valid`).

Platform: reproduced on macos-aarch64, default optimizer level. Not
platform-dependent by inspection — the defect is in shared IR lowering and a shared
census, not a backend — but Phase 3 must confirm on the Linux axis.

## Root Cause

One wrong fact, consulted from three places.

`src/codegen/builtins/mod.rs:inline_builtin_is_infallible` hard-codes:

```rust
if matches!(target, "len" | "toString" | "typeName") {
    return true;
}
```

The census is keyed on the **callee name only** — `src/ir/fallible.rs:Fallibility`
documents why (*"Overloads share a name and a call site carries no types here"*). But
`toString` is overloaded across every type, and exactly one of those overloads is
fallible: `List OF Byte → String` performs a UTF-8 decode that raises `77020004`.
Every other overload (`Integer`, `Float`, `Boolean`, `AttributedString`, a record, …)
really is total, which is why a name-keyed verdict looked safe.

`src/ir/fallible.rs:call_is_fallible` asks that census first and returns `false`, and
three consumers then act on it:

1. **`src/ir/lower.rs:lower_inline_trap`** (shape A). `check_root` is
   `hoists.is_empty() || context.fallible.call_is_fallible(target)`. With a fallible
   call nested in the scrutinee (`latin1()`) the hoist list is non-empty, so the root
   is checked *only if it is fallible* — and `toString` reports infallible, so the
   root is emitted as a plain `Call`. Confirmed by `mfb build --ir`: inside the Ok
   branch the IR is
   `{"op":"assign","name":"$trap_val3","value":{"kind":"call","type":"String","target":"toString",...}}`
   — a bare `call`, never a `callResult`. The raise auto-propagates past the handler
   to the function-level trap (or, absent one, to process exit). This is precisely the
   failure mode `lower_inline_trap`'s own bug-457 comment describes for *nested* calls,
   re-entered from the root side.
2. **`src/ir/verify/resources.rs`** (~line 858, the `CallResult | Call` arm) emits
   `TYPE_INLINE_TRAP_DEAD_HANDLER` from the same census — the wrong advice on shape B.
3. **`src/ir/fallible.rs:analyze`**'s fixpoint, so a user function whose only failure
   path is a byte decode is itself judged infallible, propagating the error up the
   call graph.

**Shape B is a second, distinct defect and the root cause is not yet proven.** Its IR
*is* correct — `mfb build --ir` shows
`{"op":"bind","name":"$trap_res0","type":"Result OF String","value":{"kind":"callResult","target":"toString",...}}`
followed by a `resultIsOk` test and a populated `else` — and it still aborts. So
something below IR does not honour `CallResult` for this builtin. Hypotheses, most
likely first:

- **(H1)** The native lowering of an inline-lowered builtin ignores the `CallResult`
  wrapper and emits the raising helper directly, so no `Result` is ever produced.
  `src/target/shared/nir/lower.rs` carries `IrValue::CallResult` through to
  `NirValue::CallResult` unchanged, so the divergence is further down, in the
  per-target emission of an inline builtin. *Confirm:* `mfb build --nir` on shape B and
  follow the `toString` emission; if no error-slot/branch is produced, H1 holds.
- **(H2)** The byte→String conversion aborts through a fatal runtime path (a hard
  abort, not a raise) that the `Result` protocol cannot observe at all. *Eliminate:*
  shape C catches it with a real error *code* (`77020004`), which a fatal abort could
  not deliver — so H2 is already weak, but confirm the raise and the abort share one
  helper before ruling it out.
- **(H3)** An optimizer row strips the check. *Eliminate:* the repro reproduces at the
  default level; `src/optimizer/opt1/recovery.rs` self-guards on catalog level 3 and
  only removes *function-level* traps. Already effectively eliminated; record it so
  nobody re-litigates.

Phase 1 must settle which of these holds before any code changes, because H1 and H2
have different fixes and only H1 is contained.

## Goal

- An inline `TRAP` on `toString(<List OF Byte>)` catches `77020004` and `RECOVER`s,
  for **both** scrutinee shapes: a bare local (B) and an expression containing a
  nested fallible call (A).
- No `TYPE_INLINE_TRAP_DEAD_HANDLER` is emitted when the argument is `List OF Byte`.
- `TYPE_INLINE_TRAP_DEAD_HANDLER` is still emitted, unchanged, for `toString` on every
  other argument type, and for `len` / `typeName` / the total `bits::*` ops / the
  listed pure collection and string members.
- `mfb spec language error-model` §8.6 rule 11 no longer claims `toString` is
  unconditionally infallible.

### Non-goals (must NOT change)

- **`toString` must not become unconditionally fallible.** The name-keyed shortcut of
  deleting `"toString"` from `inline_builtin_is_infallible` is the tempting wrong fix:
  it would make `toString(42)` fallible, break the correct existing contract pinned by
  `tests/rt-behavior/trap/inline-trap-infallible-builtin-valid` (which traps
  `toString(42)` and expects `DEAD-toString` plus the warning), force `Result`
  plumbing through every string interpolation in the tree, and churn `.ncode` broadly.
  That fixture is **right**; per `AGENTS.md` it must stay green and must not be
  re-baselined to accommodate a lazy fix.
- The success path of `toString` on valid UTF-8 bytes — same value, same type.
- `len` and `typeName` stay infallible; this bug is not a licence to re-audit the whole
  census. Any other member found wrong during the Phase 1 audit gets its own bug.
- The function-level `TRAP` path (shape C) keeps working with the same error code —
  `examples/browser` depends on it.
- Error code `77020004` and its rendered form `7-702-0004` are unchanged.
- No change to `bug-479`'s build-time failure; the two are independent.

## Blast Radius

Found by searching the tree, not from memory: `grep -rn 'inline_builtin_is_infallible' src`
(4 consumers) and `grep -rn 'toString(' examples tests` for byte-typed arguments.

- `src/codegen/builtins/mod.rs:inline_builtin_is_infallible:273` — **fixed by this bug**;
  the wrong fact itself.
- `src/ir/fallible.rs:call_is_fallible:66` — **fixed by this bug**; must be able to
  express a type-conditional verdict.
- `src/ir/lower.rs:lower_inline_trap:1365` (`check_root`) — **fixed by this bug**;
  shape A's silent drop.
- `src/ir/verify/resources.rs` (~858, the `CallResult | Call` arm) — **fixed by this
  bug**; the wrong `DEAD_HANDLER` warning.
- `src/ir/shape.rs:1770` — consults `call_is_fallible` via `canonical_callee`. **Audit
  in Phase 1**: confirm the new type-aware verdict reaches it, or record why shape
  analysis is unaffected.
- `src/optimizer/opt1/recovery.rs` — **unaffected**: level-3 guarded and removes only
  function-level traps in a region proven non-raising; a `toString(bytes)` in that
  region will simply stop qualifying once the verdict is right.
- `src/audit/collect/source.rs:link_fallible_calls:780` — **audit**: a separate
  fallible-name set for the audit tooling; confirm whether it shares the claim.
- `examples/browser/fetch/src/lib.mfb:pageResult` — **unaffected but load-bearing**:
  decodes `toString(resp.body)` with no inline trap, relying on `fetch::fetch`'s
  function-level handler (shape C). It works today and must still work; it is the
  end-to-end proof for Phase 3.
- `tests/rt-behavior/trap/inline-trap-infallible-builtin-valid` — **must stay green
  unchanged** (see Non-goals).
- `tests/syntax/trap/inline-trap-infallible-builtin-invalid` — **audit**: confirm it
  pins no `toString`-on-bytes shape.
- Any other overloaded builtin whose fallibility varies by argument type — **latent,
  out of scope**: the census is name-keyed for `len` and `typeName` too. Both are
  believed total for every overload, but Phase 1 records the check rather than
  assuming it; anything found gets its own bug.

## Fix Design

Make the infallibility verdict **argument-type aware** for the narrow set of builtins
whose fallibility depends on the argument, then thread the argument types to the three
consumers.

The risk concentrates in the interface change, not the predicate: `call_is_fallible`
today takes only a name, and `Fallibility`'s doc comment explicitly justifies that
(*"a call site carries no types here"*). Every consumer above has an `IrValue::Call`
or `CallResult` in hand with its `args`, so the types are recoverable at each site via
the same `expression_type`/local map the lowering already uses — but the plumbing
touches four files and must not perturb the verdict for any other name.

Shape B's fix depends on Phase 1's hypothesis outcome and is deliberately not designed
here. If H1, the fix is to give the byte→String conversion a `Result`-returning
emission when reached through `CallResult`, mirroring `toInt`'s existing seam — look
at how `toInt` does it before writing anything new.

Rejected alternatives:

- **Drop `"toString"` from the census** — rejected; see Non-goals. Breaks a correct
  fixture and pessimizes every interpolation.
- **Introduce a distinct `decodeUtf8` builtin and leave `toString` infallible** —
  rejected: it does not fix the existing documented idiom, and every current
  `toString(bytes)` call site would keep the uncatchable abort. Worth considering as a
  *separate* ergonomic addition (an explicit, lossy-or-strict decoder is what the
  charset gap actually needs), but it does not close this bug.
- **Downgrade the raise to a lossy decode (U+FFFD)** — rejected: silently corrupts
  data, and changes the success-path semantics the Non-goals protect.

Expected generated-output shift: `.ncode` / `.ir` goldens for fixtures that inline-TRAP
a `toString` on bytes (today: none known — the shape is what this bug adds), plus the
new fixture's own goldens. A broad `.ncode` churn is a signal the change leaked into
the general `toString` path — investigate, do not regenerate.

## Phases

### Phase 1 — failing test + audit + hypothesis (no behavior change)

- [ ] Add `tests/rt-behavior/trap/inline-trap-tostring-bytes-rt/` covering shapes A, B
      and C from the repro, with all four goldens (`build.log`, `.ast`, `.ir`, `.run`)
      — a new rt fixture needs all four or a full `test-accept.sh` reports
      `unexpected actual` with no `mismatch:` line.
- [ ] Confirm it fails for the documented reason: A and B abort with `7-702-0004`,
      C passes, and the build log carries the wrong `DEAD_HANDLER` on B only.
- [ ] Settle H1/H2/H3 for shape B by dumping `--nir` and following the `toString`
      emission; write the verdict into Root Cause above.
- [ ] Complete the blast-radius audit: `src/ir/shape.rs:1770`,
      `src/audit/collect/source.rs:link_fallible_calls`,
      `tests/syntax/trap/inline-trap-infallible-builtin-invalid`, and whether `len` /
      `typeName` have any fallible overload. Write each verdict into this file.

Acceptance: the new fixture fails on A and B and passes on C; every audit site has a
recorded verdict; shape B's mechanism is proven, not hypothesised.
Commit: `—`

### Phase 2 — the fix

- [ ] Make `inline_builtin_is_infallible` (or a new type-aware sibling) answer for
      `toString` by argument type: fallible for `List OF Byte`, infallible otherwise.
- [ ] Thread argument types to `src/ir/fallible.rs:call_is_fallible` and its three
      consumers (`ir/lower.rs:lower_inline_trap`, `ir/verify/resources.rs`,
      `ir/fallible.rs:analyze`), leaving every other name's verdict bit-identical.
- [ ] Apply the shape-B fix indicated by Phase 1's proven mechanism.
- [ ] Update `mfb spec language error-model` §8.6 rule 11 to qualify `toString`, and
      rule 14's conversion-built-in list to include it.

Acceptance: the Phase 1 fixture passes end to end; every contrast case still behaves
as documented; `tests/rt-behavior/trap/inline-trap-infallible-builtin-valid` is green
and unmodified; nothing in Non-goals changed.
Commit: `—`

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate only the goldens the fix legitimately shifts; diff and confirm the
      delta is exactly the intended change. A broad `.ncode` churn means the change
      leaked into the general `toString` path — investigate rather than regenerate.
- [ ] `cargo test --no-fail-fast` (a failing `golden.rs` otherwise skips every later
      `rt_*`), then the acceptance harness `scripts/test-accept.sh` — it is not part
      of `cargo test`, so a green cargo run hides stale goldens. Watch the `N ran`
      count.
- [ ] Re-run the repro on macos-aarch64 and on the Linux axis; confirm A, B and C all
      report caught, exit 0.
- [ ] Rebuild `examples/browser` and confirm a non-UTF-8 page still surfaces as an
      error page rather than killing the browser (shape C unchanged).

Acceptance: full suite green; golden deltas are exactly the intended change; the
reproduction passes everywhere it previously failed.
Commit: `—`

## Validation Plan

- Regression test: `tests/rt-behavior/trap/inline-trap-tostring-bytes-rt/`, covering
  shapes A, B and C plus the absence of `DEAD_HANDLER` on the byte overload.
- Guard against the tempting wrong fix:
  `tests/rt-behavior/trap/inline-trap-infallible-builtin-valid` must stay green
  **and unmodified** — it pins that `toString(42)` is still infallible and still warns.
  Pair the RED test with this one; the shortest fix disables exactly this guarantee.
- Runtime proof: `bugs/repro/bug-486-tostring-bytes-inline-trap.mfb` printing
  `A caught` / `B caught` / `C caught: 77020004` with exit 0, plus `examples/browser`
  still rendering an error page for a non-UTF-8 body.
- Doc sync: `mfb spec language error-model` §8.6 rules 11 and 14 (required — the spec
  currently states the bug as intended behavior). Check whether `mfb man errors`
  repeats the claim.
- Full suite: `cargo test --no-fail-fast` plus `scripts/test-accept.sh`.

## Open Decisions

- **Where the type-aware verdict lives** — extend `inline_builtin_is_infallible` with an
  optional argument-type slice (recommended: one census, one place to be wrong) vs. a
  separate `builtin_is_fallible_for_args` consulted only by the inline-TRAP paths
  (smaller blast radius, but two lists that can disagree — the classic recogniser /
  measurer split). (§Fix Design)
- **Whether to also add an explicit decoder** (`encoding::utf8Decode`-style, strict and
  lossy) — recommended as a *follow-up* bug, not folded in here; it is what the
  non-UTF-8 charset gap really needs, but it does not close this one. (§Fix Design,
  rejected alternatives)

## Summary

The engineering risk is in the interface change, not the predicate: one hard-coded
name-keyed claim (`builtins/mod.rs:273`) is consulted by three IR consumers that
currently have no argument types to hand, and `Fallibility`'s own documentation
explains why. Making the verdict type-aware without perturbing any other name is the
work. A second, smaller unknown — why a correctly-emitted `callResult toString` still
aborts (shape B) — must be proven in Phase 1 before it is fixed. Left untouched: the
success path, `len`/`typeName`, the function-level `TRAP` route `examples/browser`
depends on, and the existing fixture proving `toString(42)` is genuinely infallible.
