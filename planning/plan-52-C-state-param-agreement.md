# plan-52-C: STATE agreement at parameters — close the type confusion

Status: **COMPLETE.** `TYPE_STATE_MISMATCH` (`2-203-0129`) rejects both a re-typed and an
attaching parameter; the bare-param opt-out survives (every close op still compiles, and
the five pre-existing STATE fixtures pass unchanged). The type confusion this closes was
observed at runtime first (plan-52-A Phase 3) and is now a compile error. Two of §2's
claims were corrected against a green tree: the disclosure variant is **not** a disclosure
primitive (it leaks a block-relative offset, not an address), and the `_mfb_str_empty`
error was a live bug, not a stale-tree artifact (`bugs/bug-256`). `thread::transfer`
remains open and is **not** closed by this sub-plan — `bugs/bug-257`.

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: plan-52-A (the model + the pending fixtures this flips green)

Closes a live memory-safety hole. Nothing checks that a parameter's declared `STATE` type
agrees with the state the argument actually carries, so a borrow can **re-type** a payload
it should only observe — or **attach** one to a resource that never had it. Both compile
clean today, from safe source, with no `LINK`, no threads, and no unsafe construct.

The single outcome: **a resource's STATE type is fixed at its owning binding, and any
parameter declaring a different one — or demanding one the argument doesn't have — is a
compile error.**

References:

- `plan-52-A` §3 — the model table this implements (the param row). **Read first.**
- `planning/res.md` §2 — the verified behavior table this rests on.
- `./mfb spec language resource-management` §15 — as amended by plan-52-A.

## 1. Goal

- `test2(RES p AS File STATE Cursor)` given an argument carrying `Label` → **rejected**.
- `test2(RES p AS File STATE Cursor)` given a **stateless** argument → **rejected**
  (params observe; they do not attach).
- `test(RES p AS File)` given anything → **accepted**, `.state` inaccessible inside. The
  opt-out survives.
- Reading `.state` through a bare param produces a diagnostic that **names STATE**.
- plan-52-A's rows 4 and 5 flip from failing to passing; rows 1, 2, 3, 10 unchanged.

### Non-goals (explicit constraints)

- **The bare-param opt-out.** `RES p AS File` accepting a stateful argument with `.state`
  inaccessible is the model's param rule (plan-52-A §3), not laxity. Every close op depends
  on it — `FUNC close(RES db AS Db)` must accept the resource whatever state it carries.
  A "fix" that tightens bare params to stateless-only breaks every close op.
- **`emit_resource_state_init`'s null-check** (`src/target/shared/code/builder_value_semantics.rs:10-36`).
  Stays. "Allocate once; a carried state survives a move" is required by plan-52-D. The fix
  is a front-end rejection, **not** a codegen change — do not make init type-aware, and do
  not re-initialize per binding (see §3's rejected alternatives).
- **The binding and return rows** of the model table. Those are plan-52-D — the binding
  rule must land *with* the return rule, never before or after it (plan-52-D §3).
- **Runtime checks.** The payload has no type tag; there is nothing to check against, and
  adding one is a layout change (plan-52-B Non-goals). This is statically decidable —
  reject at the call site.
- **Tempting wrong fix:** rewriting plan-52-A's fixtures so the two STATE types agree. The
  programs are *accepted today*; making them consistent removes the demonstration, not the
  hole.

## 2. Current State

`STATE` rides inside the type string (`"File STATE Cursor"`), split back out by
`crate::builtins::resource::state_type_name` (`src/builtins/resource.rs:231-233`). The
payload itself is a pointer at `FILE_OFFSET_STATE` (= 16) and **carries no runtime type
tag** — its type comes entirely from whichever type string the reader holds.

`src/ir/lower.rs:739-742` carries a param's declared STATE into its local type string
("so `s.state` resolves inside the callee, matching `lower_param`"). Codegen's bind path
then runs `emit_resource_state_init`, which allocates **iff the slot is null**:

```rust
self.emit(abi::load_u64(&current, &ptr, FILE_OFFSET_STATE));
self.emit(abi::compare_immediate(&current, "0"));
self.emit(abi::branch_ne(&done));          // already has a state -> keep it
```

That null-check is correct for the *move* case it was written for ("a moved/returned
resource that already carries a state keeps it"). Nothing scoped it to that case. So a
param **attaches** when the slot is null and **re-types** when it is not.

The read side types the load purely from the string:
`src/target/shared/code/builder_value_semantics.rs:175-190` returns
`ValueResult { type_: state_type, … }` where `state_type` came from the *reader's* type
string. `src/ir/lower.rs:2188-2195` does the same for `.state` member typing. Neither
consults what was allocated.

The STATE verify rules (`src/ir/verify/mod.rs:825-845`) check a declared state type **in
isolation** — is it defaultable (`TYPE_STATE_INVALID`), is the base a union
(`TYPE_UNION_STATE_FORBIDDEN`). **No rule relates two declarations to each other.** That
is the gap.

**Verified today** (plan-52-A rows 4/5): a stateful param on a stateless owner allocates
(`a saw pos = 7`); a `Cursor{pos:Integer}=42` read through a `STATE Label` param is
interpreted as `Label{name:String}` and falls over with a bogus `Allocation failed` — the
integer 42 read as a String header.

**RESOLVED — it is NOT a disclosure primitive.** The reverse direction (allocate as
`Label{name:String}`, read back as `Cursor{pos:Integer}`) was suspected of disclosing a raw
arena pointer as an `Integer` rather than faulting. It has now been **built on a green tree
and observed**: it prints `pos as integer = 8`. A record's `String` field is a
**block-relative offset** to an inlined sub-block, never a pointer, so the reverse read
leaks the constant `8` — a structural offset, not an address. **The severity is not
raised**; the `Cursor`→`Label` direction below remains the whole of the finding.

Two things blocked this demonstration and both are now resolved:

- The `native code data relocation target '_mfb_str_empty' is not a data object or defined
  symbol` error was **not** a mid-plan-50 artifact — it reproduced on the green tree and
  was a live bug: any `STATE` carrying a `String` field failed to link. Fixed as
  `bugs/completed-bugs/bug-256`, which is what made this observation possible at all.
- The stale-binary trap below is real and was re-confirmed; every run recorded here was
  checked against `ls -la`/`date` on a freshly removed `build/`.

> **Trap, generally:** a failed `mfb build` leaves the previous `build/<name>.out` in
> place, and running it looks like a pass. `rm -rf build` and check the binary timestamp.
> This produced one false finding during this plan's research before it was caught.

The confirmed `Cursor`→`Label` direction is sufficient to establish the type confusion and
to justify every rule in §3; the disclosure variant would only raise the severity.

**Precedent to mirror:** the two rules at `:825-845` are the right shape and the right
home; they just need a *relational* sibling.

## 3. Design Overview

Add a `TYPE_STATE_MISMATCH` rule beside the existing STATE rules
(`src/ir/verify/mod.rs:825-845`), applied at the argument→parameter boundary. Both sides'
type strings are already in hand; this is a comparison the verifier can make today with no
new plumbing.

The param row, in full:

| argument | param `STATE T` | param **bare** |
|---|---|---|
| carries `T` | ✓ | ✓ (opt-out) |
| carries `T2 ≠ T` | ✗ `TYPE_STATE_MISMATCH` | ✓ (opt-out) |
| **stateless** | ✗ `TYPE_STATE_MISMATCH` | ✓ |

**The correctness risk is over-rejecting — but not where it first appears.** The obvious
reading is that *"stateless → stateful is fine (allocate)"* should stay legal, and that
rejecting it is the danger. That is backwards: param-attach is precisely what makes two
disagreeing borrows reachable **with no stateful binding anywhere**:

```basic
SUB a(RES p AS File STATE Cursor)   ' first call allocates a Cursor
SUB b(RES p AS File STATE Label)    ' second reads that block as a Label
RES h AS File = fs::openFile(...)   ' stateless owner
a(h)
b(h)                                ' -> confusion, no diagnostic
```

And no in-tree fixture depends on it — `resource-state-field-assign-valid`'s **owner**
declares `RES f AS File STATE Cursor`, so its param never allocates. plan-52-A Phase 3
confirms this by audit. The real risk is the opposite one: breaking the **bare-param
opt-out** (row 1/2/3), which every close op needs.

**Rejected: re-initialize on every binding that declares a STATE.** Trivially kills the
confusion by giving the second declaration its own block — and silently breaks the feature:
the owner would stop observing a borrow's update (`resource-state-field-assign-valid`), and
a moved state would be discarded. Contradicts §15's "a state update made through a borrowed
RES parameter is visible to the owner after the call."

**Rejected: a runtime type tag on the payload.** Robust, but it is a layout change, costs a
load and compare on every `.state`, and turns a statically-decidable error into a runtime
one.

**Rejected: documenting it as a footgun.** Neither the resource's producer nor its consumer
can generally see the other's state declaration. Not author-avoidable.

## 4. The `.state` read diagnostic

Secondary, same area, worth doing here. The **write** path is diagnosed precisely today:

```
error[2-203-0085 TYPE_STATE_INVALID]: `p` has no STATE to assign; declare the resource with `STATE T`.
```

The **read** path is not: `p.state.pos` degrades to `Unknown` and the error surfaces
wherever that Unknown lands — observed as `TYPE_CALL_ARGUMENT_MISMATCH` on `toString`
complaining about argument types, never mentioning STATE. It *is* rejected, so this is a
diagnostic gap, not a hole. Make the two paths say the same thing; flip plan-52-A row 3's
`TODO(plan-52-C)` golden.

## Compatibility / Format Impact

- **No layout, no `.mfp`, no API change.** Front-end rules only.
- **Source compatibility:** programs relying on param-attach or on a re-typed STATE stop
  compiling. Both are the bug. No in-tree fixture does either (plan-52-A Phase 3 audit).
- **Goldens:** plan-52-A rows 4/5 flip; row 3's message changes. No other movement expected
  — verify with the artifact gate rather than assuming.

## Phases

### Phase 1 — the mismatch rule

- [x] Add `TYPE_STATE_MISMATCH` to `src/rules/table.rs` (`2-203-0129`) with a
      site-specific message naming **both** types, plus the two spec registries
      (`diagnostics rule-codes`, `package verifier-rules`).
- [x] Implement the §3 table at the argument→param boundary —
      `check_argument_state_agreement`, called from `check_call_argument_types`, which
      already had both full type strings in hand (it strips STATE off each to compare
      bases, so the relational data was right there).
- [x] Keep bare params accepting anything — `state_type_name(param) == None` returns
      early, so the opt-out is the rule's first line.
- [x] **Also required, not in the plan:** add `TYPE_STATE_MISMATCH` to
      `RELOCATED_TO_IR_VERIFY`. `collect_source_diagnostics` filters the source path to
      that allowlist, so without the entry the rule fired but was **dropped on the source
      path**, surfacing only through the package path's `check()` as an unlocated
      `error: TYPE_STATE_MISMATCH: …` with no `file:line` and no rule code. ir::verify is
      the sole implementer (syntaxcheck has no twin), so there is nothing to duplicate.

Acceptance: plan-52-A rows 4 and 5 are rejected with `TYPE_STATE_MISMATCH`; rows 1, 2, 10
still pass unchanged; `resource-state-field-assign-valid` and its four siblings pass
unchanged.
Commit: —

### Phase 2 — the `.state` read diagnostic

- [x] Give the read path a STATE-naming error, matching the write path's wording. Landed in
      `check_member_access` (`src/ir/verify/mod.rs`), which already had the target's
      inferred type in hand — the natural home, beside the `t.result` arm.
- [x] Row 3's golden now carries `TYPE_STATE_INVALID` naming STATE.

Acceptance: met. `p.state.pos` on a bare param now reports

    error[2-203-0085 TYPE_STATE_INVALID]: `File` here has no STATE to read; declare the
    resource with `STATE T`. A bare `RES` parameter cannot read the state its caller
    attached.

`TYPE_CALL_ARGUMENT_MISMATCH` still appears alongside it (the `Unknown` still lands on
`io.print`), so the fixture pins both: the point was that the diagnostic *names STATE*, not
that the consumer error disappears. Removing the latter would mean suppressing a poisoned
value's downstream error, which is a separate concern.
Commit: —

### Phase 3 — validation

- [x] `scripts/artifact-gate.sh`: **967 tests, 1141 goldens, 0 diffs** — the codegen delta
      is nil, as a front-end-rules-only change should be.
- [x] Goldens: rows 3/4/5 shifted and nothing else.
- [x] The two-disagreeing-borrows program now **fails to build** with
      `TYPE_STATE_MISMATCH`. The runtime confusion proof is inverted, as planned.

Acceptance: **full suite green — 981 acceptance tests, 2901 unit tests, 0 failures.** The
five pre-existing STATE fixtures pass unchanged (over-rejection would have shown up there
first).
Commit: —

**One regression found and fixed here, worth recording:** the new `.state`-read rule also
fired on the *write* path, because `s.state = WITH s.state { … }` reads `s.state` in its
`WITH` target — so a stateless state-assign reported the same line twice, and the read
message said "parameter" where `s` was a binding. Suppressed inside a state assignment (the
assign arm's diagnostic is the precise one) and the wording no longer assumes a parameter.
A diagnostic that fires on a sub-expression of a statement another rule already owns is a
regression even when both messages are true.

## Validation Plan

- Tests: plan-52-A's `resource-state-param-mismatch-invalid` and
  `resource-state-param-attach-invalid` (flip to passing); rows 1/2/3/10 as guards; the
  five existing STATE fixtures unchanged.
- Runtime proof: **inverted** — before this sub-plan the proof is that the confusion
  program builds and misreads its payload; after, the proof is that it no longer builds.
  The five existing STATE fixtures still need their runtime pass (`.ai/compiler.md` gate),
  since over-rejection would show up there first.
- Doc sync: `src/docs/spec/language/15_resource-management.md` — the param row is
  plan-52-A's; add `TYPE_STATE_MISMATCH` to the diagnostics tables per
  `.ai/specifications.md`.
- Acceptance: `scripts/test-accept.sh`, `scripts/artifact-gate.sh`, `cargo test --bin mfb`.

## Open Decisions

- **One code or three?** One `TYPE_STATE_MISMATCH` across param/return/binding with a
  site-specific message, vs. a code per site. Recommend **one** — the user's fix is the
  same shape everywhere. plan-52-D reuses it for the return and binding rows.
- **`thread::transfer`** — **RESOLVED: yes, it can, and this sub-plan does not close it.**
  Filed as `bugs/bug-257` (HIGH). Confirmed at runtime: a sender attaching
  `Cursor{pos:Integer}=99` and a worker declaring `STATE Label{name:String}` build clean and
  type-confuse across the thread boundary. It survives both C and D **by construction** —
  `thread::accept`'s static return type is a bare `File`, so the receiver's binding reads as
  a legal *attach* (plan-52-D's one true attach point) while `emit_resource_state_init`'s
  null-check silently **adopts** the sender's payload and re-types it. The STATE arrives in
  a pointer the type system never sees, so no static rule over type strings can catch it.
  Closing it requires the STATE on the plane type (`Thread OF RES File STATE Cursor TO …`)
  — a language-surface change, hence its own plan rather than scope creep here.

## Summary

A relational rule the verifier already has both sides' information to make. The engineering
risk is entirely in **what not to reject**: the bare-param opt-out must survive, or every
close op breaks. The intuitive rule — allow `stateless → stateful` so a param may
attach — is the unsafe one, and the fixture it was protecting turns out not to need it.
Attachment belongs at the owning binding, exactly once; params observe.
