# bug-376: an inferred resource binding is not rejected — `LET f = fs::open(...)` compiles clean and silently drops the close obligation

Last updated: 2026-07-21
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness (resource leak / footgun)

Status: Fixed (2026-07-21)
Regression Test: `tests/syntax/resources/resource-let-binding-inferred-invalid`,
`tests/syntax/resources/resource-let-binding-wrapper-invalid`

`TYPE_RESOURCE_REQUIRES_RES` (2-203-0082) — the rule that a resource-typed
binding must be declared `RES`, not `LET`/`MUT` — only fires when the binding
carries an explicit `AS T` annotation. Drop the annotation and the rule goes
silent: the binding holds a real resource, but it is not in the function's
resource-owner table, so it gets no `RES` semantics, no scope-drop close, and
no use-after-close tracking. **The close obligation vanishes with the type
annotation.** Nothing is reported at any layer.

This is dangerous because it is silent and because the *shorter*, more natural
form is the broken one — a user writing idiomatic MFBASIC hits it by default.
The leak is invisible at the source: the program compiles, runs, and looks
correct.

**The single correct behavior a fix produces:** a binding whose type is a
resource is rejected with 2-203-0082 regardless of whether that type was
written explicitly or inferred from the initializer — while every binding form
that legitimately holds a resource without `RES` syntax (union match arms,
lambda captures) continues to compile.

References:

- `./mfb spec resources` — the `RES` ownership axis
- Memory: `res-is-a-pointer-not-a-borrow` — a `RES` is a POINTER to the ONE
  resource; every holder binds with `RES`, owner or not. This is why there is
  no legal `LET`-of-a-resource, and why the fix direction is "widen the rule",
  not "narrow it".
- Retired by plan-20-Z: `src/syntaxcheck/checking.rs:check_resource_declaration`
  was reduced to resolving the STATE type reference, moving the rejection into
  `ir::verify`. That move is what left a single gated rejecter.
- Found 2026-07-21 while fixing `examples/audio` against the `libsnd` binding,
  where it surfaced only indirectly (see Failing Reproduction).
- Reverted first attempt: `6c4ee58ed` (bad), reverted by `c5fdafbf2`.

## Failing Reproduction

```
mfb init /tmp/resrepro
cat > /tmp/resrepro/src/main.mfb <<'EOF'
IMPORT fs

FUNC main AS Integer
  LET f = fs::open("x", "r")
  fs::close(f)
  RETURN 0
END FUNC
EOF
cd /tmp/resrepro && mfb build
```

- Observed: `Building resrepro (executable) for macos-aarch64` /
  `Wrote executable to ./build/resrepro.out` — **exit 0, no diagnostic.**
- Expected:
  ```
  error[2-203-0082 TYPE_RESOURCE_REQUIRES_RES]: resource must be bound with RES
                 binding `f` holds resource `File`; bind it with `RES`, not `LET`/`MUT`.
  ```

Contrast case that works correctly today — the identical program with an
annotation is rejected, which is the whole of the inconsistency:

```
LET f AS File = fs::open("x", "r")   -> error 2-203-0082   (exit 1)
LET f         = fs::open("x", "r")   -> compiles clean     (exit 0)
```

The annotated form is pinned by `tests/syntax/resources/resource-let-binding-invalid`.
No fixture covers the inferred form — that is the hole.

**How it surfaces in real code (the original report).** With an imported
binding package the failure is displaced to a *different line* with an
unrelated message, which is what makes it hard to diagnose:

```
RES music AS SoundFile STATE FileInfo = libsnd::openSound(path)  ' correct
LET music = libsnd::openSound(path)                              ' accepted (bug)
LET info  = music.state
    ^ error[2-203-0043 TYPE_UNKNOWN_VALUE]: Initializer for binding `info`
      does not have a known type.
```

The `LET` is accepted; the STATE association is lost; the error lands one line
later on the `.state` read and names the wrong binding.

Not platform-dependent — the rule is in the shared IR verify pass.

## Root Cause

`src/ir/verify/mod.rs`, in the `IrOp::Bind` arm (~line 902):

```rust
if *explicit_type && !name.starts_with('$') {
    let base = resource_base_type(type_);
    let is_resource = self.is_resource_or_resource_union(base);
    let is_res_declared = self.current_owners.borrow().contains(name.as_str());
    if is_resource && !is_res_declared {
        self.emit("TYPE_RESOURCE_REQUIRES_RES", ...)
```

The `*explicit_type` gate is correct for the *type-agreement* check directly
above it (`check_binding_type`, ~line 885), whose comment states the reasoning:
"Only an explicit `AS T` annotation can disagree with the initializer; an
inferred type is the initializer's type by construction." That reasoning was
carried onto this block, where it does not hold — this block does not compare
two types. It asks two independent questions: is `type_` a resource, and is
`name` in the owner table? `type_` is populated for inferred bindings by
construction, so the check would work ungated.

`ir::verify` is the **sole** rejecter: `src/syntaxcheck/checking.rs:74
check_resource_declaration` was reduced by plan-20-Z to resolving the STATE
type reference and nothing else, so no earlier layer catches it.

### Why the naive fix fails

Removing the gate outright (commit `6c4ee58ed`) produced **18 acceptance
mismatches across 8 fixtures** and was reverted (`c5fdafbf2`). The gate was
incidentally suppressing three other binding forms that also carry
`explicit_type: false`:

**Class 1 — union match-arm pattern bindings.** `src/ir/lower.rs:~1955` emits
the `CASE File(f)` binding as `IrOp::Bind { explicit_type: false, .. }`. There
is nowhere to write `RES` in `CASE File(f)`, so the name is legitimately absent
from the owner table. Ungated, the rule rejects valid code:

```
tests/rt-behavior/resources/resource-union-valid/src/main.mfb:13
  CASE File(f)   -> error 2-203-0082: binding `f` holds resource `File`
```

Affected: `resource-union-valid`, `resource-union-drop-valid`,
`resource-union-foreach-valid`, `bug141_resource_union_return`.

**Class 2 — lambda captures.** The rule fires on a binding that *is* already
`RES`, because the synthesized capture binding is not in `current_owners`:

```
tests/syntax/functions/lambda-capture-invalid/src/main.mfb:7-8
  RES file = fs::openFile(...)                       ' already RES
  LET badResource AS FUNC() AS String = LAMBDA() -> fs::readLine(file)
     -> error 2-203-0082: binding `file` holds resource `File`; bind it with `RES`
```

Affected: `lambda-capture-invalid`, `lambda-mut-capture-invalid`.

**Class 3 — a genuine true positive that shifts a security golden.**
`tests/syntax/security/bug96_audit_tls_http_crypto/src/main.mfb:11` contains
`LET s = tls::connect("example.com", 443)` in a fixture that expects exit 0.
This is a real instance of the bug, so the ungated rule is *right* to reject
it — but fixing it flips a security-audit golden, which is a deliberate call,
not a drive-by.

## Outcome (2026-07-21)

Fixed in `b07f2c4bf` (Phase 1 fixtures in `46b169d74`). Approach **A** — the
RES-axis check is ungated and the synthesized binds are excluded by
initializer shape: `UnionExtract`, `ResultValue`, `ResultError`, `Capture`,
plus the pre-existing `$`-prefix guard. The exclusion list came from an
exhaustive audit of every `IrOp::Bind` emitter in `src/ir/lower.rs`, done
*before* running acceptance — which is what the reverted attempt skipped.

It held: the targeted run of the 8 fixtures the reverted attempt broke
produced **zero** mismatches (15 tests ran; the only 2 diffs were the new
Phase 1 fixtures, whose goldens were deliberately seeded to the buggy
behavior). Full validation: `cargo test` 3188 passed / 0 failed / 1 ignored;
full acceptance **1073 tests, zero mismatches**.

Scope note: only the RES axis widened. The STATE checks in the same block
(`TYPE_UNION_STATE_FORBIDDEN`, `TYPE_STATE_INVALID`,
`check_binding_state_agreement`) keep their `explicit_type` gate — a STATE
clause can only be *written*, so an inferred binding gives them nothing new
to check.

**Split out: bug-377.** The imported-package case is a separate, pre-existing
defect — an imported package's resource types are absent from the consuming
project's IR, so *every* resource rule in `ir::verify` skips them. That is
strictly worse than this bug: a **double close of an imported resource
compiles clean**. See `bugs/bug-377-imported-package-resources-invisible-to-ir-verify.md`.

## Resolved: the `closed-default-tls-drop-rt` segfault — NOT this bug

Under the reverted commit, `tests/rt-behavior/resources/closed-default-tls-drop-rt`
built cleanly, printed both expected lines (`tls-failed=TRUE`, `clean`), then
exited **139 (SIGSEGV)** at scope-drop, against a golden of exit 0.

**SETTLED 2026-07-21: not attributable to this change, and not reproducible.**
On unmodified `main` the fixture built and ran clean 25/25 times (exit 0, both
expected lines). With the fix it passes in the full acceptance run. The
mechanistic argument below was right — `ir::verify` is diagnostics-only. The
most likely explanation is the one the write-up itself flags: the tree did not
compile at the time (unrelated in-flight `DocHeaderKind::Resource` work), so
the artifact under test was not built from the reverted commit's source.
No separate bug filed. Original text follows.

`ir::verify` is
diagnostics-only and cannot alter codegen, so mechanistically this change
should not be able to cause it. The competing hypothesis is an independent
recurrence of the plan-38 F7 macOS bug that this fixture exists to guard (the
`TlsSocket` closed-default drop calling `nw_connection_cancel((void*)0x1)`).
It could not be re-run at write time: the built artifact is cleaned up after
the acceptance run, and the tree did not compile (unrelated in-flight
`DocHeaderKind::Resource` work). **If this reproduces without the ir/verify
change, it is a separate and more serious bug than this one** — a live
macOS memory-safety regression — and must be split into its own bug document.

## Goal

- `LET f = fs::open("x", "r")` (no annotation) is rejected with 2-203-0082,
  naming `f` and `File`, at the line of the binding.
- The annotated form keeps reporting exactly as it does today
  (`resource-let-binding-invalid` golden unchanged).
- All four union-match fixtures, both lambda-capture fixtures, and
  `closed-default-tls-drop-rt` build and behave exactly as their current
  goldens specify.
- ~~`LET music = libsnd::openSound(path)` reports at the `LET`, not as a
  downstream `TYPE_UNKNOWN_VALUE` on `.state`.~~ **Withdrawn — mis-attributed.
  Split to bug-377.** This goal is not achievable by ungating `explicit_type`:
  an imported package's resource type never reaches the consuming project's IR
  as a resource, so `is_resource_or_resource_union` returns false and the
  rule's precondition is never met. Proof it is a different defect: the
  *annotated* `LET music AS SoundFile = libsnd::openSound(...)` fails
  identically, and that path carries `explicit_type: true`, so it took the
  same code path before and after this fix. The same-project wrapper shape
  (`LET h = myOpen()`) IS fixed here and is pinned by
  `resource-let-binding-wrapper-invalid`.

### Non-goals (must NOT change)

- **The `RES`-is-a-pointer model.** Do not reintroduce ownership/borrow
  distinctions. Every holder binds with `RES`; `resource-non-owner-return-valid`,
  `resource-invalidate-not-owner-valid` and `resource-collection-not-owner-valid`
  must keep compiling unchanged.
- **Match-arm and lambda-capture syntax.** The fix must not require `RES` in
  `CASE File(f)` or in a capture list. Those are exemptions, not new
  obligations.
- **The type-agreement check at ~line 885 keeps its `explicit_type` gate** —
  that gate is correct there. Only this block's gate is wrong.
- **Codegen, IR binary format, and `.ncode` output.** If a fix requires a new
  `IrOp::Bind` field, the serialization change in `src/ir/binary.rs` must be
  version-guarded and must not shift any `.ncode` golden.
- **Explicitly forbidden wrong fix:** editing
  `resource-let-binding-invalid`, the union fixtures, or the lambda fixtures so
  they stop exercising the path. Also forbidden: "fixing"
  `bug96_audit_tls_http_crypto` by changing its `LET s` to `RES s` *before*
  deciding whether the audit golden should shift — the fixture is evidence of
  the bug's reach, and silently rewriting it destroys that evidence.

## Blast Radius

Found by search, not memory. Census command:

```
grep -rnE "^\s*(LET|MUT)\s+\w+(\s+AS\s+\S+)?\s*=\s*(tls|net|fs|audio|os|proc)::(connect|listen|accept|open|openFile|createTempFile|openOutput|openInput|listenTcp|connectTcp|spawn)" --include="*.mfb" tests/ examples/ bindings/
grep -rnE "<same>" --include="*.md" src/docs/
```

- `src/ir/verify/mod.rs:~902` (the gate) — **fixed by this bug**.
- `tests/syntax/security/bug96_audit_tls_http_crypto/src/main.mfb:11`
  (`LET s = tls::connect(...)`) — **live instance, in scope**; decide whether
  the fixture source or the audit golden moves (see Open Decisions).
- `tests/syntax/tls/{connect,listen,accept}_invalid`,
  `tests/syntax/audio/open_invalid` — **unaffected**. They use `LET` on
  resource producers, but every call is a deliberate arity/type error that
  fails overload resolution before the resource rule is reached. Confirmed: the
  reverted commit did not shift their goldens.
- `src/docs/man/**` — **unaffected**. Census returns **zero** hits; the man
  pages consistently use `RES` (`tls/connect.md:110,123` are `RES conn = ...`).
  A report that `mfb man tls connect` shows `LET` could not be reproduced
  against the current tree — re-check against a freshly built binary before
  assuming the docs are wrong.
- `examples/audio/src/main.mfb` — **already corrected** in the working tree
  (`RES music AS SoundFile STATE FileInfo = ...`); it is the original report,
  not a remaining site.
- `bindings/**` — no hits.

Note the census pattern only catches `pkg::producer()` initializers. It cannot
see a resource obtained via a user-defined wrapper (`LET h = myOpen()`), which
is exactly the libsnd shape that started this. Re-run a type-aware audit in
Phase 1 rather than trusting this grep — see memory `rename-census-by-grep-underreports`.

## Fix Design

The rule needs to distinguish a **user `LET`/`MUT` statement** from a
**synthesized binding** (match arm, lambda capture). `explicit_type` was
serving as a bad proxy for that distinction; the fix is to make the
distinction real.

Two candidate approaches, in preference order:

**A. Discriminate on initializer shape (no IR format change).** The match-arm
binding is emitted with an `IrValue::UnionExtract` initializer
(`src/ir/lower.rs:match_case_binding`), a shape no user `LET` can produce.
Exempt `UnionExtract` (and the `ResultValue`/`ResultError` siblings) and the
capture-synthesized shape, then ungate. Cheap and contained, but relies on
initializer shape as a proxy — it must be verified that *every* synthesized
resource binding has a recognizable shape, or the same class of false positive
recurs. The lambda-capture case needs its own investigation: the fix there may
instead be to add captures to `current_owners`, which is arguably more correct
than exempting them.

**B. Add an explicit discriminator to `IrOp::Bind`** (e.g.
`origin: BindOrigin::{Let, MatchArm, Capture}`). Unambiguous and
self-documenting, and makes the rule read exactly as intended. Costs a field in
`src/ir/op.rs` plus a version-guarded change in `src/ir/binary.rs`, and must be
proven not to shift any `.ncode` golden.

Rejected: gating on `loc` equality with the case arm's line (fragile — a user
`LET` on the same line as the `CASE` would alias), and keeping `explicit_type`
while adding a second rule for inferred bindings (duplicates the diagnostic and
leaves two rules to keep in sync).

The correctness risk concentrates in the **exemption list being complete**, not
in the ungating itself. The reverted attempt failed precisely because the
exemption set was assumed empty without an audit.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tests/syntax/resources/resource-let-binding-inferred-invalid`, the
      un-annotated sibling of `resource-let-binding-invalid`, with a seeded
      golden. Confirm it fails today (compiles clean, exit 0). — confirmed
      exit 0.
- [x] Add a fixture covering the imported-binding shape. **Landed as a
      SAME-PROJECT wrapper** (`resource-let-binding-wrapper-invalid`), not an
      imported package. The wrapper shape reproduced the displaced
      `TYPE_UNKNOWN_VALUE` symptom and IS fixed here. The genuinely *imported*
      shape turned out to be a different defect that this fix cannot reach —
      split to bug-377. An imported-package fixture needs hermetic package
      install machinery and belongs to that bug.
- [x] Enumerate every lowering site that emits `IrOp::Bind` with
      `explicit_type: false`. Done before touching the gate; 14 sites with a
      verdict each. Exactly two need excluding (`UnionExtract` match arms,
      `Capture` lambda prologue, plus the `ResultValue`/`ResultError`
      siblings); the rest are either `$`-guarded or structurally non-resource
      (FOR loop var = numeric, `TRAP(e)` = `Error`). The list proved complete:
      zero unexpected acceptance mismatches.
- [x] Settle the `closed-default-tls-drop-rt` segfault attribution. **Not
      reproducible on unmodified `main` (25/25 clean), passes with the fix.**
      No separate bug filed — see the section above.
- [x] Re-run the blast-radius census type-aware rather than by grep. Found the
      wrapper-producer shape (fixture added) and the imported-package shape
      (bug-377) — both invisible to the grep pattern, as predicted.

Acceptance: met.
Commit: `46b169d74`

### Phase 2 — the fix

- [x] Implemented **approach A** (initializer shape, no IR format change) per
      the audit; RES-axis check ungated at `src/ir/verify/mod.rs`.
- [x] Lambda captures: **exempted**, with the reasoning recorded under Open
      Decisions.
- [x] Type-agreement check's `explicit_type` gate untouched. The STATE checks
      in the same block also keep theirs — only the RES axis widened.

Acceptance: met. Targeted run = 15 tests, zero unexpected mismatches;
`resource-let-binding-invalid`, all four union fixtures, both lambda fixtures
and the three `not-owner-valid` fixtures byte-identical.
Commit: `b07f2c4bf`

### Phase 3 — the `bug96` decision + regeneration + full validation

- [x] Applied the Open Decision to `bug96_audit_tls_http_crypto` (`LET s` →
      `RES s`); `.audit` golden unchanged, only the AST gained
      `"resource": true`.
- [x] Regenerated only the three goldens the fix legitimately shifts (the two
      new fixtures + bug96's AST). Confirmed the delta is exactly the intended
      change.
- [x] `cargo test` 3188 passed / 0 failed / 1 ignored. Full acceptance
      **1073 tests, zero mismatches, exit 0**.
- [x] Re-ran the `examples/audio` reproduction. It builds clean with the
      already-corrected `RES music AS SoundFile STATE FileInfo`. Reverting
      that line to `LET` is **still not caught** — that is bug-377, not this
      bug (see the withdrawn goal bullet above).
- [x] Runtime proof: `LET f = fs::open(...)` now exits 1 with 2-203-0082,
      while an inferred `RES f = fs::createTempFile()` builds and runs (`ok`,
      exit 0) — the fix rejects the broken form without breaking the correct
      one.

Acceptance: met.
Commit: `b07f2c4bf`

## Validation Plan

- Regression tests: `tests/syntax/resources/resource-let-binding-inferred-invalid`
  plus the imported-binding fixture, both from Phase 1.
- Runtime proof: the `/tmp/resrepro` reproduction above now exits 1 with
  2-203-0082; and a `RES`-corrected variant still builds and runs, proving the
  fix rejects the broken form without breaking the correct one.
- Doc sync: none expected — the census found zero `LET`-bound resources in
  `src/docs/`. Confirm after the fix by rebuilding and re-running
  `scripts/check-man-examples.py`, which compiles the man-page examples and
  would catch any that the widened rule newly rejects.
- Full suite: `cargo test` + `./scripts/test-accept.sh`.

## Open Decisions — both RESOLVED 2026-07-21

- **`bug96_audit_tls_http_crypto` — took the recommendation.** `LET s` →
  `RES s`. The evidence supports it: the *only* golden delta is the AST
  gaining `"resource": true`. The `.audit` golden — what the fixture actually
  asserts (network capability, randomness capability, `TlsSocket` resource,
  fallible call sites) — is **byte-identical**, as are the `.ir` and
  `build.log`. So the `LET` really was incidental to the fixture's purpose,
  and it was itself a live instance of this bug. Bonus: it now also covers an
  *inferred* `RES` binding (`RES s = tls::connect(...)`, no annotation).

- **Lambda captures — exempted, not owner-listed** (against the doc's
  recommendation, on evidence). Owner-listing every capture would make
  ordinary *data* captures satisfy `is_res_declared && !is_resource` and trip
  `TYPE_RES_REQUIRES_RESOURCE` — trading one false positive for a broader
  one. Owner-listing only *resource* captures is equivalent to exempting for
  this rule's purposes. The feared hole ("a capture that genuinely needs the
  diagnostic would never get it") does not exist: a resource capture is
  already rejected, more precisely, by `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`
  ("Lambda captures resource local `file`; resource captures are invalid"),
  pinned by `tests/syntax/functions/lambda-capture-invalid`.

Original text:

- **`bug96_audit_tls_http_crypto`** — recommended: change the fixture source
  `LET s` → `RES s` and leave the audit golden's *findings* unchanged, since
  the fixture's purpose is auditing TLS/HTTP/crypto usage, not resource
  binding, and the `LET` is incidental to what it asserts. Alternative: accept
  the new 2-203-0082 diagnostic into the golden, which pins that the rule
  reaches security fixtures but couples an unrelated golden to this rule.
  Decide before Phase 3; do not change the fixture during Phase 2.
- **Lambda captures** — recommended: add captured resource bindings to
  `current_owners` (they *are* holders under the pointer model, so the rule
  should see them as `RES`-declared) rather than exempting them from the check.
  Exempting is smaller but leaves a real hole: a capture that genuinely needs
  the diagnostic would never get it. Needs the Phase 1 audit to confirm
  feasibility.

## Summary

The one-line gate removal is *not* the fix, and the reverted attempt proves it:
the engineering is in the **audit** — enumerating every synthesized
`IrOp::Bind` that carries `explicit_type: false` for a resource type, and
deciding per site whether it is an exemption or a missing owner-table entry.
The ungating itself is trivial once that list exists. Left untouched: the
`RES`-is-a-pointer model, match-arm and capture syntax, codegen and the IR
binary format, and the type-agreement check's own (correct) `explicit_type`
gate. One unattributed segfault must be resolved in Phase 1 before it is
assumed to belong to this bug.
