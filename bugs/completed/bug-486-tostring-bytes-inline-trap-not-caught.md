# bug-486: an inline `TRAP` does not catch `toString(List OF Byte)` failing on invalid UTF-8

Last updated: 2026-09-02
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: FIXED
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

**Shape B — H1 CONFIRMED (Phase 1).** The dispatch is
`src/codegen/engine/value/builder_values.rs`'s `CallResult` arm, a **fourth** consumer
of the census this document did not list: it asks `inline_builtin_is_infallible(target)`
and, on `true`, routes to `lower_inline_infallible_raw`, which lowers the member with
**no `raw_result_capture` set**. `lower_to_string`'s `List OF Byte` arm then reaches
`raise_error_bare("ErrEncoding")` → `emit_error_register_return`, whose capture branch
is `None`, so the error auto-propagates exactly as at an untrapped call site. Proven
from `mfb build --ncode` on the repro, not from reading: at shape B's
`byte_list_string_invalid_39` label the emission is
`mov_imm x8, 77020004` … `bl _mfb_make_error_result` / `bl _mfb_rt_park_error` /
`ldr lr` / `add_sp` / **`ret`** — a return from `main`, with no branch to any
`raw_builtin_done`/`raw_conversion_done` capture label. H1 holds, and the fix is
contained: give the byte-list overload a raw-supported lowering
(`lower_inline_builtin_raw` + a `toString` arm), which reuses the existing capture
seam rather than inventing one.

**H2 eliminated.** The raise and the "abort" are the *same* path: shape C catches
`77020004` through the ordinary function-level trap route, and the `.ncode` above
shows one `emit_error_register_return` tail whose destination is chosen by
`raw_result_capture` / `error_exit_destination`. There is no separate fatal abort.

**H3 eliminated.** The error return is emitted directly by the `toString` lowering at
the default optimizer level; no optimizer row is involved. `opt1/recovery.rs` is
level-3 gated and removes only function-level traps.

### Blast-radius audit verdicts (Phase 1)

- `src/ir/shape.rs:1770` — **affected, fixed.** It consults `call_is_fallible` for the
  callee in a short-circuited `AND`/`OR` operand and has a full type oracle
  (`type_of` → `lower::expression_type`) in hand, so it now passes argument types.
  A short-circuited `toString(<bytes>)` — genuinely fallible, and the one shape the
  desugar cannot lift — is now reported instead of silently miscompiled.
- `src/audit/collect/source.rs:link_fallible_calls` / `is_fallible_builtin` —
  **does not share the claim; unaffected.** Measured with
  `grep -n '"toString"\|"len"\|"typeName"' src/audit/collect/source.rs` → no hits.
  That census is an opt-in list of *fallible* names for `mfb audit`'s reporting, so
  `toString` is absent rather than asserted-infallible. It therefore under-reports a
  byte decode in the Control-flow section. Left alone deliberately: it is an AST-level
  oracle with no types, and the module's own doc says a report that over-reports is
  noisy while a desugar that under-reports miscompiles — the two are separate by
  design. A type-aware audit oracle is its own concern, not this bug.
- `tests/syntax/trap/inline-trap-infallible-builtin-invalid` — **pins no `toString`
  shape.** Its two cases are a non-call literal (`5`) and a package constant
  (`math::pi()`), both `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`. Unaffected.
- `len` / `typeName` overload audit — **neither has a fallible overload.**
  `lower_len` (`builder_collection_layout.rs:1110`) has exactly two arms, `String`
  (a UTF-8 scalar count loop) and `typed_is_collection_type` (a count load); neither
  emits an error return — `grep -c 'raise_error\|emit_error_'` over the function is 0,
  and its third arm is a compile-time `Err(...)` for an unsupported argument type, not
  a runtime raise. `typeName` folds to a string constant
  (`static_type_name_for_fold` + `load_string_constant`) and emits no code that can
  fail. Both stay in the name-keyed half of the census; a unit test pins that a
  byte-list argument does not flip them.
- `mfb man errors` — **does not repeat the claim.** `grep -n 'infallible'
  src/docs/man/errors/package.md` → no hits. No man-page change needed.
- `src/codegen/builtins/http/helper_bytes_to_text.rs` — **found during the audit**: its
  MFBASIC comment documented the bug as intended behavior ("the inline-TRAP analysis
  treats `toString` as infallible and would elide an inline handler"). Corrected in
  place, same line count so no `ErrorLoc` shift. The helper keeps its function-level
  `TRAP`; only the rationale changed.

**Shape B's original write-up follows.** Its IR
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

- [x] Add `tests/rt-behavior/trap/inline-trap-tostring-bytes-rt/` covering shapes A, B
      and C from the repro, with all four goldens (`build.log`, `.ast`, `.ir`, `.run`)
      — a new rt fixture needs all four or a full `test-accept.sh` reports
      `unexpected actual` with no `mismatch:` line. **Grew to ten shapes A–J**; the
      seven beyond A/B/C are each a distinct failure found while fixing, listed under
      Corrections. Confirmed the `.run` marker is what makes the harness build and RUN
      the binary: without it the harness stopped after the `-ast -ir` build and
      `build.log` never recorded a run at all.
- [x] Confirm it fails for the documented reason: A and B abort with `7-702-0004`,
      C passes, and the build log carries the wrong `DEAD_HANDLER` on B only. Observed
      exactly that on the committed repro (exit 255, one warning on B). Every later
      shape was RED-verified against the pre-fix binary before its fix landed.
- [x] Settle H1/H2/H3 for shape B. H1 CONFIRMED, H2 and H3 eliminated — see Root
      Cause. Settled from `--ncode` rather than `--nir`: the `.ncode` shows the
      emitted branch target directly, which is the whole question.
- [x] Complete the blast-radius audit. Every site has a written verdict under Root
      Cause, plus two the document did not list: `builder_values.rs`'s `CallResult`
      arm (a fourth census consumer — this is shape B's mechanism) and
      `builtins/http/helper_bytes_to_text.rs` (documented the bug as intended
      behavior; corrected).

Acceptance: the new fixture fails on A and B and passes on C; every audit site has a
recorded verdict; shape B's mechanism is proven, not hypothesised. **Met.**
Commit: `65a634b8b` (fixture + fix), `6c564ce8d` (audit verdicts)

### Phase 2 — the fix

- [x] Make `inline_builtin_is_infallible` (or a new type-aware sibling) answer for
      `toString` by argument type: fallible for `List OF Byte`, infallible otherwise.
      Resolved the Open Decision the way it recommended — ONE census. Both
      `inline_builtin_is_infallible` and `inline_builtin_raw_supported` take
      `arg_types` and consult a single `arg_type_makes_inline_builtin_fallible`.
- [x] Thread argument types to `src/ir/fallible.rs:call_is_fallible` and its
      consumers — the three listed, plus `ir/shape.rs` and the unlisted
      `builder_values.rs`. Every other name's verdict is bit-identical: each consumer
      checks `inline_builtin_fallibility_depends_on_args` first and passes an empty
      slice otherwise, so a name-decided callee reaches the identical code path.
      `ir/fallible.rs:analyze` needed a real type oracle, so `lower_facts` now builds
      the context first and runs the fixpoint through it.
- [x] Apply the shape-B fix indicated by Phase 1's proven mechanism. As predicted by
      H1 it was contained: route the byte-list overload to `lower_inline_builtin_raw`
      (one new arm calling the existing `lower_to_string`) so the established
      `raw_result_capture` seam catches the `ErrEncoding` return. No new mechanism.
- [x] Update `mfb spec language error-model` §8.6 rule 11 to qualify `toString`, and
      rule 14's conversion-built-in list to include it. Both done. `mfb man errors`
      was checked and does not repeat the claim.

Acceptance: the Phase 1 fixture passes end to end; every contrast case still behaves
as documented; `tests/rt-behavior/trap/inline-trap-infallible-builtin-valid` is green
and unmodified; nothing in Non-goals changed. **Met.**
Commit: `65a634b8b`, `2d3210c0e` (gate the typing), `e5b78f535` (shapes I/J)

### Phase 3 — regenerate expected outputs + full validation

- [x] Regenerate only the goldens the fix legitimately shifts. **The only goldens
      that moved in the whole tree are this bug's own new fixture's four.** Measured:
      a full `scripts/test-accept.sh` over the pre-final binary reported
      `1352 test(s) ran` with 3 mismatches, and all three were
      `rt-behavior/trap/inline-trap-tostring-bytes-rt` (its source had grown a shape
      since those goldens were cut). No `.ncode` anywhere churned, which is the
      signal this box asks for: the change did not leak into the general `toString`
      path.
- [x] `cargo test --no-fail-fast`: **every `test result:` line `ok`, 0 failed**
      (`3757 passed` in the `--bin mfb` suite — the build that actually runs the
      `debug_assert!`s the hoist walkers carry — plus 96 `rt_*`/integration binaries).
      Acceptance harness: **`acceptance tests passed (1352 test(s) ran)`, 0
      mismatches.** Trap noted for the record: `cargo test … | tail` reports *tail's*
      exit status, so the first run's "exit 0" proved nothing; re-run capturing
      cargo's own status.
- [x] Re-run the repro on macos-aarch64 and on the Linux axis. The committed repro
      prints `A caught` / `B caught` / `C caught: 77020004` at exit 0 on **macOS
      aarch64, Linux aarch64 (2223) and Linux x86_64 (2228)**. The full eleven-shape
      fixture was cross-built and run on both Linux boxes too, all green — and shape
      K was RED on Linux *before* its fix, which is what shows that gap is in shared
      HIR analysis rather than a backend.
- [x] Rebuild `examples/browser` (all four projects, in dependency order) and
      confirm a non-UTF-8 page still surfaces as an error page. Proven end to end, not
      by inspection: a loopback `tcp` server answering `caf\xE9` as `text/html`,
      fetched through `thread::start(fetch::fetch, …)` exactly as `app/src/main.mfb`
      does, yields `ok=FALSE` / "Error loading … Text encoding or decoding failed."
      at exit 0 — an error page, with the process alive.

Acceptance: full suite green; golden deltas are exactly the intended change; the
reproduction passes everywhere it previously failed. **Met.**
Commit: `77edf0dfe` (goldens), `6def750a1` (doc), plus the post-merge re-run.

## Corrections

Seven shapes beyond the document's A/B/C. Every one was reproduced against the
pre-fix binary before its fix landed — none is a hypothetical.

- **D — `"D " & toString(bytes)`**, the decode nested inside a larger trapped
  expression. Not "aborts": *rejected at build time*, `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`
  — "this expression is not a call". While `toString` was named infallible nothing
  hoisted, so the desugar saw a scrutinee with no fallible call in it and refused a
  valid program. The same wrong fact, surfacing as a false rejection rather than a
  dropped handler.
- **E — a user `FUNC` whose only raise is a byte decode**, called with a nested
  fallible call. `fallible::analyze` judged it infallible, so `check_root` left it a
  plain `Call`: shape A one frame up. Fixing it is why `lower_facts` was restructured
  — `analyze` had no type oracle at all, and the document's Fix Design did not
  anticipate that consumer needing one.
- **I — a top-level binding** (`IrValue::Global`). A `Global` node is a bare name
  with no `type_`, so it typed `Unknown` and fell back to the name-keyed verdict.
  Needed `context.binding_types` threaded through the three hoist walkers.
- **J — a record field** (`IrValue::MemberAccess`) — which is the idiom this
  document is *written against*, `toString(resp.body)`. It was missing because the
  first draft of `ir_call_arg_types` hand-listed the typed `IrValue` variants, and a
  hand copy of that list is a second list to keep in step. Replaced with the
  canonical `IrValue::annotated_parameter_type()`, whose `None` cases are exactly the
  two binding-environment kinds (`Local`, `Global`) the maps cover.
- **F, G, H** are guards rather than defects: `toString(486)` still warns
  `TYPE_INLINE_TRAP_DEAD_HANDLER` with a dead handler (the Non-goal); valid UTF-8
  under an inline `TRAP` still yields the real string; and the decode one frame below
  a function-level `TRAP` — the literal `fetch::pageResult` / `fetch::fetch` pair —
  still delivers `77020004` to the caller's handler.

Two mechanical notes worth keeping:

- **The `.run` golden is what makes the harness run the binary.** Without it,
  `test-accept.sh` stopped after the `-ast -ir` build and `build.log` recorded no run
  at all — so a fixture missing it looks green while never executing.
- **`--ncode`, not `--nir`, settled H1.** The question was which branch the error
  return takes, and the `.ncode` shows the emitted target directly.

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

- **Where the type-aware verdict lives** — **RESOLVED as recommended: one census.**
  `inline_builtin_is_infallible` and `inline_builtin_raw_supported` both take
  `arg_types` and both defer to a single `arg_type_makes_inline_builtin_fallible`.
  The recogniser/measurer split the decision warned about was avoided a second time
  too: the cheap gate consumers use to decide *whether to bother typing arguments*
  (`inline_builtin_fallibility_depends_on_args`) shares its `matches!` with the rule
  it gates, and a unit test pins the direction that matters — a name the gate skips
  can never be flipped by an argument type.
- **Whether to also add an explicit decoder** (`encoding::utf8Decode`-style, strict and
  lossy) — **still a follow-up, still not folded in.** Nothing learned here changes
  that: the charset gap is real (a Latin-1 page has no lossless path today) but an
  explicit decoder would not have closed this bug, since every existing
  `toString(bytes)` call site would have kept the uncatchable abort.

## Summary

The engineering risk is in the interface change, not the predicate: one hard-coded
name-keyed claim (`builtins/mod.rs:273`) is consulted by three IR consumers that
currently have no argument types to hand, and `Fallibility`'s own documentation
explains why. Making the verdict type-aware without perturbing any other name is the
work. A second, smaller unknown — why a correctly-emitted `callResult toString` still
aborts (shape B) — must be proven in Phase 1 before it is fixed. Left untouched: the
success path, `len`/`typeName`, the function-level `TRAP` route `examples/browser`
depends on, and the existing fixture proving `toString(42)` is genuinely infallible.


## STATUS: FIXED

Landed on `worktree-B-486`, merged to `main`.

**What was wrong, in one line:** one name-keyed fact — `toString` is infallible —
consulted by five places, when exactly one of `toString`'s overloads
(`List OF Byte`, a UTF-8 decode) can raise.

**What changed:** the census answers per overload
(`arg_type_makes_inline_builtin_fallible`), and every consumer now passes argument
types: `ir/lower.rs` (`check_root`, `trap_hoist_kind`), `ir/verify/resources.rs`,
`ir/shape.rs`, `ir/fallible.rs:analyze`, and `builder_values.rs`'s `CallResult` arm.
The byte-list overload became raw-supported, reusing the existing
`raw_result_capture` seam rather than inventing a mechanism.

**Deviations from the plan as written, all upward in scope:**

- The document named **three** census consumers; there are **five**. The unlisted
  `builder_values.rs` arm *is* shape B's mechanism (H1), and `ir/shape.rs` was listed
  only as "audit" but genuinely needed the change.
- The document scoped the fixture to shapes **A/B/C**; it ships **A–K**. Shapes D, E,
  I, J and K are each a separate reproduced failure found while fixing — including J,
  `toString(resp.body)`, which is the very idiom the document's opening paragraph is
  written against. See Corrections.
- `ir/fallible.rs:analyze` needed a **type oracle it did not have**, so `lower_facts`
  now builds the lowering context first and runs the fallibility fixpoint through it
  (it also runs after binding inference now, which is strictly better information).
  The Fix Design did not anticipate this; it is the largest structural change here.

**Gates, all measured on the merged tree:** `cargo test --no-fail-fast` every
`test result: ok`, 0 failed; `scripts/test-accept.sh` `1352 test(s) ran`, 0
mismatches; the repro and the eleven-shape fixture green on macOS aarch64, Linux
aarch64 and Linux x86_64; `examples/browser` proven end-to-end against a loopback
server serving non-UTF-8 HTML.

**Non-goals held.** `toString` on every other argument type is still infallible and
still warns `TYPE_INLINE_TRAP_DEAD_HANDLER`;
`tests/rt-behavior/trap/inline-trap-infallible-builtin-valid` is green **and
unmodified**; the success path, `len`/`typeName`, the function-level `TRAP` route,
and error code `77020004` are all unchanged; `bug-479` is untouched.

**One thing deliberately left undone.** `src/audit/collect/source.rs` under-reports a
byte decode in `mfb audit`'s Control-flow section. That census is an AST-level
reporting oracle with no types and is separate from the lowering oracle *by design*
(a report that over-reports is noisy; a desugar that under-reports miscompiles), so
making it type-aware is its own concern, not this bug's.
